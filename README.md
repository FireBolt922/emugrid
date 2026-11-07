# emugrid

**See and control all your Android emulators — and iOS simulators — in one window.**
A tiny, fast, cross-platform desktop app written in Rust. ~4 MB, no Electron, no browser engine.

> Status: v0.2. Live screen grid, interactive device view, iOS simulators.
> See the [roadmap](docs/ROADMAP.md).

## Why

Testing on several emulators at once means juggling many emulator windows.
Tools like Orca show every emulator side by side, but there's no small, free,
open-source app that does just that. emugrid aims to be it:

- **Lightweight first:** a single native binary (~4 MB), low memory, starts instantly.
- **Cross-platform:** macOS, Windows, Linux.
- **Zero setup:** finds your Android SDK (and Xcode's `simctl`) on its own.

## Features

**Grid view**
- Every running emulator, phone and iOS simulator as a **live screen** in a phone frame
- SDK-started emulators stream over the emulator's **gRPC** API (smooth, pushed on change);
  everything else (Orca/TCP emulators, USB phones, iOS simulators) is polled as PNG (~2 fps)
- Tiles scrolled out of view, and a minimized window, **pause** their streams

**Focused view** (click a tile)
- The phone in a device frame, with a slim toolbar: device name and status, a **device picker**
  to switch phones without going back (stopped ones are listed too: pick one to start and open it), and the device buttons
- Touch works like a real finger: **click** = tap, **drag** = swipe or fling, **hold** = long press,
  **hold then drag** = move an icon, **right-click** = long press, **scroll** = swipe
- **Type** to send text and keys (Backspace, Enter, Tab, arrows); **Esc** goes back
- **Rotate** (emulators), Back, Home, Recents, screenshot to Desktop, and volume / power keys under **⋯**
- The power icon shuts the emulator or simulator down

**List view**
- Every AVD you've created, running or not, plus USB devices
- **Start**, **Stop**, **Cold boot**, **Wipe data and start**: emulators start without their own window,
  since emugrid shows them (**… → Start in its own window** if you want it)
- iOS simulators (macOS with Xcode): **Boot**, **Shut down**, **Open in Simulator**

**Everywhere**
- **Drop an `.apk`** on the window to install it on every running Android device at once
- Auto-refreshes every 3 seconds without freezing the UI

## Requirements

- Android SDK with `platform-tools` (adb) and `emulator`.
  Found through `ANDROID_HOME`, `ANDROID_SDK_ROOT`, the default install folder or your `PATH`.
- Optional, macOS: Xcode for iOS simulators. iOS simulators are view-only
  (`simctl` cannot send touches); use **Open in Simulator** to interact.

## Download

Builds are produced by the manual **Build** workflow (Actions tab → *Build* → *Run workflow*):
macOS (Apple Silicon and Intel, as `emugrid.app`), Windows (`emugrid.exe`) and Linux.
Give it a `release_tag` to publish a GitHub release too.

> The macOS app is not notarized. The first time you open it, macOS blocks it:
> go to **System Settings → Privacy & Security** and click **Open Anyway**
> (or run `xattr -dr com.apple.quarantine emugrid.app`).

## Build & run

```bash
cargo run --release
```

The binary ends up in `target/release/emugrid` (`emugrid.exe` on Windows).
On Linux you need `libxkbcommon-dev libgl1-mesa-dev libwayland-dev` to build.

## How it works

| Piece | Source |
|---|---|
| Installed emulators | `emulator -list-avds` |
| Running devices | `adb devices` |
| Which AVD a running emulator is | `adb -s <serial> emu avd name` |
| Start / stop | `emulator -avd <name>` (`-no-snapshot-load`, `-wipe-data`) / `adb emu kill` |
| Live screen (SDK emulators) | gRPC `EmulatorController/streamScreenshot`, RGB888 scaled on the emulator; port and token from the emulator's discovery file |
| Screen (everything else) | `adb exec-out screencap -p` / `xcrun simctl io <udid> screenshot` |
| Touch | A virtual touchscreen made with the device's built-in `uinput` tool (Android 11+) over one open `adb shell`, so every press, move and release arrives live. Older devices fall back to `adb shell input`. Nothing is installed. |
| Keys, buttons | `adb shell input text / keyevent`, `adb emu rotate` |
| iOS simulators | `xcrun simctl list / boot / shutdown` |
| UI | [egui](https://github.com/emilk/egui) via `eframe` (OpenGL) |

The gRPC client is a few hundred lines over `h2` with hand-written protobuf, so there's
no codegen and no `protoc` needed to build. More detail is in [docs/IDEA.md](docs/IDEA.md).

### Performance (MacBook, 1 gRPC emulator + 4 Orca emulators streaming)

| | |
|---|---|
| Release binary | 3.7 MB |
| CPU | ~3.5% of one core |
| Memory | ~103 MB RSS |

## CI

All workflows are **manual** (`workflow_dispatch`), nothing runs on push:

- **Checks:** `cargo fmt`, `clippy -D warnings`, `cargo test` on macOS, Windows, Linux
- **Build:** release binaries for all platforms, fails if any is over the 8 MB budget,
  optional GitHub release

## License

MIT
