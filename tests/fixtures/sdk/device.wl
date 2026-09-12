version 1;

enum Mode @id(1) { MODE_IDLE = 0; MODE_ACTIVE = 1; }
message InfoRequest @id(2) {}
message Settings @id(3) {
  optional string<12> label @id(1) [default = "设备"];
  required bool enabled @id(2);
  optional Mode mode @id(3) [default = 1];
  required bytes<8> token @id(4);
  required packed float32 gains[3] @id(5);
  packed fixed64 counters[2] @id(6);
  optional int64 floor @id(7) [default = -9223372036854775808];
  optional uint64 ceiling @id(8) [default = 18446744073709551615];
  optional string<8> class @id(9);
  optional uint16 timeout @id(10);
}
message InfoResponse @id(4) {
  required string<32> name @id(1);
  required Settings settings @id(2);
}
message ConfigureRequest @id(5) {
  required Settings settings @id(1);
  optional bytes<4> opaque @id(2);
  optional Settings backup @id(3);
}
message ConfigureResponse @id(6) {
  required Settings settings @id(1);
  optional bytes<4> opaque @id(2);
  optional Settings backup @id(3);
  required uint32 count @id(4);
}
message Numbers @id(7) {
  required int8 i8 @id(1);
  required uint8 u8 @id(2);
  required int16 i16 @id(3);
  required uint16 u16 @id(4);
  required int32 i32 @id(5);
  required uint32 u32 @id(6);
  required int64 i64 @id(7);
  required uint64 u64 @id(8);
  required fixed32 f32 @id(9);
  required fixed64 f64 @id(10);
  required float32 single @id(11);
  required float64 double @id(12);
  packed float64 doubles[2] @id(13);
  required packed fixed32 words[2] @id(14);
  optional int64 offset @id(15) [default = -7];
  optional int32 minimum @id(16) [default = -2147483648];
  optional bool flag @id(17) [default = true];
  optional string<16> text @id(18) [default = "a\"@NAME@"];
}

// Public response names must not collide with local C++ operation machinery.
message CallRequest @id(100) {}
message Call @id(101) {}
