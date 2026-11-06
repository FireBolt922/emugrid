# Roadmap

## v0.1: List & control ✅ (this release)
- [x] Detect Android SDK on macOS / Windows / Linux
- [x] List AVDs + running emulators + physical devices
- [x] Start / stop emulators
- [x] Background auto-refresh
- [x] Size-optimized release build (~3 MB)

## v0.2: Live grid
- [ ] Screen thumbnail per running device (`adb screencap` first)
- [ ] Grid layout that adapts to window size
- [ ] Click a tile to enlarge it

## v0.3: Smooth streaming & control
- [ ] Emulator gRPC screen stream
- [ ] Tap / swipe / type on a tile
- [ ] Hardware buttons (back, home, recents, rotate)
- [ ] Per-tile FPS / resolution cap

## v0.4: Quality of life
- [ ] Cold boot / wipe data
- [ ] Install an APK on all running devices at once
- [ ] Remember window layout
- [ ] iOS simulators (macOS, list-only)

## Release
- [ ] GitHub Actions: build binaries for macOS (arm64 + x86_64), Windows, Linux on tag
- [ ] Homebrew tap / winget / AUR
