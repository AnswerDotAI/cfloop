"""An asyncio event loop on macOS's CFRunLoop: a stock `SelectorEventLoop` whose selector pumps
Carbon events during every idle wait, so coroutines, Carbon events, and main-queue work all
share the main thread. See DEV.md for why dispatch must be manual."""
import asyncio, selectors, threading

from . import _core
from ._core import __version__, FdWatch, call_later, post_wake, pump, quit_app, run_app

__all__ = ["__version__", "new_event_loop", "run", "call_later", "post_wake", "pump", "quit_app", "run_app"]


class _CarbonSelector(selectors.KqueueSelector):
    """A KqueueSelector whose blocking wait is the Carbon pump. An `FdWatch` on the kqueue's own
    fd posts a wake event when any registered fd turns ready, so the pump pops for fd traffic,
    Carbon events, and wake posts alike; readiness is then collected without blocking. The watch
    is level-triggered with no arming state (see DEV.md), so a missed wake self-heals on the
    next wait."""
    def __init__(self):
        super().__init__()
        self._watch = _core.FdWatch(self._selector.fileno())

    def select(self, timeout=None):
        ready = super().select(0)
        if ready or timeout == 0: _core.pump(0.0)  # never starve Carbon under fd load
        else:
            _core.pump(-1.0 if timeout is None else timeout)  # kEventDurationForever is -1
            ready = super().select(0)
        return ready

    def close(self):
        self._watch.invalidate()
        super().close()


def new_event_loop():
    "A stock asyncio `SelectorEventLoop` on a Carbon-pumping selector; usable as a `loop_factory`"
    return asyncio.SelectorEventLoop(_CarbonSelector())


def run(
    coro, # A coroutine, run like `asyncio.run(coro)`
    debug:bool=None, # Passed to `asyncio.Runner`; True enables slow-callback warnings etc.
):
    "The `asyncio.run` twin that owns the main thread, where Carbon events and main-queue work need it"
    if threading.current_thread() is not threading.main_thread():
        raise RuntimeError('cfloop.run must own the main thread: Carbon events and main-queue work only dispatch there')
    with asyncio.Runner(loop_factory=new_event_loop, debug=debug) as r: return r.run(coro)
