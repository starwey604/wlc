// SPDX-License-Identifier: Apache-2.0
use crate::semantic::MessageSymbol;

#[derive(Clone, Debug, Eq, PartialEq)]
pub(crate) struct Symbols {
    pub(crate) storage: &'static str,
    pub(crate) prefix: &'static str,
    pub(crate) fn_prefix: &'static str,
    pub(crate) constants: &'static str,
}

impl Symbols {
    pub fn public() -> Self {
        Self {
            storage: "",
            prefix: "",
            fn_prefix: "wl_codec_",
            constants: "WL_CODEC_",
        }
    }

    pub fn message_name(&self, message: &MessageSymbol) -> String {
        super::type_name(&message.name)
    }
}
