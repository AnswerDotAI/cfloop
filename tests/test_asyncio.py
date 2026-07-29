"""The loop contract, exercised through the public two-function API: a stock asyncio
SelectorEventLoop whose selector pumps Carbon during every idle wait. Each test pins one
inherited behavior that the custom selector must not break: timers, cross-thread wakeups,
subprocesses, and CF-timer interop (callbacks that fire *inside* the pump)."""
import asyncio, sys, threading, time

import pytest

if sys.platform != 'darwin': pytest.skip('CFRunLoop is macOS-only', allow_module_level=True)

import cfloop


def test_sleep_is_prompt():
    "asyncio timers work: sleep returns close to on time, not at a pump-timeout boundary"
    async def main():
        t0 = time.monotonic()
        await asyncio.sleep(0.1)
        return time.monotonic() - t0
    dt = cfloop.run(main())
    assert 0.09 < dt < 1


def test_threadsafe_wake():
    "call_soon_threadsafe from a foreign thread wakes the pump promptly via the self-pipe"
    async def main():
        loop, ev = asyncio.get_running_loop(), asyncio.Event()
        threading.Timer(0.05, lambda: loop.call_soon_threadsafe(ev.set)).start()
        t0 = time.monotonic()
        await asyncio.wait_for(ev.wait(), 5)
        return time.monotonic() - t0
    assert cfloop.run(main()) < 1


def test_subprocess():
    "asyncio subprocess machinery (child watcher, pipe transports) is inherited intact"
    async def main():
        p = await asyncio.create_subprocess_exec('echo', 'hi', stdout=asyncio.subprocess.PIPE)
        out, _ = await p.communicate()
        return out
    assert cfloop.run(main()) == b'hi\n'


def test_cf_timer_interop():
    "A CF timer fires inside the pump, on the loop thread, and wakes the selector by itself"
    async def main():
        ev = asyncio.Event()
        cfloop.call_later(0.05, ev.set)
        t0 = time.monotonic()
        await asyncio.wait_for(ev.wait(), 5)
        return time.monotonic() - t0
    assert cfloop.run(main()) < 1


def test_loop_factory_form():
    "new_event_loop composes with asyncio.Runner, the modern policy-free integration point"
    with asyncio.Runner(loop_factory=cfloop.new_event_loop) as r: assert r.run(asyncio.sleep(0.01, result=42)) == 42
