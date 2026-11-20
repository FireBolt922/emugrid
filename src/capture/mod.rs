//! Screen capture backends. Each running device gets one worker thread that
//! pushes `Frame`s; the UI only ever sees the latest frame per device.

mod adb_frames;
pub mod grpc;
pub mod png;
pub mod proto;

use crate::sdk;
use eframe::egui;
use std::collections::HashMap;
use std::path::PathBuf;
use std::sync::atomic::{AtomicBool, AtomicU32, AtomicU64, Ordering};
use std::sync::{Arc, Mutex, OnceLock};
use std::thread;
use std::time::{Duration, Instant};

/// Frame rate caps. gRPC frames are pushed on change; PNG is polled.
pub const TILE_FPS: u32 = 15;
pub const FOCUSED_FPS: u32 = 30;
/// Zero dimensions ask the emulator gRPC API for actual display pixels.
pub const NATIVE_SIZE: u32 = 0;
const PNG_FPS: u32 = 2;
const INTERACTIVE_PNG_FPS: u32 = 10;
const INPUT_PREVIEW: Duration = Duration::from_millis(450);

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
    /// Longest requested edge in pixels, or NATIVE_SIZE.
    pub size: AtomicU32,
    pub max_fps: AtomicU32,
    /// True while frames come from the gRPC stream (vs. PNG polling).
    pub live: AtomicBool,
    /// True while the focused Android device streams scaled frames over ADB.
    pub adb_live: AtomicBool,
    /// Frames delivered so far.
    pub frames: AtomicU64,
    native_size: AtomicU64,
    worker: OnceLock<thread::Thread>,
    refresh: AtomicBool,
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
            adb_live: AtomicBool::new(false),
            frames: AtomicU64::new(0),
            native_size: AtomicU64::new(0),
            worker: OnceLock::new(),
            refresh: AtomicBool::new(false),
            latest: Mutex::new(None),
        }
    }

    pub fn set_active(&self, active: bool) {
        if self.active.swap(active, Ordering::AcqRel) != active {
            self.wake();
        }
    }

    pub fn refresh(&self) {
        self.refresh.store(true, Ordering::Release);
        self.wake();
    }

    pub fn set_focused(&self, focused: bool) {
        let fps = if focused { FOCUSED_FPS } else { TILE_FPS };
        if self.max_fps.swap(fps, Ordering::AcqRel) != fps {
            self.refresh();
        }
    }

    pub fn focused(&self) -> bool {
        self.max_fps.load(Ordering::Acquire) == FOCUSED_FPS
    }

    pub fn is_live(&self) -> bool {
        self.live.load(Ordering::Relaxed) || self.adb_live.load(Ordering::Relaxed)
    }

    pub fn native_size(&self) -> Option<(u32, u32)> {
        let value = self.native_size.load(Ordering::Relaxed);
        (value != 0).then_some(((value >> 32) as u32, value as u32))
    }

    fn record_size(&self, frame: &Frame) {
        self.native_size.store(
            u64::from(frame.width) << 32 | u64::from(frame.height),
            Ordering::Relaxed,
        );
    }

    pub fn request_size(&self, size: u32) {
        if self.size.swap(size, Ordering::Relaxed) != size {
            self.refresh();
        }
    }

    fn wake(&self) {
        if let Some(worker) = self.worker.get() {
            worker.unpark();
        }
    }

    fn stop(&self) {
        self.stop.store(true, Ordering::Release);
        self.wake();
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
#[derive(Debug, Clone, PartialEq)]
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
    workers: HashMap<String, (Source, Arc<Control>)>,
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
        self.workers.retain(|id, (source, c)| {
            let alive = wanted
                .iter()
                .any(|(wanted_id, wanted_source)| wanted_id == id && wanted_source == source);
            if !alive {
                c.stop();
            }
            alive
        });
        for (id, source) in wanted {
            if self.workers.contains_key(&id) {
                continue;
            }
            let control = Arc::new(Control::new(320));
            control.active.store(false, Ordering::Relaxed);
            self.workers
                .insert(id.clone(), (source.clone(), control.clone()));
            let ctx = self.ctx.clone();
            thread::spawn(move || {
                let _ = control.worker.set(thread::current());
                let send = |f: Frame| {
                    if control.active.load(Ordering::Acquire)
                        && !control.stop.load(Ordering::Acquire)
                    {
                        control.publish(f);
                        ctx.request_repaint();
                    }
                };
                run(&id, source, &control, send);
            });
        }
    }

    pub fn control(&self, id: &str) -> Option<&Arc<Control>> {
        self.workers.get(id).map(|(_, control)| control)
    }

    pub fn controls(&self) -> impl Iterator<Item = (&String, &Arc<Control>)> {
        self.workers.iter().map(|(id, (_, control))| (id, control))
    }

    /// The newest frame per device that arrived since the last call.
    pub fn frames(&self) -> impl Iterator<Item = (String, Frame)> + '_ {
        self.controls()
            .filter_map(|(id, c)| Some((id.clone(), c.take_frame()?)))
    }
}

impl Drop for Manager {
    fn drop(&mut self) {
        for (_, c) in self.workers.values() {
            c.stop();
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
            let mut failed_size = None;
            poll(
                control,
                &send,
                || {
                    let size = control.size.load(Ordering::Relaxed);
                    if !control.focused() || size == NATIVE_SIZE {
                        failed_size = None;
                    } else if failed_size != Some(size) {
                        failed_size = adb_frames::stream(&adb, id, control, &send).err();
                    }
                    (control.active.load(Ordering::Acquire)
                        && !control.stop.load(Ordering::Acquire))
                    .then(|| adb_screencap(&adb, id))
                    .flatten()
                },
                None,
            );
        }
        Source::Simctl => {
            poll(control, &send, || simctl_screenshot(id), None);
        }
    }
}

/// Block while the tile is hidden. Returns false if the worker should exit.
fn wait_until_active(control: &Control) -> bool {
    while !control.active.load(Ordering::Acquire) {
        if control.stop.load(Ordering::Acquire) {
            return false;
        }
        thread::park();
    }
    !control.stop.load(Ordering::Acquire)
}

/// Poll PNG screenshots. Returns true when `limit` ran out, false when the
/// worker should exit.
fn poll(
    control: &Control,
    send: &impl Fn(Frame),
    mut capture: impl FnMut() -> Option<Vec<u8>>,
    limit: Option<Duration>,
) -> bool {
    let begun = Instant::now();
    let mut boost_until = begun;
    let mut last_capture = None;
    let mut retry_after = begun;
    while wait_until_active(control) {
        let now = Instant::now();
        if limit.is_some_and(|l| now.duration_since(begun) >= l) {
            return true;
        }
        if control.refresh.swap(false, Ordering::AcqRel) {
            boost_until = now + INPUT_PREVIEW;
        }
        let fps = if now < boost_until && control.focused() {
            INTERACTIVE_PNG_FPS
        } else {
            PNG_FPS
        };
        let gap = Duration::from_secs(1) / fps;
        let due = last_capture.map_or(now, |last| last + gap).max(retry_after);
        if due > now {
            thread::park_timeout(due - now);
            continue;
        }
        last_capture = Some(now);
        let bytes = capture();
        if !control.active.load(Ordering::Acquire) || control.stop.load(Ordering::Acquire) {
            continue;
        }
        match bytes.and_then(|b| png::decode(&b)) {
            Some(f) => {
                control.record_size(&f);
                let size = control.size.load(Ordering::Relaxed);
                send(if size == NATIVE_SIZE {
                    f
                } else {
                    png::downscale(f, size)
                });
            }
            None => retry_after = Instant::now() + Duration::from_secs(2),
        }
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

    #[test]
    fn focused_png_capture_wakes_before_the_idle_poll() {
        let control = Control::new(320);
        let (tx, rx) = std::sync::mpsc::channel();
        let mut png = Vec::new();
        {
            let mut encoder = ::png::Encoder::new(&mut png, 1, 1);
            encoder.set_color(::png::ColorType::Rgb);
            encoder
                .write_header()
                .unwrap()
                .write_image_data(&[1, 2, 3])
                .unwrap();
        }
        thread::scope(|scope| {
            scope.spawn(|| {
                control.worker.set(thread::current()).unwrap();
                poll(
                    &control,
                    &|_| {
                        let _ = tx.send(Instant::now());
                    },
                    || Some(png.clone()),
                    None,
                );
            });
            let first = rx.recv_timeout(Duration::from_secs(2));
            control.set_focused(true);
            control.refresh();
            let next = rx.recv_timeout(Duration::from_secs(2));
            control.stop();
            assert!(
                next.unwrap().duration_since(first.unwrap()) < Duration::from_secs(1) / PNG_FPS
            );
        });
    }
}
