//! Direct touch for Android: a virtual touchscreen made with the device's own
//! `uinput` tool (Android 11+, nothing to install). One long-lived
//! `adb shell uinput -` takes JSON lines on stdin, so a press, every move and
//! the release reach Android as real touch events with no per-event process
//! start (the old `input tap` path costs ~60-100 ms per event).

use crate::sdk;
use std::io::Write;
use std::path::Path;
use std::process::{Child, Stdio};
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, AtomicU8, Ordering};
use std::thread;
use std::time::{Duration, Instant};

const NAME: &str = "emugrid-touch";
/// Raw axis range of the virtual touchscreen; Android scales it to the display.
const MAX: i32 = 32767;

const EV_SYN: i32 = 0;
const EV_KEY: i32 = 1;
const EV_ABS: i32 = 3;
const BTN_TOUCH: i32 = 0x14a;
const ABS_MT_SLOT: i32 = 0x2f;
const ABS_MT_TOUCH_MAJOR: i32 = 0x30;
const ABS_MT_POSITION_X: i32 = 0x35;
const ABS_MT_POSITION_Y: i32 = 0x36;
const ABS_MT_TRACKING_ID: i32 = 0x39;
const ABS_MT_PRESSURE: i32 = 0x3a;
const INPUT_PROP_DIRECT: i32 = 1;

/// One touch step, in the shown screen's own coordinates (0..1, 0..1).
#[derive(Debug, Clone, Copy, PartialEq)]
pub enum Touch {
    Down(f32, f32),
    Move(f32, f32),
    Up,
}

/// The `register` command that creates the virtual touchscreen.
pub fn register_json() -> String {
    let abs = |code: i32, max: i32| {
        format!(
            r#"{{"code":{code},"info":{{"value":0,"minimum":0,"maximum":{max},"fuzz":0,"flat":0,"resolution":0}}}}"#
        )
    };
    format!(
        r#"{{"id":1,"command":"register","name":"{NAME}","vid":6353,"pid":20199,"bus":"usb","configuration":[{{"type":"UI_SET_EVBIT","data":[{EV_KEY},{EV_ABS}]}},{{"type":"UI_SET_KEYBIT","data":[{BTN_TOUCH}]}},{{"type":"UI_SET_ABSBIT","data":[{ABS_MT_SLOT},{ABS_MT_TOUCH_MAJOR},{ABS_MT_POSITION_X},{ABS_MT_POSITION_Y},{ABS_MT_TRACKING_ID},{ABS_MT_PRESSURE}]}},{{"type":"UI_SET_PROPBIT","data":[{INPUT_PROP_DIRECT}]}}],"abs_info":[{},{},{},{},{},{}]}}"#,
        abs(ABS_MT_SLOT, 9),
        abs(ABS_MT_TOUCH_MAJOR, 255),
        abs(ABS_MT_POSITION_X, MAX),
        abs(ABS_MT_POSITION_Y, MAX),
        abs(ABS_MT_TRACKING_ID, 65535),
        abs(ABS_MT_PRESSURE, 255),
    )
}

/// Raw touchscreen axes are in the device's natural orientation; Android
/// rotates them by the display rotation. Undo that (the inverse of
/// TouchInputMapper's rotation) so a point lands where it was shown.
pub fn to_raw(u: f32, v: f32, rotation: u8) -> (i32, i32) {
    let (u, v) = (u.clamp(0.0, 1.0), v.clamp(0.0, 1.0));
    let (x, y) = match rotation % 4 {
        0 => (u, v),
        1 => (1.0 - v, u),
        2 => (1.0 - u, 1.0 - v),
        _ => (v, 1.0 - u),
    };
    let axis = |t: f32| (t * MAX as f32).round() as i32;
    (axis(x), axis(y))
}

/// The `inject` command for one touch step.
pub fn inject_json(touch: Touch, rotation: u8) -> String {
    let events: Vec<i32> = match touch {
        Touch::Down(u, v) => {
            let (x, y) = to_raw(u, v, rotation);
            vec![
                EV_ABS,
                ABS_MT_SLOT,
                0,
                EV_ABS,
                ABS_MT_TRACKING_ID,
                1,
                EV_ABS,
                ABS_MT_POSITION_X,
                x,
                EV_ABS,
                ABS_MT_POSITION_Y,
                y,
                EV_ABS,
                ABS_MT_PRESSURE,
                128,
                EV_ABS,
                ABS_MT_TOUCH_MAJOR,
                6,
                EV_KEY,
                BTN_TOUCH,
                1,
                EV_SYN,
                0,
                0,
            ]
        }
        Touch::Move(u, v) => {
            let (x, y) = to_raw(u, v, rotation);
            vec![
                EV_ABS,
                ABS_MT_POSITION_X,
                x,
                EV_ABS,
                ABS_MT_POSITION_Y,
                y,
                EV_SYN,
                0,
                0,
            ]
        }
        Touch::Up => vec![
            EV_ABS,
            ABS_MT_TRACKING_ID,
            -1,
            EV_KEY,
            BTN_TOUCH,
            0,
            EV_SYN,
            0,
            0,
        ],
    };
    let list: Vec<String> = events.iter().map(i32::to_string).collect();
    format!(
        r#"{{"id":1,"command":"inject","events":[{}]}}"#,
        list.join(",")
    )
}

/// Display rotation (0-3) from `dumpsys window displays`.
pub fn parse_rotation(out: &str) -> Option<u8> {
    let at = out.find("mDisplayRotation=ROTATION_")? + "mDisplayRotation=ROTATION_".len();
    let digits: String = out[at..].chars().take_while(char::is_ascii_digit).collect();
    match digits.as_str() {
        "0" => Some(0),
        "90" => Some(1),
        "180" => Some(2),
        "270" => Some(3),
        _ => None,
    }
}

/// A live virtual touchscreen on one device.
pub struct Finger {
    child: Option<Child>,
    stdin: Option<Box<dyn Write + Send>>,
    rotation: Arc<AtomicU8>,
    alive: Arc<AtomicBool>,
}

impl Finger {
    /// Create the touchscreen. Blocks for up to a few seconds; call it off
    /// the UI thread. Fails on devices without `uinput` access.
    pub fn start(adb: &Path, serial: &str) -> Result<Finger, String> {
        let mut child = sdk::command(adb)
            .args(["-s", serial, "shell", "uinput", "-"])
            .stdin(Stdio::piped())
            .stdout(Stdio::null())
            .stderr(Stdio::null())
            .spawn()
            .map_err(|e| e.to_string())?;
        let mut stdin = child.stdin.take().ok_or("no stdin")?;
        let registered = writeln!(stdin, "{}", register_json()).and_then(|_| stdin.flush());
        let ready = registered.is_ok() && wait_for_device(adb, serial, &mut child);
        if !ready {
            drop(stdin);
            let _ = child.kill();
            let _ = child.wait();
            return Err("uinput not available".into());
        }

        let rotation = Arc::new(AtomicU8::new(read_rotation(adb, serial).unwrap_or(0)));
        let alive = Arc::new(AtomicBool::new(true));
        // The display can rotate at any time; keep the rotation fresh.
        let (adb, serial, rot, live) = (
            adb.to_path_buf(),
            serial.to_string(),
            rotation.clone(),
            alive.clone(),
        );
        thread::spawn(move || {
            while live.load(Ordering::Relaxed) {
                thread::sleep(Duration::from_millis(1500));
                if let Some(r) = read_rotation(&adb, &serial) {
                    rot.store(r, Ordering::Relaxed);
                }
            }
        });
        Ok(Finger {
            child: Some(child),
            stdin: Some(Box::new(stdin)),
            rotation,
            alive,
        })
    }

    /// A finger that writes its commands to `out` (tests).
    #[cfg(test)]
    pub fn to_writer(out: impl Write + Send + 'static) -> Finger {
        Finger {
            child: None,
            stdin: Some(Box::new(out)),
            rotation: Arc::new(AtomicU8::new(0)),
            alive: Arc::new(AtomicBool::new(false)),
        }
    }

    /// Send one touch step. Returns false once the touchscreen is gone.
    pub fn send(&mut self, touch: Touch) -> bool {
        let line = inject_json(touch, self.rotation.load(Ordering::Relaxed));
        let ok = self
            .stdin
            .as_mut()
            .is_some_and(|s| writeln!(s, "{line}").and_then(|_| s.flush()).is_ok());
        if !ok {
            self.stdin = None;
        }
        ok
    }
}

impl Drop for Finger {
    fn drop(&mut self) {
        self.alive.store(false, Ordering::Relaxed);
        // EOF makes `uinput` exit, which removes the touchscreen.
        self.stdin = None;
        if let Some(child) = self.child.as_mut() {
            let _ = child.kill();
            let _ = child.wait();
        }
    }
}

/// Wait until Android lists the new touchscreen.
fn wait_for_device(adb: &Path, serial: &str, child: &mut Child) -> bool {
    let begun = Instant::now();
    while begun.elapsed() < Duration::from_secs(4) {
        thread::sleep(Duration::from_millis(250));
        if child.try_wait().ok().flatten().is_some() {
            return false;
        }
        let listed = sdk::command(adb)
            .args(["-s", serial, "shell", "getevent", "-pl"])
            .output()
            .is_ok_and(|o| String::from_utf8_lossy(&o.stdout).contains(NAME));
        if listed {
            return true;
        }
    }
    false
}

fn read_rotation(adb: &Path, serial: &str) -> Option<u8> {
    let out = sdk::command(adb)
        .args(["-s", serial, "shell", "dumpsys window displays"])
        .output()
        .ok()?;
    parse_rotation(&String::from_utf8_lossy(&out.stdout))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn rotation_is_undone() {
        // A point a quarter across and a tenth down, as shown.
        let (u, v) = (0.25, 0.1);
        let m = MAX as f32;
        let r = |t: f32| (t * m).round() as i32;
        assert_eq!(to_raw(u, v, 0), (r(0.25), r(0.1)));
        assert_eq!(to_raw(u, v, 1), (r(0.9), r(0.25)));
        assert_eq!(to_raw(u, v, 2), (r(0.75), r(0.9)));
        assert_eq!(to_raw(u, v, 3), (r(0.1), r(0.75)));
        assert_eq!(to_raw(-1.0, 2.0, 0), (0, MAX));
    }

    /// Checked live: rotation 90, tapping (640, 660) of a 2400x1080 screen
    /// opened the item drawn there.
    #[test]
    fn landscape_tap_matches_live_check() {
        let (x, y) = to_raw(640.0 / 2399.0, 660.0 / 1079.0, 1);
        // The live check sent (12724, 8741).
        assert!((x - 12724).abs() <= 1 && (y - 8741).abs() <= 1, "{x} {y}");
    }

    #[test]
    fn parses_rotation() {
        assert_eq!(
            parse_rotation("x mDisplayRotation=ROTATION_90 mWindowingMode"),
            Some(1)
        );
        assert_eq!(parse_rotation("mDisplayRotation=ROTATION_0}"), Some(0));
        assert_eq!(parse_rotation("mDisplayRotation=undefined"), None);
    }

    #[test]
    fn down_move_up_events() {
        let down = inject_json(Touch::Down(0.0, 1.0), 0);
        assert!(down.starts_with(
            r#"{"id":1,"command":"inject","events":[3,47,0,3,57,1,3,53,0,3,54,32767,"#
        ));
        assert!(down.ends_with("1,330,1,0,0,0]}"));
        assert_eq!(
            inject_json(Touch::Move(0.5, 0.5), 0),
            r#"{"id":1,"command":"inject","events":[3,53,16384,3,54,16384,0,0,0]}"#
        );
        assert_eq!(
            inject_json(Touch::Up, 0),
            r#"{"id":1,"command":"inject","events":[3,57,-1,1,330,0,0,0,0]}"#
        );
    }

    /// Each step is one `inject` line; a closed pipe reports the finger gone.
    #[test]
    fn finger_writes_one_line_per_step() {
        use std::sync::Mutex;
        #[derive(Clone, Default)]
        struct Sink(Arc<Mutex<Vec<u8>>>);
        impl Write for Sink {
            fn write(&mut self, b: &[u8]) -> std::io::Result<usize> {
                self.0.lock().unwrap().extend_from_slice(b);
                Ok(b.len())
            }
            fn flush(&mut self) -> std::io::Result<()> {
                Ok(())
            }
        }
        let sink = Sink::default();
        let mut f = Finger::to_writer(sink.clone());
        assert!(f.send(Touch::Down(0.5, 0.5)));
        assert!(f.send(Touch::Up));
        let out = String::from_utf8(sink.0.lock().unwrap().clone()).unwrap();
        let lines: Vec<_> = out.lines().collect();
        assert_eq!(lines.len(), 2);
        assert_eq!(lines[1], inject_json(Touch::Up, 0));

        struct Closed;
        impl Write for Closed {
            fn write(&mut self, _: &[u8]) -> std::io::Result<usize> {
                Err(std::io::ErrorKind::BrokenPipe.into())
            }
            fn flush(&mut self) -> std::io::Result<()> {
                Ok(())
            }
        }
        let mut gone = Finger::to_writer(Closed);
        assert!(!gone.send(Touch::Up));
        assert!(!gone.send(Touch::Up), "stays gone");
    }

    #[test]
    fn register_describes_a_direct_touchscreen() {
        let r = register_json();
        assert!(r.contains(r#""name":"emugrid-touch""#));
        assert!(r.contains(r#"{"type":"UI_SET_PROPBIT","data":[1]}"#));
        assert!(r.contains(r#"{"code":53,"info":{"value":0,"minimum":0,"maximum":32767"#));
    }

    /// Live: needs a running emulator (`EMUGRID_SERIAL`, default emulator-5554).
    #[test]
    #[ignore]
    fn live_finger_registers() {
        let serial = std::env::var("EMUGRID_SERIAL").unwrap_or("emulator-5554".into());
        let adb = sdk::Sdk::detect(None).adb.expect("adb");
        let mut f = Finger::start(&adb, &serial).expect("touchscreen");
        // Swipe up from the lower middle, like a finger (opens the app
        // drawer on a home screen).
        assert!(f.send(Touch::Down(0.5, 0.8)));
        for i in 1..=10 {
            thread::sleep(Duration::from_millis(16));
            assert!(f.send(Touch::Move(0.5, 0.8 - 0.05 * i as f32)));
        }
        assert!(f.send(Touch::Up));
        thread::sleep(Duration::from_millis(300));
    }
}
