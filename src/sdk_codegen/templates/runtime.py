# SPDX-License-Identifier: Apache-2.0
"""Private support copied into each generated SDK; no shared runtime wheel is needed."""
from __future__ import annotations

from dataclasses import dataclass
from enum import IntEnum
import math
import struct
from typing import NoReturn, Any

from . import _native

class WirelinkError(Exception):
    """A native failure, retaining the separate RPC diagnostic domains."""

    def __init__(self, error: _native.Error) -> None:
        self.kind = error.kind
        self.status = error.status
        self.rejection = error.rejection
        self.local_error = error.local_error
        self.transport_error = error.transport_error
        self.runtime_error = error.runtime_error
        self.codec_error = error.codec_error
        self.os_error = error.os_error
        self.os_message = error.os_message
        super().__init__(
            f"{self.kind}: status={self.status}, rejection={self.rejection}, "
            f"local={self.local_error}, transport={self.transport_error}, "
            f"runtime={self.runtime_error}, codec={self.codec_error}, os={self.os_error}"
        )


class ClosedError(WirelinkError):
    """The connection is closed."""


class RpcTimeoutError(WirelinkError, TimeoutError):
    """The call's deadline expired, including time spent in the owner queue."""


class CancelledError(WirelinkError):
    """An admitted call was cancelled by connection shutdown."""


class RejectedError(WirelinkError):
    """The service rejected the request; rejection contains its business code."""


class QueueFullError(WirelinkError):
    """The bounded call capacity is exhausted."""


class TransportError(WirelinkError):
    """A local transport operation or link transaction failed."""


class CodecError(WirelinkError):
    """The payload could not be encoded or decoded."""


class InvalidArgumentError(WirelinkError, ValueError):
    """Native configuration validation failed."""


def _raise(error: _native.Error) -> NoReturn:
    exception = {
        "closed": ClosedError,
        "timed_out": RpcTimeoutError,
        "cancelled": CancelledError,
        "rejected": RejectedError,
        "queue_full": QueueFullError,
        "transport": TransportError,
        "codec": CodecError,
        "invalid_argument": InvalidArgumentError,
    }.get(error.kind, WirelinkError)
    raise exception(error)


def _integer(value: int, name: str, minimum: int, maximum: int) -> None:
    if not isinstance(value, int) or isinstance(value, bool):
        raise TypeError(f"{name} must be an integer")
    if not minimum <= value <= maximum:
        raise ValueError(f"{name} must be in [{minimum}, {maximum}]")


@dataclass(frozen=True, slots=True)
class Udp:
    peer: tuple[str, int]
    bind: tuple[str, int] = ("127.0.0.1", 0)

    def __post_init__(self) -> None:
        for name, address, minimum in (("peer", self.peer, 1), ("bind", self.bind, 0)):
            if not isinstance(address, tuple) or len(address) != 2:
                raise TypeError(f"{name} must be an (IP address, port) tuple")
            if not isinstance(address[0], str) or not address[0] or "\0" in address[0]:
                raise ValueError(f"{name} IP address must be a nonempty string without NUL")
            _integer(address[1], f"{name} port", minimum, 65535)



class OpenIntEnum(IntEnum):
    @classmethod
    def _missing_(cls, value: object) -> OpenIntEnum:
        _integer(value, cls.__name__, -(2**31), 2**31 - 1)
        # Unknown values are owned, unnamed instances; do not grow a global cache.
        member = int.__new__(cls, value)
        member._name_ = None
        member._value_ = value
        return member


def integer(value: int, name: str, minimum: int, maximum: int) -> int:
    _integer(value, name, minimum, maximum)
    return value


def boolean(value: bool, name: str) -> bool:
    if not isinstance(value, bool):
        raise TypeError(f"{name} must be bool")
    return value


def real(value: float, name: str, bits: int) -> float:
    if not isinstance(value, (int, float)) or isinstance(value, bool):
        raise TypeError(f"{name} must be a number")
    try:
        result = float(value)
        struct.pack("=f" if bits == 32 else "=d", result)
    except (OverflowError, struct.error) as error:
        raise ValueError(f"{name} exceeds float{bits} range") from error
    return result


def string(value: str, name: str, maximum: int) -> str:
    if not isinstance(value, str):
        raise TypeError(f"{name} must be str")
    if len(value.encode("utf-8")) > maximum:
        raise ValueError(f"{name} exceeds {maximum} UTF-8 bytes")
    return value


def blob(value: bytes, name: str, maximum: int) -> bytes:
    if not isinstance(value, (bytes, bytearray, memoryview)):
        raise TypeError(f"{name} must be bytes, bytearray or memoryview")
    result = bytes(value)
    if len(result) > maximum:
        raise ValueError(f"{name} exceeds {maximum} bytes")
    return result


def array(value: tuple[Any, ...], name: str, count: int) -> tuple[Any, ...]:
    if not isinstance(value, (tuple, list)):
        raise TypeError(f"{name} must be a tuple or list")
    if len(value) != count:
        raise ValueError(f"{name} must contain exactly {count} elements")
    return tuple(value)


def message(value: Any, name: str, cls: type) -> Any:
    if not isinstance(value, cls):
        raise TypeError(f"{name} must be {cls.__name__}")
    return value


def timeout_ms(timeout: float) -> int:
    if not isinstance(timeout, (int, float)) or isinstance(timeout, bool):
        raise TypeError("timeout must be a finite positive number of seconds")
    if not 0 < timeout <= (2**31 - 1) / 1000 or not math.isfinite(timeout):
        raise ValueError("timeout must be positive and at most 2147483.647 seconds")
    return math.ceil(timeout * 1000)
