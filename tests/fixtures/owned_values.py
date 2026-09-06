"""C ABI ownership smoke test; no endpoint layout or Python clock callback."""
import ctypes as c
import sys


class Name(c.Structure):
    _fields_ = [("length", c.c_size_t), ("data", c.c_ubyte * 32)]


class Greeting(c.Structure):
    _fields_ = [("length", c.c_size_t), ("data", c.c_ubyte * 4)]


class Serial(c.Structure):
    _fields_ = [("length", c.c_size_t), ("data", c.c_ubyte * 4)]


class DeviceInfo(c.Structure):
    _fields_ = [
        ("has_name", c.c_bool), ("name", Name),
        ("has_greeting", c.c_bool), ("greeting", Greeting),
        ("has_serial", c.c_bool), ("serial", Serial),
        ("has_state", c.c_bool), ("state", c.c_int32),
    ]


lib = c.CDLL(sys.argv[1])
decode = lib.device_info_value_decode
decode.argtypes = [c.c_void_p, c.c_size_t, c.POINTER(DeviceInfo)]
decode.restype = c.c_int32
encode = lib.device_info_value_encode
encode.argtypes = [c.POINTER(DeviceInfo), c.c_void_p, c.c_size_t, c.POINTER(c.c_size_t)]
encode.restype = c.c_int32
wire = c.create_string_buffer(b"\x0a\x04A\0\xc3\xa9")
value = DeviceInfo()
assert decode(wire, 6, c.byref(value)) == 0
copy = DeviceInfo.from_buffer_copy(value)
c.memset(wire, 0xEE, c.sizeof(wire))
c.memset(c.byref(value), 0xEE, c.sizeof(value))
assert copy.has_name and copy.name.length == 4
assert bytes(copy.name.data[:4]) == b"A\0\xc3\xa9"
assert bytes(copy.greeting.data[:3]) == "电".encode()
assert not copy.has_greeting and copy.state == 1 and not copy.has_state
out = c.create_string_buffer(64)
length = c.c_size_t()
assert encode(c.byref(copy), out, 64, c.byref(length)) == 0
assert out.raw[:length.value] == b"\x0a\x04A\0\xc3\xa9"
