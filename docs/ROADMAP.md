# Roadmap

## v0.1: List & control ✅ (this release)
- [x] Detect Android SDK on macOS / Windows / Linux
- [x] List AVDs + running emulators + physical devices
- [x] Start / stop emulators
- [x] Background auto-refresh
- [x] Size-optimized release build (~3 MB)

## v0.2: Live grid ([plan](plans/v0.2-live-grid.md))
- [ ] Grid of live screen tiles, List / Grid toggle
- [ ] PNG fallback via `adb screencap` (~1 fps, works for every device)
- [ ] Emulator gRPC stream for SDK-started emulators (≥ 10 fps)
- [ ] Click a tile to enlarge it
- [ ] Binary stays ≤ 8 MB (checked in CI)

## v0.3: Control & smooth streaming everywhere
- [ ] Tap / swipe / type on a tile (gRPC `sendTouch` / `sendKey`)
- [ ] Hardware buttons (back, home, recents, rotate)
- [ ] scrcpy-based stream for TCP emulators (e.g. Orca's) and real phones

## v0.4: Quality of life
- [ ] Cold boot / wipe data
- [ ] Install an APK on all running devices at once
- [ ] Remember window layout
- [ ] iOS simulators (macOS, list-only)

## Release
- [ ] GitHub Actions: build binaries for macOS (arm64 + x86_64), Windows, Linux on tag
- [ ] Homebrew tap / winget / AUR
