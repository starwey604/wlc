/* SPDX-License-Identifier: Apache-2.0 */
#include "demo_runtime.c"
#include "test_environment.h"
#include "wirelink/loopback.h"
#include <assert.h>
#include <limits.h>

static void scalar_vectors(void) {
  values_value_t value, got, before;
  values_value_clear(&value);
  value.has_flag = value.has_u8 = value.has_u16 = value.has_u32 = value.has_u64 = true;
  value.has_i8 = value.has_i16 = value.has_i32 = value.has_i64 = value.has_mode = true;
  value.has_f32 = value.has_f64 = value.has_x = value.has_y = value.has_words = value.has_longs = true;
  value.longs[0]=0;value.longs[1]=UINT64_MAX;
  value.flag = true; value.u8 = UINT8_MAX; value.u16 = UINT16_MAX; value.u32 = UINT32_MAX; value.u64 = UINT64_MAX;
  value.i8 = INT8_MIN; value.i16 = INT16_MIN; value.i32 = INT32_MIN; value.i64 = INT64_MIN; value.mode = INT32_MAX;
  value.f32 = UINT32_MAX; value.f64 = UINT64_MAX;
  uint32_t x = UINT32_C(0xFFC12345); uint64_t y = UINT64_C(0xFFF0000000000000);
  memcpy(&value.x, &x, sizeof(x)); memcpy(&value.y, &y, sizeof(y));
  value.words[0] = UINT32_C(0x01234567); value.words[1] = UINT32_C(0x89ABCDEF);
  uint8_t wire[256]; size_t size;
  assert(values_value_encode(&value, wire, sizeof(wire), &size) == WL_CODEC_OK);
  /* Independent full-domain vector: bitmap, bool, uint, ZigZag, BE, raw IEEE, array length. */
  const uint8_t expected[] = {0,1, 0xFF,1, 0xFF,0xFF,3, 0xFF,0xFF,0xFF,0xFF,15,
    0xFF,0xFF,0xFF,0xFF,0xFF,0xFF,0xFF,0xFF,0xFF,1, 0xFF,1, 0xFF,0xFF,3,
    0xFF,0xFF,0xFF,0xFF,15, 0xFF,0xFF,0xFF,0xFF,0xFF,0xFF,0xFF,0xFF,0xFF,1,
    0xFE,0xFF,0xFF,0xFF,15, 0xFF,0xFF,0xFF,0xFF,15,
    0xFF,0xFF,0xFF,0xFF,0xFF,0xFF,0xFF,0xFF, 0xFF,0xC1,0x23,0x45,
    0xFF,0xF0,0,0,0,0,0,0, 8,1,0x23,0x45,0x67,0x89,0xAB,0xCD,0xEF,
    16,0,0,0,0,0,0,0,0,0xFF,0xFF,0xFF,0xFF,0xFF,0xFF,0xFF,0xFF};
  assert(size == sizeof(expected) && memcmp(wire, expected, size) == 0);
  assert(values_value_encoded_size(&value) == size && size <= VALUES_MAX_ENCODED_SIZE);
  assert(values_value_decode(wire, size, &got) == WL_CODEC_OK);
  assert(got.mode == INT32_MAX && got.i64 == INT64_MIN && got.u64 == UINT64_MAX && got.words[1] == value.words[1] && got.has_longs && got.longs[1] == UINT64_MAX);
  assert(memcmp(&got.x,&value.x,sizeof(x)) == 0 && memcmp(&got.y,&value.y,sizeof(y)) == 0 && !got.has_floats);
  memset(&before,0xA5,sizeof(before));
  for (size_t n=0; n<size; ++n) {got=before; assert(values_value_decode(wire,n,&got) != WL_CODEC_OK); assert(memcmp(&before,&got,sizeof(got)) == 0);}
  wire[size]=0; got=before; assert(values_value_decode(wire,size+1,&got) == WL_CODEC_ERR_MALFORMED && memcmp(&before,&got,sizeof(got))==0);
  wire[0]=2; assert(values_value_decode(wire,size,&got) == WL_CODEC_ERR_MALFORMED); wire[0]=0;
  wire[1]=2; assert(values_value_decode(wire,size,&got) == WL_CODEC_ERR_INVALID_VALUE); wire[1]=1;
  wire[size-26]=7; assert(values_value_decode(wire,size,&got) != WL_CODEC_OK); wire[size-26]=8;
  /* Every legal enum int32 value remains open; float bit patterns survive unchanged. */
  const int32_t modes[]={INT32_MIN,-123,0,1,123,INT32_MAX};
  const uint32_t floats[]={0,UINT32_C(0x80000000),UINT32_C(0x7F800000),UINT32_C(0xFF800000),UINT32_C(0x7FC12345),UINT32_C(0x7F812345),UINT32_C(0xFF812345)};
  for(size_t i=0;i<sizeof(modes)/sizeof(modes[0]);++i) for(size_t j=0;j<sizeof(floats)/sizeof(floats[0]);++j) {
    value.mode=modes[i]; memcpy(&value.x,&floats[j],4); value.has_floats=true;
    uint64_t signaling=UINT64_C(0x7FF0123456789ABC);memcpy(&value.floats[0],&signaling,8); uint64_t minus_zero=UINT64_C(0x8000000000000000); memcpy(&value.floats[1],&minus_zero,8);
    assert(values_value_encode(&value,wire,sizeof(wire),&size)==WL_CODEC_OK);
    assert(values_value_decode(wire,size,&got)==WL_CODEC_OK && got.mode==value.mode && got.has_floats);
    assert(memcmp(&got.x,&value.x,4)==0 && memcmp(got.floats,value.floats,16)==0);
    values_t view;assert(values_value_to_view(&value,&view)==WL_CODEC_OK);
    assert(memcmp(&view.x,&value.x,4)==0 && memcmp(view.floats,value.floats,16)==0);
    assert(values_value_from_view(&view,&got)==WL_CODEC_OK && memcmp(&got.x,&value.x,4)==0);
    rpc_float_value_t private_value = {.has_x=true, .has_y=true, .has_floats=true}, private_got;
    memcpy(&private_value.x,&value.x,4); memcpy(&private_value.y,&value.y,8);
    memcpy(private_value.floats,value.floats,16);
    assert(demo_rpc_rpc_float_value_encode(&private_value,wire,sizeof(wire),&size)==WL_CODEC_OK);
    assert(demo_rpc_rpc_float_value_decode(wire,size,&private_got)==WL_CODEC_OK);
    assert(memcmp(&private_got.x,&value.x,4)==0 && memcmp(&private_got.y,&value.y,8)==0 && memcmp(private_got.floats,value.floats,16)==0);
  }
}
static void small_floats(void) {
  small_value_t value, got, before; small_value_clear(&value);
  value.has_x=value.has_y=value.has_z=true;value.z=UINT64_MAX;
  const uint32_t xs[]={0,UINT32_C(0x80000000),UINT32_C(0x7F800000),UINT32_C(0xFF800000),UINT32_C(0x7FC12345),UINT32_C(0x7F812345),UINT32_C(0xFF812345)};
  const uint64_t ys[]={0,UINT64_C(0x8000000000000000),UINT64_C(0x7FF0000000000000),UINT64_C(0xFFF0000000000000),UINT64_C(0x7FF8123456789ABC),UINT64_C(0x7FF0123456789ABC),UINT64_C(0xFFF0123456789ABC)};
  for(size_t i=0;i<sizeof(xs)/sizeof(xs[0]);++i) {
    memcpy(&value.x,&xs[i],4);memcpy(&value.y,&ys[i],8);
    uint8_t wire[32];size_t n;assert(small_value_encode(&value,wire,sizeof(wire),&n)==WL_CODEC_OK && n==21);
    assert(small_value_decode(wire,n,&got)==WL_CODEC_OK && got.has_x && got.has_y && got.has_z && got.z==UINT64_MAX);
    assert(memcmp(&got.x,&value.x,4)==0 && memcmp(&got.y,&value.y,8)==0);
    memset(&before,0xA5,sizeof(before));
    for(size_t j=0;j<n;++j){got=before;assert(small_value_decode(wire,j,&got)!=WL_CODEC_OK && memcmp(&got,&before,sizeof(got))==0);}
    wire[n]=0;got=before;assert(small_value_decode(wire,n+1,&got)==WL_CODEC_ERR_MALFORMED && memcmp(&got,&before,sizeof(got))==0);
    wire[0]=2;assert(small_value_decode(wire,n,&got)==WL_CODEC_ERR_MALFORMED);
  }
}
static void primitive_errors(void) {
  narrow_value_t narrow, before; memset(&before,0xA5,sizeof(before));narrow=before;
  const uint8_t narrow_overflow[]={0x80,0x02}, noncanonical[]={0x80,0};
  assert(narrow_value_decode(narrow_overflow,sizeof(narrow_overflow),&narrow)==WL_CODEC_ERR_OVERFLOW && memcmp(&before,&narrow,sizeof(narrow))==0);
  assert(narrow_value_decode(noncanonical,sizeof(noncanonical),&narrow)==WL_CODEC_ERR_MALFORMED);
  wide_t wide;const uint8_t wide_overflow[]={0xFF,0xFF,0xFF,0xFF,0xFF,0xFF,0xFF,0xFF,0xFF,2};
  assert(wide_decode(wide_overflow,sizeof(wide_overflow),&wide)==WL_CODEC_ERR_OVERFLOW);
  open_enum_t mode;const uint8_t enum_overflow[]={0x80,0x80,0x80,0x80,0x10};
  assert(open_enum_decode(enum_overflow,sizeof(enum_overflow),&mode)==WL_CODEC_ERR_OVERFLOW);
}
static void bitmap_and_errors(void) {
  bits_t bits={.has_a=true,.a=true,.has_i=true,.i=false}, got;
  uint8_t out[32];size_t size;
  const uint8_t expected[]={1,1,1,0};
  assert(bits_encode(&bits,out,sizeof(out),&size)==WL_CODEC_OK && size==4 && memcmp(out,expected,4)==0);
  assert(bits_decode(out,size,&got)==WL_CODEC_OK && got.has_a && got.a && got.has_i && !got.i && !got.has_b);
  out[1]=2;assert(bits_decode(out,size,&got)==WL_CODEC_ERR_MALFORMED);
  empty_t empty={0}; assert(empty_encode(&empty,NULL,0,&size)==WL_CODEC_OK && size==0);
  assert(empty_decode(NULL,0,&empty)==WL_CODEC_OK && empty_decode(out,1,&empty)==WL_CODEC_ERR_MALFORMED);
  request_t request={.has_a=true,.has_b=true,.a=1,.b=UINT32_MAX}, decoded;
  assert(demo_rpc_request_encode(&request,out,sizeof(out),&size)==WL_CODEC_OK);
  assert(demo_rpc_request_decode(out,size,&decoded)==WL_CODEC_OK && decoded.a==1 && decoded.b==UINT32_MAX && !decoded.has_c && decoded.c==19);
#if POSITIONAL_RPC
  const uint8_t vector[]={0,1,0xFF,0xFF,0xFF,0xFF,15};
  assert(size==sizeof(vector) && memcmp(out,vector,size)==0);
  const uint8_t noncanonical[]={0,0x80,0,1}, overflow[]={0,0xFF,0xFF,0xFF,0xFF,0x1F,1};
  assert(demo_rpc_request_decode(noncanonical,sizeof(noncanonical),&decoded)==WL_CODEC_ERR_MALFORMED);
  assert(demo_rpc_request_decode(overflow,sizeof(overflow),&decoded)==WL_CODEC_ERR_OVERFLOW);
#endif
  request.has_b=false; assert(demo_rpc_request_encode(&request,out,sizeof(out),&size)==WL_CODEC_ERR_MISSING_REQUIRED_FIELD);
}
static demo_endpoint_t client, server;
static wl_loopback_t cable;
static unsigned completed, executed;
static wl_time_ms_t now;
static wl_time_ms_t read_clock(void *unused) {(void)unused;return now;}
static int32_t execute(void *unused,const request_value_t *req,response_value_t *res) {
  (void)unused;++executed;assert(req->a==UINT32_MAX && req->b==1 && !req->has_c && req->c==19);
  res->has_result=true;res->result=req->a;return 0;
}
static void done(void *unused,const wl_rpc_completion_t *result,const response_value_t *value) {
  (void)unused;assert(result->status==WL_RPC_SUCCESS && value && value->has_result && value->result==UINT32_MAX);++completed;
}
static void endpoints(void) {
  demo_endpoint_config_t config;
  assert(demo_endpoint_init(&client,test_environment_id(1,(wl_clock_t){read_clock,NULL}), wl_link_defaults())==WL_OK);
  assert(demo_endpoint_config_defaults(&config,test_environment_id(2,(wl_clock_t){read_clock,NULL}), wl_link_defaults())==WL_OK);
  config.on_execute=execute;
  assert(demo_endpoint_init_config(&server,&config)==WL_OK);
  assert(wl_loopback_connect(&cable,demo_endpoint_handle(&client),demo_endpoint_handle(&server))==WL_OK);
  request_value_t value;request_value_clear(&value);value.has_a=value.has_b=true;value.a=UINT32_MAX;value.b=1;
  for (unsigned call=0;call<12;++call) {
    assert(demo_endpoint_execute_async(&client,&value,100,done,NULL,NULL)==WL_OK);
    for (unsigned step=0;step<100 && completed!=call+1;++step) {assert(demo_endpoint_step(&client)==WL_OK);assert(demo_endpoint_step(&server)==WL_OK);++now;}
    assert(completed==call+1 && executed==call+1);
    for (unsigned step=0;step<5;++step) {assert(demo_endpoint_step(&client)==WL_OK);assert(demo_endpoint_step(&server)==WL_OK);++now;}
  }
  sample_t sample={.has_samples=true,.samples={1.0f,-0.0f},.has_sequence=true,.sequence=UINT32_MAX};
  assert(demo_sample_send(wl_endpoint_link(demo_endpoint_handle(&client)),&sample,WL_DELIVERY_UNRELIABLE,now).domain==DEMO_SEND_OK);
  for(unsigned i=0;i<5;++i){assert(demo_endpoint_step(&client)==WL_OK);assert(demo_endpoint_step(&server)==WL_OK);++now;}
  demo_sample_latest_view_t received; assert(demo_sample_latest_acquire(demo_endpoint_runtime(&server),&received)==WL_OK);
  assert(received.value->sequence==UINT32_MAX && memcmp(received.value->samples,sample.samples,sizeof(sample.samples))==0);
  assert(demo_sample_latest_release(demo_endpoint_runtime(&server),&received)==WL_OK);
  assert(demo_endpoint_close(&client)==WL_OK && demo_endpoint_close(&server)==WL_OK);
}
int main(void) {scalar_vectors();small_floats();primitive_errors();bitmap_and_errors();endpoints();return 0;}
