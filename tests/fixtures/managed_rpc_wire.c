/* SPDX-License-Identifier: Apache-2.0 */
#include "wire.h"
#include "wirelink/kernel_wire.h"
#include <assert.h>
#include <string.h>
/* Same metadata/domain cases through unified Schema-v1 codecs. */
static void headers(void) {
  const uint32_t ids[]={1U,63U,64U,127U,128U,8191U,8192U,UINT32_C(1)<<27,UINT32_MAX};
  const uint32_t sessions[]={1U,UINT32_C(0x01234567),UINT32_MAX};
  const int32_t statuses[]={0,1,-1,INT32_MIN,INT32_MAX};
  uint8_t bytes[16];
  for(size_t i=0;i<sizeof(ids)/sizeof(ids[0]);++i)
    for(size_t j=0;j<sizeof(sessions)/sizeof(sessions[0]);++j)
      for(size_t k=0;k<sizeof(statuses)/sizeof(statuses[0]);++k) {
        size_t length=0;wl_kernel_rpc_metadata_t value;
        assert(wl_wire_rpc_write(bytes,sizeof(bytes),sessions[j],ids[i],statuses[k],&length)==WL_OK);
        assert(length==wl_kernel_rpc_header_size(statuses[k]));
        assert(length==(statuses[k]?12U:8U));
        uint8_t kind=statuses[k]?WL_WIRE_RPC_ERR:WL_WIRE_RPC_RESP;
        assert(wl_wire_rpc_read(kind,bytes,length,&value)==WL_OK);
        assert(value.session==sessions[j] && value.operation==ids[i] && value.status==statuses[k] && value.length==0);
        if (!statuses[k]) assert(wl_wire_rpc_read(WL_WIRE_RPC_REQ,bytes,length,&value)==WL_OK);
        else assert(wl_wire_rpc_read(WL_WIRE_RPC_REQ,bytes,length,&value)==WL_ERR_BAD_FRAME || length==WL_KERNEL_RPC_HEADER_ERR);
        for(size_t truncated=0;truncated<length;++truncated)
          assert(wl_wire_rpc_read(kind,bytes,truncated,&value)==WL_ERR_CORRUPT_PAYLOAD);
      }
}
static void payloads(void) {
  const uint32_t values[] = {0U, 1U, 63U, 64U, 127U, 128U, 16383U, 16384U, UINT32_MAX};
  request_t request = {.has_first = true, .has_second = true, .has_third = true}, decoded;
  uint8_t public_wire[32]; size_t public_length;
  for (size_t i = 0; i < sizeof(values) / sizeof(values[0]); ++i) {
    request.first = values[i]; request.second = ~values[i]; request.third = values[i] ^ UINT32_C(0xA5A5A5A5);
    assert(request_encode(&request, public_wire, sizeof(public_wire), &public_length) == WL_CODEC_OK);
    assert(public_wire[0] == 13U && public_length == 15U);
    assert(request_decode(public_wire, public_length, &decoded) == WL_CODEC_OK);
    assert(decoded.has_first && decoded.has_second && decoded.has_third && decoded.first == request.first && decoded.second == request.second && decoded.third == request.third);
    public_wire[public_length] = 0x20U; public_wire[public_length + 1U] = 0x96U; public_wire[public_length + 2U] = 0x01U;
    assert(request_decode(public_wire, public_length + 3U, &decoded) == WL_CODEC_OK);
    assert(decoded.first == request.first && decoded.second == request.second && decoded.third == request.third);
  }
  const uint8_t wire_type_bad[] = {8U, 1U, 21U, 0U, 0U, 0U, 0U, 29U, 0U, 0U, 0U, 0U};
  assert(request_decode(wire_type_bad, sizeof(wire_type_bad), &decoded) == WL_CODEC_ERR_WIRE_TYPE);
  uint8_t encoded[32]; size_t length;
  response_t response = {.has_reply = true, .reply = UINT32_MAX, .has_nested = true}, got;
  response.nested.has_scalar = response.nested.has_array = true;
  response.nested.scalar = UINT32_MAX;
  response.nested.array[0] = 0U; response.nested.array[1] = UINT32_MAX;
  assert(response_encode(&response, encoded, sizeof(encoded), &length) == WL_CODEC_OK);
  assert(response_decode(encoded, length, &got) == WL_CODEC_OK);
  assert(got.has_nested && got.nested.has_scalar && got.nested.scalar == UINT32_MAX && got.nested.has_array && got.nested.array[1] == UINT32_MAX);
}
static void names(void) {
  uint8_t encoded[16]; size_t length;
  wlc_foo_t one = {.has_wlc_foo_encode = true, .wlc_foo_encode = UINT32_MAX}, decoded;
  assert(wlc_foo_encode(&one, encoded, sizeof(encoded), &length) == WL_CODEC_OK);
  assert(wlc_foo_decode(encoded, length, &decoded) == WL_CODEC_OK);
  assert(decoded.has_wlc_foo_encode && decoded.wlc_foo_encode == UINT32_MAX);
  request_extra_t two = {.has_request_extra_encode = true, .request_extra_encode = UINT32_MAX,
      .has_wlc_clear = true, .wlc_clear = 19U}, got;
  assert(request_extra_encode(&two, encoded, sizeof(encoded), &length) == WL_CODEC_OK);
  assert(request_extra_decode(encoded, length, &got) == WL_CODEC_OK);
  assert(got.has_request_extra_encode && got.request_extra_encode == UINT32_MAX && got.has_wlc_clear && got.wlc_clear == 19U);
}
int main(void) {headers();payloads();names();return 0;}
