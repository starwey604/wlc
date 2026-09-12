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


# Exactly one notification thread per AsyncClient, independent of RPC count.
# The native owner only signals a condition variable; it never acquires the GIL.
import asyncio
import threading
import weakref
from collections.abc import Callable
from typing import TypeVar

_Response = TypeVar("_Response")


def _drain_weak(owner: weakref.ReferenceType[AsyncConnection]) -> None:
    connection = owner()
    if connection is not None:
        connection._drain()


def _finish_close_weak(owner: weakref.ReferenceType[AsyncConnection]) -> None:
    connection = owner()
    if connection is not None:
        # The worker has finished native close; only its return remains. Join
        # before publishing closure so no notification thread escapes close().
        connection._worker.join()
        connection._drain()
        if connection._closed is not None and not connection._closed.done():
            connection._closed.set_result(None)


def _completion_pump(signal: _native.CompletionSignal,
                     loop: asyncio.AbstractEventLoop,
                     owner: weakref.ReferenceType[AsyncConnection],
                     client: _native.Client) -> None:
    try:
        while signal.wait():  # Native wait releases the GIL; no polling timer.
            try:
                loop.call_soon_threadsafe(_drain_weak, owner)
            except RuntimeError:  # Event loop already closed by its application.
                break
    finally:
        # Stop the native owner on this existing thread, without blocking the
        # event loop or depending on its default executor for connection close.
        client.close()
        try:
            loop.call_soon_threadsafe(_finish_close_weak, owner)
        except RuntimeError:
            pass


class AsyncConnection:
    # Include finished-but-undelivered calls in this bound. Even when the loop
    # stalls, neither Python-owned results nor scheduled notifications can grow
    # without bound. Native RPC admission may report a smaller endpoint limit.
    _capacity = 8

    def __init__(self, native_client: _native.Client) -> None:
        self._loop = asyncio.get_running_loop()
        self._client = native_client
        self._pending: dict[int, tuple[Any, asyncio.Future[Any], Callable[..., Any]]] = {}
        self._sequence = 0
        self._closing = False
        self._closed: asyncio.Future[None] | None = None
        self._signal = _native.CompletionSignal()
        self._worker = threading.Thread(
            target=_completion_pump,
            args=(self._signal, self._loop, weakref.ref(self), native_client),
            name="wirelink-completions", daemon=True,
        )
        self._worker.start()

    def _check_loop(self) -> None:
        if asyncio.get_running_loop() is not self._loop:
            raise RuntimeError("AsyncClient belongs to a different event loop")

    @property
    def is_open(self) -> bool:
        return not self._closing and self._client.is_open

    @property
    def local_port(self) -> int:
        return self._client.local_port

    async def _invoke(self, submit: Callable[..., Any], request: tuple[Any, ...],
                      decode: Callable[..., _Response], timeout: int) -> _Response:
        self._check_loop()
        if self._closing:
            _raise(_native.closed_error())
        if len(self._pending) >= self._capacity:
            _raise(_native.queue_full_error())
        operation, error = submit(request, timeout)
        if error is not None:
            _raise(error)
        future: asyncio.Future[_Response] = self._loop.create_future()
        self._sequence += 1
        key = self._sequence
        self._pending[key] = operation, future, decode
        # Registration handles completion before this point, without a lost wake.
        operation.notify_on_completion(self._signal)
        try:
            return await future
        except asyncio.CancelledError:
            operation.cancel()
            raise

    def _drain(self) -> None:
        for key, (operation, future, decode) in tuple(self._pending.items()):
            if not operation.done:
                continue
            del self._pending[key]
            if future.done():
                continue
            try:
                value, error = operation.result()
                if error is not None:
                    _raise(error)
                future.set_result(decode(value))
            except Exception as error:
                future.set_exception(error)

    async def close(self) -> None:
        self._check_loop()
        if self._closed is None:
            self._closing = True
            self._closed = self._loop.create_future()
            self._signal.stop()
        # Cancelling one waiter does not interrupt connection cleanup.
        await asyncio.shield(self._closed)

    def __del__(self) -> None:
        # The worker holds a weak reference, so an unused connection can be
        # collected. Deterministic cleanup remains async with / await close().
        signal = getattr(self, "_signal", None)
        if signal is not None:
            signal.stop()
        worker = getattr(self, "_worker", None)
        client = getattr(self, "_client", None)
        if client is not None and (worker is None or worker.ident is None):
            client.close()  # Construction failed before starting the pump.
