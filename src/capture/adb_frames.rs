//! Scaled RGB frames from Android's built-in screen recorder. Only the focused
//! device uses this stream, since the frames are uncompressed over ADB.

use super::{Control, Frame};
use crate::sdk;
use std::io::{self, BufReader, Read};
use std::path::Path;
use std::process::{Child, Stdio};
use std::sync::atomic::Ordering;
use std::sync::mpsc::{self, RecvTimeoutError};
use std::sync::{Arc, Mutex};
use std::thread;
use std::time::{Duration, Instant};

const STARTUP_TIMEOUT: Duration = Duration::from_secs(2);
const CHECK_FOCUS: Duration = Duration::from_millis(70);
const CHECK_DISPLAY: Duration = Duration::from_secs(1);

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
struct Display {
    rotation: u8,
    width: u32,
    height: u32,
}

impl Display {
    fn parse(text: &str) -> Option<Self> {
        let line = text.lines().find(|s| {
            s.contains("Viewport ") && s.contains("displayId=0,") && s.contains("logicalFrame=[")
        })?;
        let rotation = line
            .split_once("orientation=")?
            .1
            .split_once(',')?
            .0
            .parse()
            .ok()?;
        let bounds = line.split_once("logicalFrame=[")?.1.split_once(']')?.0;
        let mut bounds = bounds.split(',').map(|s| s.trim().parse::<u32>().ok());
        let (left, top, right, bottom) = (
            bounds.next()??,
            bounds.next()??,
            bounds.next()??,
            bounds.next()??,
        );
        let (width, height) = (right.checked_sub(left)?, bottom.checked_sub(top)?);
        (rotation <= 3 && width > 0 && height > 0).then_some(Self {
            rotation,
            width,
            height,
        })
    }

    fn stream_size(self, requested_edge: u32) -> (u32, u32) {
        let native_edge = self.width.max(self.height);
        let edge = requested_edge.min(native_edge);
        // Keep the aspect ratio and use even dimensions for device compatibility.
        let scaled =
            |n| ((u64::from(n) * u64::from(edge) / u64::from(native_edge)) as u32 & !1).max(2);
        (scaled(self.width), scaled(self.height))
    }
}

fn read_display(adb: &Path, serial: &str) -> Option<Display> {
    let output = sdk::command(adb)
        .args([
            "-s",
            serial,
            "shell",
            "-T",
            "dumpsys -t 1 input | grep -m 1 'displayId=0,.*orientation=.*logicalFrame='",
        ])
        .output()
        .ok()?;
    Display::parse(std::str::from_utf8(&output.stdout).ok()?)
}

/// `screenrecord --output-format=frames` prefixes RGB24 with a little-endian
/// packet length (excluding itself), width, height, stride, and bytes/pixel.
fn read_frame(reader: &mut impl Read, size: (u32, u32)) -> io::Result<Frame> {
    let mut header = [0; 20];
    reader.read_exact(&mut header)?;
    let [length, width, height, stride, bpp] =
        std::array::from_fn(|i| u32::from_le_bytes(header[i * 4..i * 4 + 4].try_into().unwrap()));
    let invalid = || io::Error::new(io::ErrorKind::InvalidData, "invalid ADB frame header");
    if (width, height) != size
        || bpp != 3
        || stride != width * bpp
        || length != 16 + stride * height
    {
        return Err(invalid());
    }
    let mut rgb = vec![0; (stride * height) as usize];
    reader.read_exact(&mut rgb)?;
    Ok(Frame { width, height, rgb })
}

struct Recorder(Child);

impl Drop for Recorder {
    fn drop(&mut self) {
        let _ = self.0.kill();
        let _ = self.0.wait();
    }
}

/// Returns the failed size request if this device needs PNG capture at that size.
pub fn stream(
    adb: &Path,
    serial: &str,
    control: &Control,
    send: &impl Fn(Frame),
) -> Result<(), u32> {
    let fit_request = || {
        let edge = control.size.load(Ordering::Relaxed);
        (control.active.load(Ordering::Acquire)
            && !control.stop.load(Ordering::Acquire)
            && control.focused()
            && edge != super::NATIVE_SIZE)
            .then_some(edge)
    };
    let Some(initial_size) = fit_request() else {
        return Ok(());
    };
    let mut display = read_display(adb, serial).ok_or(initial_size)?;
    while let Some(requested_edge) = fit_request() {
        let size = display.stream_size(requested_edge);
        let size_arg = format!("{}x{}", size.0, size.1);
        let Ok(child) = sdk::command(adb)
            .args([
                "-s",
                serial,
                "shell",
                "-T",
                "screenrecord",
                "--output-format=frames",
                "--size",
                &size_arg,
                "-",
            ])
            .stdout(Stdio::piped())
            .stderr(Stdio::null())
            .spawn()
        else {
            return Err(requested_edge);
        };
        let mut recorder = Recorder(child);
        let mut stdout = BufReader::new(recorder.0.stdout.take().unwrap());
        let (tx, rx) = mpsc::sync_channel(1);
        let latest = Arc::new(Mutex::new(None));
        let incoming = latest.clone();
        thread::spawn(move || {
            while let Ok(frame) = read_frame(&mut stdout, size) {
                *incoming.lock().unwrap() = Some(frame);
                match tx.try_send(()) {
                    Ok(()) | Err(mpsc::TrySendError::Full(_)) => {}
                    Err(mpsc::TrySendError::Disconnected(_)) => break,
                }
            }
        });

        let start = Instant::now();
        let mut next_check = start + CHECK_DISPLAY;
        let mut last_sent = None;
        let mut pending = None;
        let min_gap = Duration::from_secs(1) / super::FOCUSED_FPS;
        let mut failed = false;
        while fit_request().is_some_and(|edge| display.stream_size(edge) == size) {
            let now = Instant::now();
            if now >= next_check {
                match read_display(adb, serial) {
                    Some(current) if current != display => {
                        display = current;
                        break;
                    }
                    Some(_) => next_check = Instant::now() + CHECK_DISPLAY,
                    None => {
                        failed = true;
                        break;
                    }
                }
            }
            let until_frame = if pending.is_some() {
                last_sent.map_or(Duration::ZERO, |last: Instant| {
                    min_gap.saturating_sub(last.elapsed())
                })
            } else {
                CHECK_FOCUS
            };
            match rx.recv_timeout(until_frame) {
                Ok(()) => {
                    if let Some(frame) = latest.lock().unwrap().take() {
                        pending = Some(frame);
                    }
                }
                Err(RecvTimeoutError::Disconnected) => {
                    failed = true;
                    break;
                }
                Err(RecvTimeoutError::Timeout) => {}
            }
            if !fit_request().is_some_and(|edge| display.stream_size(edge) == size) {
                break;
            }
            if last_sent.is_none_or(|last: Instant| last.elapsed() >= min_gap)
                && let Some(frame) = pending.take()
            {
                last_sent = Some(Instant::now());
                control.adb_live.store(true, Ordering::Relaxed);
                send(frame);
            }
            // Static screens can go quiet once they have delivered a frame.
            if last_sent.is_none() && start.elapsed() >= STARTUP_TIMEOUT {
                failed = true;
                break;
            }
        }
        control.adb_live.store(false, Ordering::Relaxed);
        if failed
            && let Some(edge) = fit_request().filter(|edge| display.stream_size(*edge) == size)
        {
            return Err(edge);
        }
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn sizes_streams_from_the_current_display_orientation() {
        let viewport = |rotation: u8, width: u32, height: u32| {
            format!(
                "Viewport INTERNAL: displayId=0, uniqueId=local:123, orientation={rotation}, logicalFrame=[0, 0, {width}, {height}], isActive=[1]"
            )
        };
        let portrait = Display::parse(&viewport(0, 1080, 2400)).unwrap();
        let landscape = Display::parse(&viewport(1, 2400, 1080)).unwrap();
        assert_eq!(portrait.stream_size(800), (360, 800));
        assert_eq!(landscape.stream_size(960), (960, 432));
        assert_eq!(portrait.stream_size(2500), (1080, 2400));
        assert_eq!(portrait.stream_size(801), (360, 800));
        assert_ne!(portrait, Display::parse(&viewport(2, 1080, 2400)).unwrap());
        assert!(Display::parse("External viewport for another display").is_none());
    }

    #[test]
    fn reads_rgb_frames_and_rejects_bad_sizes_before_allocating() {
        let mut bytes: Vec<u8> = [28u32, 2, 2, 6, 3]
            .into_iter()
            .flat_map(u32::to_le_bytes)
            .collect();
        bytes.extend([7; 12]);
        let f = read_frame(&mut bytes.as_slice(), (2, 2)).unwrap();
        assert_eq!(f.rgb, [7; 12]);
        bytes[..4].copy_from_slice(&u32::MAX.to_le_bytes());
        assert!(read_frame(&mut bytes.as_slice(), (2, 2)).is_err());
    }

    #[test]
    #[ignore = "needs a running Android device from EMUGRID_TEST_SERIAL"]
    fn streams_requested_sizes_and_stops_when_focus_leaves() {
        let serial = std::env::var("EMUGRID_TEST_SERIAL").unwrap();
        let adb = sdk::Sdk::detect(None).adb.unwrap();
        let display = read_display(&adb, &serial).unwrap();
        assert!(
            display.width.max(display.height) >= 960,
            "test needs a display at least 960 px long"
        );
        let control = Control::new(800);
        control.set_focused(true);
        let (tx, rx) = mpsc::sync_channel(1);
        let (first, resized) = thread::scope(|scope| {
            let worker = scope.spawn(|| {
                stream(&adb, &serial, &control, &|frame| {
                    let _ = tx.try_send(frame);
                })
            });
            let first = rx.recv_timeout(Duration::from_secs(5));
            control.request_size(960);
            let deadline = Instant::now() + Duration::from_secs(5);
            let resized = loop {
                match rx.recv_timeout(deadline.saturating_duration_since(Instant::now())) {
                    Ok(frame)
                        if frame.width.max(frame.height) != 960 && Instant::now() < deadline =>
                    {
                        continue;
                    }
                    result => break result,
                }
            };
            control.set_focused(false);
            assert_eq!(worker.join().unwrap(), Ok(()));
            (first, resized)
        });
        for (frame, edge) in [(first, 800), (resized, 960)] {
            let frame = frame.unwrap();
            assert_eq!(frame.width.max(frame.height), edge);
            assert_eq!(frame.rgb.len(), (frame.width * frame.height * 3) as usize);
        }
    }
}
