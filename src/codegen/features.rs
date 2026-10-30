// SPDX-License-Identifier: Apache-2.0
//! Select shared engine branches once for the complete (including imported) schema.
use super::descriptor::field_descriptor_data;
use crate::{ast::Cardinality, semantic::MessageSymbol};
use std::collections::BTreeSet;

pub(super) struct Features(BTreeSet<&'static str>);
impl Features {
    pub(super) fn new(
        messages: &[&MessageSymbol],
        codecs: &crate::codec_plan::CodecPlan<'_>,
    ) -> Self {
        let mut used = BTreeSet::new();
        if messages.iter().any(|message| {
            super::fixed::supports(&super::plan::MessagePlan::new(message))
                && message.fields.iter().any(|field| {
                    crate::codec_plan::field_encoding(field, codecs.messages[&message.id].profile)
                        == crate::codec_plan::ValueEncoding::UnsignedVarint(32)
                })
        }) {
            used.insert("DIRECT_U32");
        }
        let tagged = super::descriptor::tagged_dependencies(codecs, false);
        for (profile, field) in messages.iter().flat_map(|message| {
            std::iter::once(codecs.messages[&message.id].profile)
                .chain(tagged.contains(&message.id).then_some(codecs.profile))
                .flat_map(move |profile| message.fields.iter().map(move |field| (profile, field)))
        }) {
            if field.cardinality == Cardinality::Repeated {
                used.insert("REPEATED");
            }
            let kind = if crate::codec_plan::field_encoding(field, profile)
                == crate::codec_plan::ValueEncoding::UnsignedVarint(32)
            {
                "U32"
            } else {
                field_descriptor_data(field).0.strip_prefix("WLC_").unwrap()
            };
            used.insert(kind);
            match kind {
                "F32" | "F64" | "FLOAT32" | "FLOAT64" => {
                    used.insert("FIXED");
                }
                "BYTES" | "STRING" | "MESSAGE" => {
                    used.insert("LENGTH");
                }
                _ => {
                    used.insert("VARINT");
                }
            }
            if matches!(
                field.cardinality,
                Cardinality::Packed(_) | Cardinality::RequiredPacked(_)
            ) {
                used.insert("PACKED");
            }
        }
        Self(used)
    }
    pub(super) fn owned(
        messages: &[&MessageSymbol],
        codecs: &crate::codec_plan::CodecPlan<'_>,
    ) -> Self {
        let mut features = Self::new(messages, codecs);
        features.0.remove("REPEATED");
        features
    }
    pub(super) fn with_fingerprints(mut self, enabled: bool) -> Self {
        if enabled {
            self.0.insert("FINGERPRINT");
        }
        self
    }
    // Markers belong only to compiler-owned C fragments. Removed branches never
    // reach generated C; the ordinary literal template renderer stays unchanged.
    pub(super) fn select(&self, template: &str) -> String {
        let mut enabled = vec![true];
        let mut out = String::with_capacity(template.len());
        for line in template.split_inclusive('\n') {
            if let Some(kinds) = line.trim().strip_prefix("// WLC_IF ") {
                let keep = kinds.split_whitespace().any(|kind| self.0.contains(kind));
                enabled.push(*enabled.last().unwrap() && keep);
            } else if line.trim() == "// WLC_END" {
                assert!(enabled.len() > 1, "unmatched engine feature marker");
                enabled.pop();
            } else if *enabled.last().unwrap() {
                out.push_str(line);
            }
        }
        assert_eq!(enabled.len(), 1, "unterminated engine feature marker");
        out
    }
}
