"""The gating probe for the whole design: a Rust-created CFRunLoopTimer fires a Python
callable inside Carbon's RunApplicationEventLoop on the main thread, and the callback can
quit the loop from the loop's own thread. Green here means asyncio-on-CFRunLoop is buildable
(the pyobjc rehearsal of the same facts is macmage's DEV.md 1.13)."""
import sys, threading, time

import pytest

if sys.platform != 'darwin': pytest.skip('CFRunLoop is macOS-only', allow_module_level=True)

import cfloop


def test_timer_fires_inside_carbon_loop():
    "Schedule before run: the timer fires on the main thread, and its callback quits the loop"
    got = []
    def cb():
        got.append(threading.current_thread() is threading.main_thread())
        cfloop.quit_app()
    cfloop.call_later(0.05, cb)
    cfloop.call_later(5, cfloop.quit_app)  # failsafe: a dead callback must not hang the suite
    t0 = time.monotonic()
    cfloop.run_app()
    assert got == [True], 'callback did not run on the main thread inside the loop'
    assert time.monotonic() - t0 < 4, 'the loop only exited via the failsafe'
