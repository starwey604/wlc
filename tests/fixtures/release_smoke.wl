version 1;

enum Result = 1 {
  OK = 0;
  FAILED = 1;
}

message Control = 2 {
  required uint32 operation_id = 1;
  required packed float32 joints[6] = 2;
}

message ControlResult = 3 {
  required uint32 operation_id = 1;
  required Result result = 2;
}
