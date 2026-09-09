// SPDX-License-Identifier: Apache-2.0
//! Local deployment capabilities. These do not change message wire identities.

#[derive(Clone, Copy, Debug, Default, Eq, PartialEq)]
pub enum EndpointEnvelope {
    #[default]
    Any,
    NativePacket,
    CobsStream,
    BusLength16,
}

#[derive(Clone, Copy, Debug, Default, Eq, PartialEq)]
pub enum EndpointRpcRole {
    Client,
    Server,
    #[default]
    Both,
}

#[derive(Clone, Copy, Debug, Default, Eq, PartialEq)]
pub struct EndpointLayout {
    pub envelope: EndpointEnvelope,
    pub rpc_role: EndpointRpcRole,
}

impl EndpointLayout {
    pub fn has_client(self) -> bool {
        self.rpc_role != EndpointRpcRole::Server
    }

    pub fn has_server(self) -> bool {
        self.rpc_role != EndpointRpcRole::Client
    }
}
