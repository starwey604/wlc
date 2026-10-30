// SPDX-License-Identifier: Apache-2.0
//! Symbol policy is an input to emission, never a rewrite of generated C.
use crate::semantic::MessageSymbol;

pub(crate) struct Symbols {
    pub prefix: String,
    pub helper: String,
    pub constants: String,
    pub storage: &'static str,
    pub include: String,
}

impl Symbols {
    pub fn public(module: &str) -> Self {
        Self {
            prefix: String::new(),
            helper: "wlc_".into(),
            constants: "WLC_".into(),
            storage: "",
            include: format!("#include \"{module}.h\""),
        }
    }

    pub fn private(module: &str) -> Self {
        Self {
            prefix: format!("{module}_rpc_"),
            helper: format!("{module}_rpc_wlc_"),
            constants: format!("{}_RPC_WLC_", super::upper_snake(module)),
            storage: "static inline ",
            include: String::new(),
        }
    }

    pub fn message(&self, message: &MessageSymbol) -> String {
        format!("{}{}", self.prefix, super::type_name(&message.name))
    }

    pub fn render(&self, template: &str, values: &[(&str, &str)]) -> String {
        let mut args = vec![
            ("C", self.helper.as_str()),
            ("K", self.constants.as_str()),
            ("CODEC_INCLUDE", self.include.as_str()),
        ];
        args.extend_from_slice(values);
        crate::template::render(template, &args)
    }
}
