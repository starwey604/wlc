profile version 2;
endpoint { envelope=native_packet; }
rpc GetInfo { request = InfoRequest; response = InfoResponse; }
rpc Configure { request = ConfigureRequest; response = ConfigureResponse; }
rpc EchoCall {
  request = CallRequest;
  response = Call;
}
