# emugrid

**See and control all your Android emulators — and iOS simulators — in one window.**
A tiny, fast, cross-platform desktop app written in Rust. Under 8 MB, no Electron, no browser engine.

> Live previews, Android device controls, and view-only iOS simulators.
> See the [roadmap](docs/ROADMAP.md).

## Why

Testing on several emulators at once means juggling many emulator windows.
Tools like Orca show every emulator side by side, but there's no small, free,
open-source app that does just that. emugrid aims to be it:

- **Lightweight first:** a single native binary, low memory, starts instantly.
- **Cross-platform:** macOS, Windows, Linux.
- **Zero setup:** finds your Android SDK (and Xcode's `simctl`) on its own.

## Features

**Grid view**
- Live tiles for running devices, with Android/iOS filters, FPS, and boot progress.
- Hover over a tile for quick controls and screenshots. Press **1–9** to open a visible device.
- Use the tile-size slider and the optional sidebar to arrange the grid. emugrid remembers both settings.
- SDK emulators stream over gRPC. Other grid previews use PNG polling (~2 fps). Off-screen tiles and minimized windows pause capture.

**Focused view**
- Switch devices from the filmstrip or dropdown. The inspector shows the system, resolution, and stream source.
- Click to tap, drag to swipe, right-click to long-press, or scroll to swipe. Supported Android devices track touch as you drag.
- Type directly into Android, or use **Send text** in the inspector. **Esc** returns to Grid.
- **Fit** scales to the stage. **100%** maps one device pixel to one physical display pixel and uses scrollbars or the wheel to pan. **Pixel-perfect** uses the largest integer downscale that fits and nearest-neighbor filtering.
- Native-pixel zoom requests full-resolution frames. ADB devices use PNG capture in these modes.

**List view**
- Search by device name, serial, or AVD ID and filter by Android emulators, physical devices, or iOS simulators.
- Start from a snapshot, cold boot, or wipe and start. Wiping requires confirmation.
- On macOS with Xcode, boot or shut down iOS simulators and open them in Simulator to interact.

**Install APKs**
- Drop an `.apk` anywhere or use **Install APK** to enter a file path. Installation targets all running Android devices.
- The notification shows progress and results per device. A downgrade failure offers **Retry with -d** for that device.

### Keyboard shortcuts

Use **Cmd** on macOS and **Ctrl** on Linux and Windows to trigger these shortcuts.

| Shortcut | Action |
|---|---|
| Cmd/Ctrl + K | Focus the device filter |
| Cmd/Ctrl + 1–9 | Switch focused device |
| Cmd/Ctrl + Left | Back |
| Cmd/Ctrl + Shift + H | Home |
| Cmd/Ctrl + Shift + R | Recents |
| Cmd/Ctrl + Down / Up | Volume down / up |
| Cmd/Ctrl + P | Power |
| Cmd/Ctrl + R | Rotate Android emulator |
| Cmd/Ctrl + S | Save a full-resolution screenshot to Desktop or your home folder |

## Requirements

- Android SDK with `platform-tools` (adb) and `emulator`.
  emugrid checks `ANDROID_HOME`, `ANDROID_SDK_ROOT`, default install folders, and your `PATH`.
  If detection fails, the setup screen lists the locations it checked. Select **Choose SDK folder…**
  (or **Main menu → Set SDK folder…**), enter the folder containing `platform-tools/adb`,
  and select **Use**. **Check again** reruns detection and refreshes the device list.
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
| Screen (focused ADB device) | `screenrecord --output-format=frames` over ADB, sized to the Fit view, up to the device resolution |
| Screen (fallback and iOS) | `adb exec-out screencap -p` / `xcrun simctl io <udid> screenshot` |
| Touch, keys, buttons | Reusable `adb shell` for `input motionevent / tap / swipe / text / keyevent`, `adb emu rotate` |
| iOS simulators | `xcrun simctl list / boot / shutdown` |
| UI | [egui](https://github.com/emilk/egui) via `eframe` (OpenGL) |

The gRPC client is a few hundred lines over `h2` with hand-written protobuf, so there's
no codegen and no `protoc` needed to build. More detail is in [docs/IDEA.md](docs/IDEA.md).

### Earlier v0.2 benchmark (MacBook, 1 gRPC emulator + 4 Orca emulators streaming)

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

MIT. The bundled font subsets are distributed under the SIL Open Font License. See the
[Inter license](assets/fonts/Inter-OFL.txt) and [JetBrains Mono license](assets/fonts/JetBrainsMono-OFL.txt).
