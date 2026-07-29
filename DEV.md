# Development

## Commands

```bash
maturin develop && pytest -q
ship-rs-build
```

## Versioning

The canonical version lives in `Cargo.toml`. `pyproject.toml` gets the Python package version from Cargo via `dynamic = ["version"]`.

## Release

1. Run `maturin develop && pytest -q`.
2. Confirm the release version in `Cargo.toml` (`[package].version`).
3. Run `ship-release`.

Fastship commits the changelog, pushes the version tag for GitHub Actions, then bumps and pushes `Cargo.toml`.

## How Carbon events actually get dispatched (probed 2026-07-29)

The design goal is a stock `asyncio.SelectorEventLoop` whose custom selector pumps macOS events during every idle wait, so asyncio semantics are inherited rather than reimplemented. That hinges on dispatching Carbon events (hotkeys, notably) without being inside `RunApplicationEventLoop`. Four probes, all with a registered hotkey and a synthetic press, run under Imp with macmage's engine in `_own_loop` mode:

- `CFRunLoopRunInMode(kCFRunLoopDefaultMode, ...)` slices: NOT delivered. Running the raw run loop does not dispatch Carbon events.
- Carbon's own `RunCurrentEventLoop(timeout)` slices: NOT delivered, despite its name and docs suggesting otherwise.
- Entering `RunApplicationEventLoop` once, quitting, then `RunCurrentEventLoop` slices: NOT delivered. It is not one-time initialization; dispatch itself is the missing piece.
- Manual dispatch, `ReceiveNextEvent` + `SendEventToEventTarget(GetEventDispatcherTarget())` + `ReleaseEvent` (the classic Carbon manual loop, now `pump()` here): DELIVERED, press and release both, first try.

So `pump(timeout)` is the selector's core: it blocks in the run loop (so CF timers, sources, and main-queue work fire during it), wakes on Carbon events, and dispatches them by hand. `RunApplicationEventLoop`/`quit` stay exported for callers who want Carbon to own the thread instead (macmage's current `run_loop` arrangement, whose facts live in macmage DEV.md 1.13).

The pyobjc rehearsals of the related run-loop facts (cross-thread quit is a silent no-op; `CFRunLoopPerformBlock` + `CFRunLoopWakeUp` is the cross-thread scheduling trampoline; F-keys silently eat synthetic presses in tests, use punctuation combos) are in macmage's DEV.md.

## The selector design (2026-07-29)

`_CarbonSelector` subclasses `KqueueSelector` and changes only `select()`: collect ready fds without blocking, and when there are none, block in `pump()` instead of the kqueue. A single `FdWatch` on the kqueue's *own* fd (kqueues are pollable: readable when events are pending) posts a wake event from pure Rust when any registered fd turns ready, so the pump pops for fd traffic without a helper thread or a GIL acquisition. When fds are already ready, a zero-timeout pump still runs so Carbon is never starved under load. Everything else - the loop, transports, child watcher, signal wakeups - is stock asyncio reaching us through the self-pipe, which is just another watched fd.

The one rule this design imposes: a callback that fires *inside* the pump (a CF timer, a main-queue block) and schedules asyncio work must also wake the selector, or its work waits for the pump's timeout. Carbon events need nothing (dispatching one pops the pump by definition), `call_later` posts the wake itself after its callback, and `loop.call_soon_threadsafe` is safe from anywhere because it writes the self-pipe. So the standard asyncio thread-safety rule doubles as the pump-safety rule; `post_wake` exists for foreign in-pump callbacks that schedule work by other means.

`run()` refuses non-main threads (Carbon events and the main queue only dispatch on main) and wraps `asyncio.Runner`, which supplies the shutdown ordering GUI loops historically botch: pending-task cancellation, asyncgen and executor cleanup. Floor is 3.12 for `asyncio.run(loop_factory=)` and the settled child-watcher deprecations. Worth adopting when the loop grows ambitions: running CPython's own asyncio test suite against `new_event_loop` in CI, the uvloop/qasync acceptance bar.
