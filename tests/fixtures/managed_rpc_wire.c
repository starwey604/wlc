/* SPDX-License-Identifier: Apache-2.0 */
#include "wire_kernel_codecs.h"
#include "wirelink/kernel_wire.h"
#include <assert.h>
#include <string.h>
/* Same pre-migration metadata/domain cases, exercised through shared primitives. */
static void headers(void) {
  const uint32_t ids[]={1U,63U,64U,127U,128U,8191U,8192U,UINT32_C(1)<<27,UINT32_MAX};
  const uint64_t sessions[]={1U,UINT64_C(0x0123456789ABCDEF),UINT64_MAX};
  const int32_t statuses[]={0,1,-1,INT32_MIN,INT32_MAX};
  uint8_t bytes[18];
  for(size_t i=0;i<sizeof(ids)/sizeof(ids[0]);++i)
    for(size_t j=0;j<sizeof(sessions)/sizeof(sessions[0]);++j)
      for(size_t k=0;k<sizeof(statuses)/sizeof(statuses[0]);++k) {
        size_t length=0;wl_kernel_rpc_metadata_t value;
        assert(wl_kernel_rpc_write(bytes,sizeof(bytes),sessions[j],ids[i],statuses[k],&length)==WL_OK);
        assert(length==wl_kernel_rpc_header_size(ids[i],statuses[k]));
        assert(length<=(statuses[k]?17U:13U));
        assert(wl_kernel_rpc_read(bytes,length,0,&value)==WL_OK);
        assert(value.session==sessions[j] && value.operation==ids[i] && value.status==statuses[k] && value.length==0);
        assert(wl_kernel_rpc_read(bytes,length,1,&value)==(statuses[k]?WL_ERR_CORRUPT_PAYLOAD:WL_OK));
        for(size_t truncated=0;truncated<length;++truncated)
          assert(wl_kernel_rpc_read(bytes,truncated,0,&value)==WL_ERR_CORRUPT_PAYLOAD);
      }
}
static void payloads(void) {
  const uint32_t values[] = {0U, 1U, 63U, 64U, 127U, 128U, 16383U, 16384U, UINT32_MAX};
  request_t request = {.has_first = true, .has_second = true, .has_third = true}, decoded;
  uint8_t encoded[32], public_wire[32]; size_t length, public_length;
  for (size_t i = 0; i < sizeof(values) / sizeof(values[0]); ++i) {
    request.first = values[i]; request.second = ~values[i]; request.third = values[i] ^ UINT32_C(0xA5A5A5A5);
    assert(wire_rpc_request_encode(&request, encoded, sizeof(encoded), &length) == WL_CODEC_OK);
    assert(encoded[0] == 8U); // Tagged varuint32 inside managed RPC.
    assert(wire_rpc_request_decode(encoded, length, &decoded) == WL_CODEC_OK);
    assert(decoded.has_first && decoded.has_second && decoded.has_third && decoded.first == request.first && decoded.second == request.second && decoded.third == request.third);
    assert(request_encode(&request, public_wire, sizeof(public_wire), &public_length) == WL_CODEC_OK);
    assert(public_wire[0] == 13U && public_length == 15U); // Frozen public Schema-v1 stays fixed32.
    encoded[length] = 0x20U; encoded[length + 1U] = 0x96U; encoded[length + 2U] = 0x01U; // A newly added integer field.
    assert(wire_rpc_request_decode(encoded, length + 3U, &decoded) == WL_CODEC_OK);
    assert(decoded.first == request.first && decoded.second == request.second && decoded.third == request.third);
  }
  const uint8_t malformed[] = {8U, 0x80U, 0U, 16U, 1U, 24U, 2U};
  assert(wire_rpc_request_decode(malformed, sizeof(malformed), &decoded) == WL_CODEC_ERR_MALFORMED);
  const uint8_t overflow[] = {8U, 0xFFU, 0xFFU, 0xFFU, 0xFFU, 0x1FU, 16U, 1U, 24U, 2U};
  assert(wire_rpc_request_decode(overflow, sizeof(overflow), &decoded) == WL_CODEC_ERR_OVERFLOW);
  // Nested fixed32 changes consistently, while packed fixed32 stays fixed width.
  response_t response = {.has_reply = true, .reply = UINT32_MAX, .has_nested = true}, got;
  response.nested.has_scalar = response.nested.has_array = true;
  response.nested.scalar = UINT32_MAX;
  response.nested.array[0] = 0U; response.nested.array[1] = UINT32_MAX;
  assert(wire_rpc_response_encode(&response, encoded, sizeof(encoded), &length) == WL_CODEC_OK);
  assert(wire_rpc_response_decode(encoded, length, &got) == WL_CODEC_OK);
  assert(got.has_nested && got.nested.has_scalar && got.nested.scalar == UINT32_MAX && got.nested.has_array && got.nested.array[1] == UINT32_MAX);
}
static void names(void) {
  uint8_t encoded[16]; size_t length;
  wlc_foo_t one = {.has_wlc_foo_encode = true, .wlc_foo_encode = UINT32_MAX}, decoded;
  assert(wire_rpc_wlc_foo_encode(&one, encoded, sizeof(encoded), &length) == WL_CODEC_OK);
  assert(wire_rpc_wlc_foo_decode(encoded, length, &decoded) == WL_CODEC_OK);
  assert(decoded.has_wlc_foo_encode && decoded.wlc_foo_encode == UINT32_MAX);
  request_extra_t two = {.has_request_extra_encode = true, .request_extra_encode = UINT32_MAX,
      .has_wlc_clear = true, .wlc_clear = 19U}, got;
  assert(wire_rpc_request_extra_encode(&two, encoded, sizeof(encoded), &length) == WL_CODEC_OK);
  assert(wire_rpc_request_extra_decode(encoded, length, &got) == WL_CODEC_OK);
  assert(got.has_request_extra_encode && got.request_extra_encode == UINT32_MAX && got.has_wlc_clear && got.wlc_clear == 19U);
}
int main(void) {headers();payloads();names();return 0;}
