# The idea

## Problem

Mobile developers often test on several emulators at once (small phone, big phone,
tablet, different Android versions). Each emulator opens its own window, and keeping
track of them is messy. Some IDE tools (like Orca) show all emulators in one panel
with a device switcher, but they are heavy or tied to a specific product.

## Goal

A **small, standalone, open-source** app that shows every emulator in one window:
first as a list with controls, then as a **live grid of screens** you can interact with.

## Principles

1. **Lightweight above all.** One native binary, target < 10 MB, low idle CPU and RAM.
   No Electron, no bundled browser, no runtime to install.
2. **Cross-platform.** macOS, Windows, Linux from the same code.
3. **Uses what developers already have.** Talks to the Android SDK's own `adb` and
   `emulator`; doesn't replace or bundle them.
4. **Fast to understand.** Small codebase, few dependencies.

## Tech choices

- **Rust:** small, fast native binaries; easy cross-compiling.
- **egui / eframe:** immediate-mode GUI in pure Rust, renders with OpenGL,
  no web view. Keeps the binary around 3 MB.
- **Release profile tuned for size:** `opt-level = "s"`, LTO, `strip`, `panic = "abort"`.

## How the live grid will work (planned)

| Approach | Smoothness | Extra size | Covers |
|---|---|---|---|
| Emulator gRPC `streamScreenshot` (what Android Studio uses) | Smooth | Small (no video decoder) | Emulators |
| scrcpy protocol (H.264) | Very smooth | +2–5 MB decoder | Emulators + real phones |
| `adb exec-out screencap -p` polling | 2–5 fps | None | Everything (fallback) |

Plan: **gRPC for emulators**, `screencap` as the fallback. Input (tap / swipe / keys)
goes back via gRPC or `adb shell input`. Frame rate and resolution are capped per tile
so a grid of 6 emulators stays cheap.

## Known limits

- Needs the Android SDK installed.
- iOS simulators are macOS-only and have no live-stream API; they may be supported
  later as list-only (start / stop via `xcrun simctl`).
