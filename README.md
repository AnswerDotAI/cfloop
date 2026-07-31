# cfloop

An `asyncio` event loop on macOS's CFRunLoop, so coroutines, Carbon events, and main-queue delegates share the main thread.

```python
import asyncio, cfloop

async def main():
    await asyncio.sleep(1)  # Carbon events and main-queue callbacks keep dispatching meanwhile

# the asyncio.run twin, which must own the main thread
cfloop.run(main())
# or compose it yourself (also asyncio.Runner)
asyncio.run(main(), loop_factory=cfloop.new_event_loop)
```

That is the whole API. The loop is a stock `asyncio.SelectorEventLoop` whose selector waits inside a Carbon event pump instead of a bare kqueue. So whenever `asyncio` is idle, Carbon events dispatch (hotkeys included), CFRunLoop timers and sources fire, and the main dispatch queue drains. Everything `asyncio` provides is inherited, not reimplemented: tasks, subprocesses, signal handling, debug mode's slow-callback warnings. macOS only, Python 3.12+.

The substrate is a small `pyo3` module:

- `pump`: manual `ReceiveNextEvent` dispatch, the part macOS won't do for you (`DEV.md` has the probe history)
- `FdWatch`: a `CFSocket` source that wakes the pump when a registered file descriptor becomes ready
- `post_wake`: post a no-op Carbon event from any thread, to break the pump out of its wait
- `call_later`: a `CFRunLoopTimer` that calls back into Python
- `run_app`/`quit_app`: for callers who want Carbon's own `RunApplicationEventLoop` to own the thread instead

## Development

```bash
pip install -e .[dev]
maturin develop && pytest -q
```

## Build

```bash
ship-rs-build
```

## Release

```bash
maturin develop && pytest -q
ship-release
```

`ship-release` tags the Cargo version, leaves wheel publication to GitHub Actions, then bumps the project.
