profile version 1;

rpc ApplyControl {
  request = Control;
  response = ControlResult;
  request_operation_id = operation_id;
  response_operation_id = operation_id;
  response_status = result;
  request_delivery = reliable;
  response_delivery = reliable;
}
