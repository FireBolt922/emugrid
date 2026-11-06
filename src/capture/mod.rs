//! Screen capture backends. Each running device gets one worker thread that
//! pushes `Frame`s; the UI only ever sees the latest frame per device.

pub mod grpc;
pub mod png;
pub mod proto;

use crate::sdk;
use eframe::egui;
use std::collections::HashMap;
use std::path::PathBuf;
use std::sync::atomic::{AtomicBool, AtomicU32, AtomicU64, Ordering};
use std::sync::{Arc, Mutex};
use std::thread;
use std::time::{Duration, Instant};

/// Frame rate caps. gRPC frames are pushed on change; PNG is polled.
pub const TILE_FPS: u32 = 15;
pub const FOCUSED_FPS: u32 = 30;
const PNG_FPS: u32 = 2;

/// One screen image, tightly packed RGB.
#[derive(Debug, Clone)]
pub struct Frame {
    pub width: u32,
    pub height: u32,
    pub rgb: Vec<u8>,
}

/// Shared knobs between the UI and a capture worker.
#[derive(Debug)]
pub struct Control {
    /// Worker should exit.
    pub stop: AtomicBool,
    /// Tile is on screen; when false the worker idles.
    pub active: AtomicBool,
    /// Longest edge, in pixels, the UI wants frames at.
    pub size: AtomicU32,
    pub max_fps: AtomicU32,
    /// True while frames come from the gRPC stream (vs. PNG polling).
    pub live: AtomicBool,
    /// Frames delivered so far.
    pub frames: AtomicU64,
    /// Only the newest frame is kept, so a UI that isn't drawing can't
    /// make frames pile up.
    latest: Mutex<Option<Frame>>,
}

impl Control {
    pub fn new(size: u32) -> Self {
        Self {
            stop: AtomicBool::new(false),
            active: AtomicBool::new(true),
            size: AtomicU32::new(size),
            max_fps: AtomicU32::new(TILE_FPS),
            live: AtomicBool::new(false),
            frames: AtomicU64::new(0),
            latest: Mutex::new(None),
        }
    }

    pub fn publish(&self, frame: Frame) {
        self.frames.fetch_add(1, Ordering::Relaxed);
        if let Ok(mut slot) = self.latest.lock() {
            *slot = Some(frame);
        }
    }

    pub fn take_frame(&self) -> Option<Frame> {
        self.latest.lock().ok()?.take()
    }
}

/// Where a device's screen comes from.
#[derive(Debug, Clone)]
pub enum Source {
    /// SDK emulator with gRPC; falls back to adb PNG if the stream fails.
    Grpc { ep: grpc::Endpoint, adb: PathBuf },
    /// Any adb device (TCP emulators, phones).
    Adb { adb: PathBuf },
    /// iOS simulator, by UDID.
    Simctl,
}

/// Owns one worker thread per streamed device.
pub struct Manager {
    workers: HashMap<String, Arc<Control>>,
    ctx: egui::Context,
}

impl Manager {
    pub fn new(ctx: egui::Context) -> Self {
        Self {
            workers: HashMap::new(),
            ctx,
        }
    }

    /// Start workers for new devices and stop those that went away.
    pub fn sync(&mut self, wanted: Vec<(String, Source)>) {
        let keep: Vec<&String> = wanted.iter().map(|(id, _)| id).collect();
        self.workers.retain(|id, c| {
            let alive = keep.contains(&id);
            if !alive {
                c.stop.store(true, Ordering::Relaxed);
            }
            alive
        });
        for (id, source) in wanted {
            if self.workers.contains_key(&id) {
                continue;
            }
            let control = Arc::new(Control::new(320));
            control.active.store(false, Ordering::Relaxed);
            self.workers.insert(id.clone(), control.clone());
            let ctx = self.ctx.clone();
            thread::spawn(move || {
                let send = |f: Frame| {
                    control.publish(f);
                    ctx.request_repaint();
                };
                run(&id, source, &control, send);
            });
        }
    }

    pub fn control(&self, id: &str) -> Option<&Arc<Control>> {
        self.workers.get(id)
    }

    pub fn controls(&self) -> impl Iterator<Item = (&String, &Arc<Control>)> {
        self.workers.iter()
    }

    /// The newest frame per device that arrived since the last call.
    pub fn frames(&self) -> impl Iterator<Item = (String, Frame)> + '_ {
        self.workers
            .iter()
            .filter_map(|(id, c)| Some((id.clone(), c.take_frame()?)))
    }
}

impl Drop for Manager {
    fn drop(&mut self) {
        for c in self.workers.values() {
            c.stop.store(true, Ordering::Relaxed);
        }
    }
}

/// After falling back to PNG, try gRPC again this often (emulator restarted,
/// discovery file appeared, transient error).
const GRPC_RETRY: Duration = Duration::from_secs(30);

fn run(id: &str, source: Source, control: &Control, send: impl Fn(Frame)) {
    match source {
        Source::Grpc { ep, adb } => loop {
            let mut failures = 0;
            while failures < 3 {
                if !wait_until_active(control) {
                    return;
                }
                // Re-read the discovery file each time: port and token change
                // when the emulator restarts.
                let ep = grpc::discover().remove(id).unwrap_or_else(|| ep.clone());
                let before = control.frames.load(Ordering::Relaxed);
                let result = grpc::stream(&ep, control, &send);
                if control.frames.load(Ordering::Relaxed) > before {
                    failures = 0;
                }
                if result.is_err() {
                    failures += 1;
                    thread::sleep(Duration::from_secs(1));
                }
            }
            control.live.store(false, Ordering::Relaxed);
            if !poll(control, &send, || adb_screencap(&adb, id), Some(GRPC_RETRY)) {
                return;
            }
        },
        Source::Adb { adb } => {
            poll(control, &send, || adb_screencap(&adb, id), None);
        }
        Source::Simctl => {
            poll(control, &send, || simctl_screenshot(id), None);
        }
    }
}

/// Block while the tile is hidden. Returns false if the worker should exit.
fn wait_until_active(control: &Control) -> bool {
    while !control.active.load(Ordering::Relaxed) {
        if control.stop.load(Ordering::Relaxed) {
            return false;
        }
        thread::sleep(Duration::from_millis(150));
    }
    !control.stop.load(Ordering::Relaxed)
}

/// Poll PNG screenshots. Returns true when `limit` ran out, false when the
/// worker should exit.
fn poll(
    control: &Control,
    send: &impl Fn(Frame),
    capture: impl Fn() -> Option<Vec<u8>>,
    limit: Option<Duration>,
) -> bool {
    let gap = Duration::from_secs(1) / PNG_FPS;
    let begun = Instant::now();
    while wait_until_active(control) {
        if limit.is_some_and(|l| begun.elapsed() >= l) {
            return true;
        }
        let started = Instant::now();
        match capture().and_then(|b| png::decode(&b)) {
            Some(f) => send(png::downscale(f, control.size.load(Ordering::Relaxed))),
            None => thread::sleep(Duration::from_secs(2)),
        }
        thread::sleep(gap.saturating_sub(started.elapsed()));
    }
    false
}

fn adb_screencap(adb: &std::path::Path, serial: &str) -> Option<Vec<u8>> {
    let out = sdk::command(adb)
        .args(["-s", serial, "exec-out", "screencap", "-p"])
        .output()
        .ok()?;
    (out.status.success() && !out.stdout.is_empty()).then_some(out.stdout)
}

fn simctl_screenshot(udid: &str) -> Option<Vec<u8>> {
    let path = std::env::temp_dir().join(format!("emugrid-{udid}.png"));
    let ok = sdk::command("xcrun")
        .args(["simctl", "io", udid, "screenshot", "--type=png"])
        .arg(&path)
        .output()
        .ok()?
        .status
        .success();
    let bytes = ok.then(|| std::fs::read(&path).ok()).flatten();
    let _ = std::fs::remove_file(&path);
    bytes
}

#[cfg(test)]
mod tests {
    use super::*;

    fn frame(n: u8) -> Frame {
        Frame {
            width: 1,
            height: 1,
            rgb: vec![n; 3],
        }
    }

    /// A UI that stops reading (hidden window) must not let frames pile up.
    #[test]
    fn only_the_latest_frame_is_kept() {
        let c = Control::new(320);
        for n in 0..100 {
            c.publish(frame(n));
        }
        assert_eq!(c.frames.load(Ordering::Relaxed), 100);
        assert_eq!(c.take_frame().map(|f| f.rgb[0]), Some(99));
        assert!(c.take_frame().is_none());
    }
}
