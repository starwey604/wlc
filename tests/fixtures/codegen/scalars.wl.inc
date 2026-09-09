enum Mode @id(100) { OFF = 0; ON = 1; }
message Scalars @id(101) {
  optional bool enabled @id(1) [default = true];
  optional int8 i8 @id(2) [default = -12];
  optional uint8 u8 @id(3) [default = 255];
  optional int16 i16 @id(4) [default = -1234];
  optional uint16 u16 @id(5) [default = 65535];
  optional int32 i32 @id(6) [default = -12345];
  optional uint32 u32 @id(7) [default = 12345];
  optional int64 i64 @id(8) [default = -9223372036854775808];
  optional uint64 u64 @id(9) [default = 18446744073709551615];
  optional fixed32 f32 @id(10) [default = 123];
  optional fixed64 f64 @id(11) [default = 456];
  optional float32 single @id(12);
  optional float64 wide @id(13);
  optional Mode mode @id(14) [default = 1];
  optional string<32> label @id(15) [default = "mail@HOST@.test"];
  required bytes<16> data @id(2048);
}
message Parent @id(102) { optional Scalars child @id(16); }
message Borrowed @id(103) { repeated Scalars children @id(1); optional string text @id(2); }
message Packed32 @id(104) { packed float32 samples[30] @id(1); }
message Packed64 @id(105) { required packed float64 samples[64] @id(65535); }
message PackedFixed @id(106) { required packed fixed64 samples[1] @id(16); }
message PackedFixed32 @id(107) { packed fixed32 samples[128] @id(21); }
message MixedPacked @id(108) { packed float32 samples[3] @id(1); optional uint32 sequence @id(2); }
