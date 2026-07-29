# cfloop

An asyncio event loop on macOS's CFRunLoop, so coroutines, Carbon events, and main-queue delegates share the main thread.

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
