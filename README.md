# emugrid

**See and control all your Android emulators in one place.**
A tiny, fast, cross-platform desktop app written in Rust. ~3 MB, no Electron, no browser engine.

> Status: early (v0.1). Lists emulators and devices, starts and stops them.
> The live screen grid is next. See the [roadmap](docs/ROADMAP.md).

## Why

Testing on several emulators at once means juggling many emulator windows.
Tools like Orca show every emulator side by side, but there's no small, free,
open-source app that does just that. emugrid aims to be it:

- **Lightweight first:** a single native binary (~3 MB), low memory, starts instantly.
- **Cross-platform:** macOS, Windows, Linux.
- **Zero setup:** finds your Android SDK on its own.

## What it does today

- Lists every AVD you've created, running or not
- Shows running emulators **and** USB-connected phones (via `adb`)
- One-click **Start** / **Stop** for emulators
- Auto-refreshes every 3 seconds without freezing the UI

## Requirements

- Android SDK with `platform-tools` (adb) and `emulator`.
  Found through `ANDROID_HOME`, `ANDROID_SDK_ROOT`, the default install folder or your `PATH`.

## Build & run

```bash
cargo run --release
```

The binary ends up in `target/release/emugrid` (`emugrid.exe` on Windows).

## How it works

| Piece | Source |
|---|---|
| Installed emulators | `emulator -list-avds` |
| Running devices | `adb devices` |
| Which AVD a running emulator is | `adb -s <serial> emu avd name` |
| Start / stop | `emulator -avd <name>` / `adb -s <serial> emu kill` |
| UI | [egui](https://github.com/emilk/egui) via `eframe` (OpenGL) |

More detail is in [docs/IDEA.md](docs/IDEA.md).

## License

MIT
