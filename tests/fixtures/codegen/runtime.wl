version 1;
message Telemetry @id(1) { packed float32 values[30] @id(1); }
message Event @id(2) { required uint32 code @id(1); }
message Request @id(3) { required string<32> label @id(1); optional bytes<128> data @id(2); }
message Response @id(4) { optional string<64> result @id(1); }
message QueryRequest @id(5) {}
message QueryResponse @id(6) { optional uint32 status @id(1); }
message LegacyRequest @id(7) { optional uint32 operation_id @id(1); repeated uint32 values @id(2); }
enum Status @id(8) { SUCCESS = 0; FAILURE = 1; }
message LegacyResponse @id(9) { optional uint32 call_id @id(1); optional Status status @id(2); optional bytes data @id(3); }
