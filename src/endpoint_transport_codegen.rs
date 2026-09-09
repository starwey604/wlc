// SPDX-License-Identifier: Apache-2.0
//! One source of truth for envelope bounds and default endpoint storage.
use crate::{endpoint_layout::EndpointEnvelope, template::render};

pub(crate) fn fragments(envelope: EndpointEnvelope, prefix: &str) -> Vec<(&'static str, String)> {
    let (default_envelope, unit, control, stream) = match envelope {
        EndpointEnvelope::Any | EndpointEnvelope::CobsStream => (
            if envelope == EndpointEnvelope::Any {
                "WL_ENVELOPE_NATIVE_PACKET"
            } else {
                "WL_ENVELOPE_COBS_STREAM"
            },
            "(@P@_ENDPOINT_RAW_CAPACITY + @P@_ENDPOINT_RAW_CAPACITY / 254U + 2U)",
            "(WL_FRAME_HEADER_SIZE + WL_FRAME_MAX_CRC + 2U)",
            true,
        ),
        EndpointEnvelope::NativePacket => (
            "WL_ENVELOPE_NATIVE_PACKET",
            "@P@_ENDPOINT_RAW_CAPACITY",
            "(WL_FRAME_HEADER_SIZE + WL_FRAME_MAX_CRC)",
            false,
        ),
        EndpointEnvelope::BusLength16 => (
            "WL_ENVELOPE_BUS_LENGTH16",
            "(@P@_ENDPOINT_RAW_CAPACITY + 2U)",
            "(WL_FRAME_HEADER_SIZE + WL_FRAME_MAX_CRC + 2U)",
            false,
        ),
    };
    let fifo = if stream {
        "@P@_ENDPOINT_UNIT_CAPACITY"
    } else {
        "0U"
    };
    let capacity = format!(
        "#define @P@_ENDPOINT_UNIT_CAPACITY {unit}\n#define @P@_ENDPOINT_CONTROL_CAPACITY {control}\n#define @P@_ENDPOINT_RX_FIFO_CAPACITY {fifo}"
    );
    let validate = if envelope == EndpointEnvelope::Any {
        String::new()
    } else {
        format!("  if (config->link.envelope != {default_envelope}) return WL_ERR_NOT_SUPPORTED;")
    };
    let state = if stream {
        "    uint8_t rx_fifo[@P@_ENDPOINT_RX_FIFO_CAPACITY];"
    } else {
        ""
    };
    let bind = if stream {
        "  link_storage.rx_fifo = endpoint->private_state.rx_fifo;\n  link_storage.rx_fifo_size = sizeof(endpoint->private_state.rx_fifo);"
    } else {
        ""
    };
    [
        ("TRANSPORT_CAPACITY", capacity),
        ("DEFAULT_ENVELOPE", default_envelope.into()),
        ("LAYOUT_VALIDATE", validate),
        ("TRANSPORT_STATE", state.into()),
        ("TRANSPORT_BIND", bind.into()),
    ]
    .into_iter()
    .map(|(key, value)| (key, render(&value, &[("P", prefix)])))
    .collect()
}
