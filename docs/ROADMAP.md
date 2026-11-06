# Roadmap

## v0.1: List & control ✅
- [x] Detect Android SDK on macOS / Windows / Linux
- [x] List AVDs + running emulators + physical devices
- [x] Start / stop emulators
- [x] Background auto-refresh
- [x] Size-optimized release build (~3 MB)

## v0.2: Live grid ✅ ([plan](plans/v0.2-live-grid.md))
- [x] Grid of live screen tiles, List / Grid toggle
- [x] PNG fallback via `adb screencap` (~2 fps, works for every device)
- [x] Emulator gRPC stream for SDK-started emulators (≥ 10 fps)
- [x] Click a tile to enlarge it
- [x] Binary stays ≤ 8 MB (checked in the Build workflow)

## v0.3: Control & smooth streaming everywhere
- [x] Tap / swipe / type on a device (via `adb shell input`; shipped in v0.2)
- [x] Hardware buttons (back, home, recents, volume, power, rotate; shipped in v0.2)
- [ ] Lower-latency input over gRPC `sendTouch` / `sendKey` for SDK emulators
- [ ] scrcpy-based stream for TCP emulators (e.g. Orca's) and real phones

## v0.4: Quality of life
- [x] Cold boot / wipe data (shipped in v0.2)
- [x] Install an APK on all running devices at once (drag and drop; shipped in v0.2)
- [ ] Remember window layout
- [x] iOS simulators (macOS: list, boot, shut down, view-only screen; shipped in v0.2)

## Release
- [x] GitHub Actions (manual): build binaries for macOS (arm64 + x86_64), Windows, Linux; optional release
- [ ] Homebrew tap / winget / AUR
