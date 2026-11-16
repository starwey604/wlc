/* SPDX-License-Identifier: Apache-2.0 */
#include "demo.h"
#include <string.h>
#include <assert.h>
#include <limits.h>

static void scalar_vectors(void) {
  values_t value, got, before;
  values_clear(&value);
  value.longs[0]=0;value.longs[1]=UINT64_MAX;
  value.flag = true; value.u8 = UINT8_MAX; value.u16 = UINT16_MAX; value.u32 = UINT32_MAX; value.u64 = UINT64_MAX;
  value.i8 = INT8_MIN; value.i16 = INT16_MIN; value.i32 = INT32_MIN; value.i64 = INT64_MIN; value.mode = INT32_MAX;
  value.f32 = UINT32_MAX; value.f64 = UINT64_MAX;
  uint32_t x = UINT32_C(0xFFC12345); uint64_t y = UINT64_C(0xFFF0000000000000);
  memcpy(&value.x, &x, sizeof(x)); memcpy(&value.y, &y, sizeof(y));
  value.words[0] = UINT32_C(0x01234567); value.words[1] = UINT32_C(0x89ABCDEF);
  uint8_t wire[256]; size_t size;
  assert(values_encode(&value, wire, sizeof(wire), &size) == WL_CODEC_OK);
  /* Independent full-domain vector: bitmap, bool, uint, ZigZag, LE, raw IEEE, array length. */
  const uint8_t expected[] = {0,1, 0xFF,1, 0xFF,0xFF,3, 0xFF,0xFF,0xFF,0xFF,15,
    0xFF,0xFF,0xFF,0xFF,0xFF,0xFF,0xFF,0xFF,0xFF,1, 0xFF,1, 0xFF,0xFF,3,
    0xFF,0xFF,0xFF,0xFF,15, 0xFF,0xFF,0xFF,0xFF,0xFF,0xFF,0xFF,0xFF,0xFF,1,
    0xFE,0xFF,0xFF,0xFF,15, 0xFF,0xFF,0xFF,0xFF,15,
    0xFF,0xFF,0xFF,0xFF,0xFF,0xFF,0xFF,0xFF, 0x45,0x23,0xC1,0xFF,
    0,0,0,0,0,0,0xF0,0xFF, 8, 0x67,0x45,0x23,0x01, 0xEF,0xCD,0xAB,0x89,
    16,0,0,0,0,0,0,0,0,0xFF,0xFF,0xFF,0xFF,0xFF,0xFF,0xFF,0xFF};
  assert(size == sizeof(expected) && memcmp(wire, expected, size) == 0);
  assert(values_encoded_size(&value) == size && size <= VALUES_MAX_ENCODED_SIZE);
  assert(values_decode(wire, size, &got) == WL_CODEC_OK);
  assert(got.mode == INT32_MAX && got.i64 == INT64_MIN && got.u64 == UINT64_MAX && got.words[1] == value.words[1] && got.longs[1] == UINT64_MAX);
  assert(memcmp(&got.x,&value.x,sizeof(x)) == 0 && memcmp(&got.y,&value.y,sizeof(y)) == 0 && !got.has_floats);
  memset(&before,0xA5,sizeof(before));
  for (size_t n=0; n<size; ++n) {got=before; assert(values_decode(wire,n,&got) != WL_CODEC_OK); assert(memcmp(&before,&got,sizeof(got)) == 0);}
  wire[size]=0; got=before; assert(values_decode(wire,size+1,&got) == WL_CODEC_ERR_MALFORMED && memcmp(&before,&got,sizeof(got))==0);
  wire[0]=2; assert(values_decode(wire,size,&got) == WL_CODEC_ERR_MALFORMED); wire[0]=0;
  wire[1]=2; assert(values_decode(wire,size,&got) == WL_CODEC_ERR_INVALID_VALUE); wire[1]=1;
  wire[size-26]=7; assert(values_decode(wire,size,&got) != WL_CODEC_OK); wire[size-26]=8;
  /* Every legal enum int32 value remains open; float bit patterns survive unchanged. */
  const int32_t modes[]={INT32_MIN,-123,0,1,123,INT32_MAX};
  const uint32_t floats[]={0,UINT32_C(0x80000000),UINT32_C(0x7F800000),UINT32_C(0xFF800000),UINT32_C(0x7FC12345),UINT32_C(0x7F812345),UINT32_C(0xFF812345)};
  for(size_t i=0;i<sizeof(modes)/sizeof(modes[0]);++i) for(size_t j=0;j<sizeof(floats)/sizeof(floats[0]);++j) {
    value.mode=modes[i]; memcpy(&value.x,&floats[j],4); value.has_floats=true;
    uint64_t signaling=UINT64_C(0x7FF0123456789ABC);memcpy(&value.floats[0],&signaling,8); uint64_t minus_zero=UINT64_C(0x8000000000000000); memcpy(&value.floats[1],&minus_zero,8);
    assert(values_encode(&value,wire,sizeof(wire),&size)==WL_CODEC_OK);
    assert(values_decode(wire,size,&got)==WL_CODEC_OK && got.mode==value.mode && got.has_floats);
    assert(memcmp(&got.x,&value.x,4)==0 && memcmp(got.floats,value.floats,16)==0);
    rpc_float_t private_value = {0}, private_got;
    memcpy(&private_value.x,&value.x,4); memcpy(&private_value.y,&value.y,8);
    memcpy(private_value.floats,value.floats,16);
    assert(rpc_float_encode(&private_value,wire,sizeof(wire),&size)==WL_CODEC_OK);
    assert(rpc_float_decode(wire,size,&private_got)==WL_CODEC_OK);
    assert(memcmp(&private_got.x,&value.x,4)==0 && memcmp(&private_got.y,&value.y,8)==0 && memcmp(private_got.floats,value.floats,16)==0);
  }
}
static void small_floats(void) {
  small_t value, got, before; small_clear(&value);
  value.has_x=true;value.z=UINT64_MAX;
  const uint32_t xs[]={0,UINT32_C(0x80000000),UINT32_C(0x7F800000),UINT32_C(0xFF800000),UINT32_C(0x7FC12345),UINT32_C(0x7F812345),UINT32_C(0xFF812345)};
  const uint64_t ys[]={0,UINT64_C(0x8000000000000000),UINT64_C(0x7FF0000000000000),UINT64_C(0xFFF0000000000000),UINT64_C(0x7FF8123456789ABC),UINT64_C(0x7FF0123456789ABC),UINT64_C(0xFFF0123456789ABC)};
  for(size_t i=0;i<sizeof(xs)/sizeof(xs[0]);++i) {
    memcpy(&value.x,&xs[i],4);memcpy(&value.y,&ys[i],8);
    uint8_t wire[32];size_t n;assert(small_encode(&value,wire,sizeof(wire),&n)==WL_CODEC_OK && n==21);
    assert(small_decode(wire,n,&got)==WL_CODEC_OK && got.has_x && got.z==UINT64_MAX);
    assert(memcmp(&got.x,&value.x,4)==0 && memcmp(&got.y,&value.y,8)==0);
    memset(&before,0xA5,sizeof(before));
    for(size_t j=0;j<n;++j){got=before;assert(small_decode(wire,j,&got)!=WL_CODEC_OK && memcmp(&got,&before,sizeof(got))==0);}
    wire[n]=0;got=before;assert(small_decode(wire,n+1,&got)==WL_CODEC_ERR_MALFORMED && memcmp(&got,&before,sizeof(got))==0);
    wire[0]=2;assert(small_decode(wire,n,&got)==WL_CODEC_ERR_MALFORMED);
  }
}
static void primitive_errors(void) {
  narrow_t narrow, before; memset(&before,0xA5,sizeof(before));narrow=before;
  const uint8_t narrow_overflow[]={0x80,0x02}, noncanonical[]={0x80,0};
  assert(narrow_decode(narrow_overflow,sizeof(narrow_overflow),&narrow)==WL_CODEC_ERR_OVERFLOW && memcmp(&before,&narrow,sizeof(narrow))==0);
  assert(narrow_decode(noncanonical,sizeof(noncanonical),&narrow)==WL_CODEC_ERR_MALFORMED);
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
  request_t request={.a=1,.b=UINT32_MAX}, decoded;
  assert(request_encode(&request,out,sizeof(out),&size)==WL_CODEC_OK);
  assert(request_decode(out,size,&decoded)==WL_CODEC_OK && decoded.a==1 && decoded.b==UINT32_MAX && !decoded.has_c && decoded.c==19);
#if 1
  const uint8_t vector[]={0,1,0xFF,0xFF,0xFF,0xFF,15};
  assert(size==sizeof(vector) && memcmp(out,vector,size)==0);
  const uint8_t noncanonical[]={0,0x80,0,1}, overflow[]={0,0xFF,0xFF,0xFF,0xFF,0x1F,1};
  assert(request_decode(noncanonical,sizeof(noncanonical),&decoded)==WL_CODEC_ERR_MALFORMED);
  assert(request_decode(overflow,sizeof(overflow),&decoded)==WL_CODEC_ERR_OVERFLOW);
#endif
  assert(request_decode(out, 2U, &decoded) == WL_CODEC_ERR_MALFORMED);
}
int main(void) { scalar_vectors(); small_floats(); primitive_errors(); bitmap_and_errors(); return 0; }
