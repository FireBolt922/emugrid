//! One ordered input session per Android device, independent of screen capture.
//!
//! Touches go to a virtual touchscreen (see [`crate::touch`]) when the device
//! allows one; everything else, and touches on older devices, runs as
//! `input ...` commands in one long-lived `adb shell`.

use crate::touch::{Finger, Touch};
use crate::{capture::Control, sdk};
use std::collections::VecDeque;
use std::io::{self, BufRead, BufReader, Write};
use std::path::{Path, PathBuf};
use std::process::{Child, Command as Process, Stdio};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::mpsc::{self, Receiver, Sender};
use std::sync::{Arc, Mutex};
use std::thread;
use std::time::{Duration, Instant};

const TIMEOUT: Duration = Duration::from_secs(4);

pub enum Command {
    Shell(String),
    /// Touch steps for the virtual touchscreen, and the `input` command to
    /// run instead when the device has no virtual touchscreen.
    Touch(Vec<Touch>, String),
    Scroll {
        x: u32,
        y: u32,
        distance: i32,
        limit: i32,
    },
}

impl Command {
    /// Replace stale pointer locations and combine adjacent wheel input. Leave
    /// presses, releases, keys and text in the order the user sent them.
    fn absorb(&mut self, previous: &Self) -> bool {
        match (self, previous) {
            (Self::Shell(current), Self::Shell(previous)) => {
                let is_move = |s: &str| s.starts_with("input motionevent MOVE ");
                is_move(current) && is_move(previous)
            }
            (Self::Touch(current, _), Self::Touch(previous, _)) => {
                let is_move = |t: &[Touch]| matches!(t, [Touch::Move(..)]);
                is_move(current) && is_move(previous)
            }
            (
                Self::Scroll {
                    x,
                    y,
                    distance,
                    limit,
                },
                Self::Scroll {
                    x: old_x,
                    y: old_y,
                    distance: old,
                    ..
                },
            ) if x == old_x && y == old_y => {
                *distance = distance.saturating_add(*old).clamp(-*limit, *limit);
                true
            }
            _ => false,
        }
    }

    fn shell(self) -> Option<String> {
        match self {
            Self::Shell(cmd) | Self::Touch(_, cmd) => Some(cmd),
            Self::Scroll { x, y, distance, .. } if distance != 0 => {
                let half = distance / 2;
                let start = y.saturating_add_signed(-half);
                let end = y.saturating_add_signed(distance - half);
                Some(format!("input swipe {x} {start} {x} {end} 80"))
            }
            Self::Scroll { .. } => None,
        }
    }
}

struct Request {
    command: Command,
    capture: Option<Arc<Control>>,
}

fn drain(queue: &mut VecDeque<Request>, incoming: &Receiver<Request>) {
    for mut request in incoming.try_iter() {
        if queue
            .back()
            .is_some_and(|last| request.command.absorb(&last.command))
        {
            queue.pop_back();
        }
        queue.push_back(request);
    }
}

pub struct Worker {
    tx: Sender<Request>,
    live_touch: Arc<AtomicBool>,
    stopped: Arc<AtomicBool>,
}

impl Worker {
    pub fn start(adb: PathBuf, serial: String, report: impl Fn(String) + Send + 'static) -> Self {
        let (tx, rx) = mpsc::channel();
        let live_touch = Arc::new(AtomicBool::new(false));
        let supported = live_touch.clone();
        let stopped = Arc::new(AtomicBool::new(false));
        let shutdown = stopped.clone();
        // The virtual touchscreen takes a few seconds to appear; set it up
        // beside the shell so taps work (through `input`) in the meantime.
        let finger: Arc<Mutex<Option<Finger>>> = Arc::default();
        start_finger(&adb, &serial, &finger, &live_touch);
        thread::spawn(move || {
            // Prewarm the connection when the user opens the device.
            let mut shell = Shell::connect(&adb, &serial).ok();
            let mut persistent = shell.is_some();
            let mut queue = VecDeque::new();
            loop {
                supported.store(
                    shell.as_ref().is_some_and(|s| s.live_touch)
                        || finger.lock().is_ok_and(|f| f.is_some()),
                    Ordering::Relaxed,
                );
                if queue.is_empty() {
                    let Ok(request) = rx.recv() else { return };
                    queue.push_back(request);
                }
                if shutdown.load(Ordering::Acquire) {
                    return;
                }
                drain(&mut queue, &rx);
                let Request {
                    mut command,
                    capture,
                } = queue.pop_front().unwrap();
                if let Command::Touch(steps, fallback) = command {
                    if let Ok(mut slot) = finger.lock()
                        && let Some(f) = slot.as_mut()
                    {
                        if let Some(c) = &capture {
                            c.refresh();
                        }
                        if steps.iter().all(|&step| f.send(step)) {
                            if let Some(c) = capture {
                                c.refresh();
                            }
                            continue;
                        }
                        // The touchscreen went away (device stopped, adb
                        // restarted): use `input` from now on.
                        *slot = None;
                    }
                    command = Command::Shell(fallback);
                }
                let Some(mut cmd) = command.shell() else {
                    continue;
                };

                if shell.as_mut().is_some_and(|s| !s.is_running()) {
                    shell = None;
                }
                if persistent && shell.is_none() {
                    shell = Shell::connect(&adb, &serial).ok();
                    persistent = shell.is_some();
                }
                if let Some(s) = &shell
                    && !s.cancel_touch
                    && let Some(position) = cmd.strip_prefix("input motionevent CANCEL ")
                {
                    cmd = format!("input motionevent UP {position}");
                }
                if let Some(c) = &capture {
                    c.refresh();
                }
                let result = match shell.as_mut() {
                    Some(s) => s.run(&cmd),
                    None => run_once(&adb, &serial, &cmd),
                };
                match result {
                    Ok(true) => {
                        if let Some(c) = capture {
                            c.refresh();
                        }
                    }
                    Ok(false) => report(format!("Android rejected input on {serial}")),
                    Err(err) => {
                        // The command may already have reached Android. Reopen
                        // the session for the next action without replaying it.
                        shell = None;
                        report(format!("Could not send input to {serial} ({err})"));
                    }
                }
            }
        });
        Self {
            tx,
            live_touch,
            stopped,
        }
    }

    pub fn live_touch(&self) -> bool {
        self.live_touch.load(Ordering::Relaxed)
    }

    pub fn send(&self, command: Command, capture: Option<Arc<Control>>) {
        let _ = self.tx.send(Request { command, capture });
    }
}

/// Create the virtual touchscreen in the background and hand it to the worker.
fn start_finger(
    adb: &Path,
    serial: &str,
    slot: &Arc<Mutex<Option<Finger>>>,
    live: &Arc<AtomicBool>,
) {
    let (adb, serial, slot, live) = (
        adb.to_path_buf(),
        serial.to_string(),
        slot.clone(),
        live.clone(),
    );
    thread::spawn(move || {
        if let Ok(finger) = Finger::start(&adb, &serial)
            && let Ok(mut slot) = slot.lock()
        {
            *slot = Some(finger);
            live.store(true, Ordering::Relaxed);
        }
    });
}

impl Drop for Worker {
    fn drop(&mut self) {
        self.stopped.store(true, Ordering::Release);
    }
}

enum Reply {
    Ready {
        live_touch: bool,
        cancel_touch: bool,
    },
    Done(bool),
}

struct Shell {
    child: Child,
    requests: Sender<String>,
    replies: Receiver<Reply>,
    live_touch: bool,
    cancel_touch: bool,
}

impl Shell {
    fn connect(adb: &Path, serial: &str) -> io::Result<Self> {
        let mut cmd = sdk::command(adb);
        cmd.args(["-s", serial, "shell", "-T", "-e", "none", "sh", "-s"]);
        Self::open(cmd)
    }

    fn open(mut cmd: Process) -> io::Result<Self> {
        let mut child = cmd
            .stdin(Stdio::piped())
            .stdout(Stdio::piped())
            .stderr(Stdio::null())
            .spawn()?;
        let mut stdin = child.stdin.take().unwrap();
        let stdout = child.stdout.take().unwrap();
        let (requests, incoming) = mpsc::channel::<String>();
        let (tx, replies) = mpsc::channel();
        thread::spawn(move || {
            let mut live_touch = false;
            let mut cancel_touch = false;
            let mut lines = BufReader::new(stdout).lines();
            for command in incoming {
                if writeln!(stdin, "{command}").is_err() {
                    return;
                }
                let reply = loop {
                    let Some(Ok(line)) = lines.next() else { return };
                    match line.trim() {
                        "emugrid-ready" => {
                            break Reply::Ready {
                                live_touch,
                                cancel_touch,
                            };
                        }
                        "emugrid-done 0" => break Reply::Done(true),
                        line if line.starts_with("emugrid-done ") => break Reply::Done(false),
                        line if line.starts_with("motionevent ") => {
                            live_touch = line.contains("MOVE");
                            cancel_touch = line.contains("CANCEL");
                        }
                        _ => {}
                    }
                };
                if tx.send(reply).is_err() {
                    break;
                }
            }
        });

        let mut shell = Self {
            child,
            requests,
            replies,
            live_touch: false,
            cancel_touch: false,
        };
        let Reply::Ready {
            live_touch,
            cancel_touch,
        } = shell.exchange("input </dev/null 2>&1\nprintf 'emugrid-ready\\n'".into())?
        else {
            return Err(io::Error::other("unexpected input setup reply"));
        };
        shell.live_touch = live_touch;
        shell.cancel_touch = cancel_touch;
        Ok(shell)
    }

    fn is_running(&mut self) -> bool {
        matches!(self.child.try_wait(), Ok(None))
    }

    fn exchange(&self, command: String) -> io::Result<Reply> {
        self.requests
            .send(command)
            .map_err(|_| io::Error::new(io::ErrorKind::BrokenPipe, "ADB shell closed"))?;
        self.replies.recv_timeout(TIMEOUT).map_err(|err| match err {
            mpsc::RecvTimeoutError::Timeout => {
                io::Error::new(io::ErrorKind::TimedOut, "input timed out")
            }
            mpsc::RecvTimeoutError::Disconnected => {
                io::Error::new(io::ErrorKind::BrokenPipe, "ADB shell closed")
            }
        })
    }

    fn run(&self, command: &str) -> io::Result<bool> {
        if command.contains('\0') {
            return Ok(false);
        }
        let reply = self.exchange(format!(
            "{command} </dev/null >/dev/null 2>&1\nprintf 'emugrid-done %s\\n' \"$?\""
        ))?;
        match reply {
            Reply::Done(success) => Ok(success),
            _ => Err(io::Error::other("unexpected input reply")),
        }
    }
}

impl Drop for Shell {
    fn drop(&mut self) {
        stop_child(&mut self.child);
    }
}

fn run_once(adb: &Path, serial: &str, cmd: &str) -> io::Result<bool> {
    let mut child = sdk::command(adb)
        .args(["-s", serial, "shell", cmd])
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .spawn()?;
    let start = Instant::now();
    loop {
        match child.try_wait() {
            Ok(Some(status)) => return Ok(status.success()),
            Ok(None) if start.elapsed() < TIMEOUT => thread::sleep(Duration::from_millis(5)),
            result => {
                stop_child(&mut child);
                return Err(result.err().unwrap_or_else(|| {
                    io::Error::new(io::ErrorKind::TimedOut, "input timed out")
                }));
            }
        }
    }
}

fn stop_child(child: &mut Child) {
    let _ = child.kill();
    let _ = child.wait();
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn coalesces_moves_and_scroll_only_between_ordered_inputs() {
        let (tx, rx) = mpsc::channel();
        let shell = |cmd: &str| Command::Shell(format!("input {cmd}"));
        let scroll = |distance| Command::Scroll {
            x: 5,
            y: 50,
            distance,
            limit: 40,
        };
        for command in [
            shell("motionevent DOWN 1 1"),
            shell("motionevent MOVE 2 2"),
            shell("motionevent MOVE 3 3"),
            shell("motionevent UP 3 3"),
            scroll(20),
            scroll(30),
            shell("keyevent 4"),
            scroll(-20),
            scroll(20),
        ] {
            tx.send(Request {
                command,
                capture: None,
            })
            .unwrap();
        }
        let mut queue = VecDeque::new();
        drain(&mut queue, &rx);
        let commands: Vec<_> = queue
            .into_iter()
            .filter_map(|req| req.command.shell())
            .collect();
        assert_eq!(
            commands,
            [
                "input motionevent DOWN 1 1",
                "input motionevent MOVE 3 3",
                "input motionevent UP 3 3",
                "input swipe 5 30 5 70 80",
                "input keyevent 4",
            ]
        );
    }

    #[test]
    fn coalesces_queued_finger_moves_but_keeps_down_and_up() {
        let (tx, rx) = mpsc::channel();
        let touch = |t: Touch| Command::Touch(vec![t], String::new());
        for command in [
            touch(Touch::Down(0.1, 0.1)),
            touch(Touch::Move(0.2, 0.2)),
            touch(Touch::Move(0.3, 0.3)),
            touch(Touch::Up),
        ] {
            tx.send(Request {
                command,
                capture: None,
            })
            .unwrap();
        }
        let mut queue = VecDeque::new();
        drain(&mut queue, &rx);
        let steps: Vec<_> = queue
            .into_iter()
            .map(|req| match req.command {
                Command::Touch(steps, _) => steps,
                _ => unreachable!(),
            })
            .collect();
        assert_eq!(
            steps,
            [
                vec![Touch::Down(0.1, 0.1)],
                vec![Touch::Move(0.3, 0.3)],
                vec![Touch::Up]
            ]
        );
    }

    #[cfg(unix)]
    #[test]
    fn persistent_shell_preserves_order_and_acknowledges_each_result() {
        use std::os::unix::fs::PermissionsExt;

        // A fake `input` on PATH stands in for Android's, and `sh -s` reads
        // commands line by line from stdin, as `adb shell sh -s` does.
        // (Sourcing /dev/stdin instead stops after the first read on macOS,
        // whose /bin/sh is bash 3.2.)
        let bin = std::env::temp_dir().join(format!("emugrid-shell-test-{}", std::process::id()));
        std::fs::create_dir_all(&bin).unwrap();
        let input = bin.join("input");
        std::fs::write(
            &input,
            "#!/bin/sh\nprintf ' motionevent <DOWN|UP|MOVE|CANCEL>\\n'\n",
        )
        .unwrap();
        std::fs::set_permissions(&input, std::fs::Permissions::from_mode(0o755)).unwrap();
        let path = format!(
            "{}:{}",
            bin.display(),
            std::env::var("PATH").unwrap_or_default()
        );

        let mut process = Process::new("sh");
        process.arg("-s").env("PATH", path);
        let shell = Shell::open(process).unwrap();
        assert!(shell.live_touch && shell.cancel_touch);
        assert!(shell.run("value=first").unwrap());
        assert!(shell.run("test \"$value\" = first").unwrap());
        assert!(!shell.run("false").unwrap());
        assert!(shell.run("true").unwrap());
    }

    #[test]
    #[ignore = "needs a running Android device from EMUGRID_TEST_SERIAL"]
    fn persistent_shell_round_trips_to_adb() {
        let serial = std::env::var("EMUGRID_TEST_SERIAL").unwrap();
        let adb = sdk::Sdk::detect(None).adb.unwrap();
        let shell = Shell::connect(&adb, &serial).unwrap();
        assert!(shell.run("true").unwrap());
        assert!(!shell.run("false").unwrap());
        assert!(shell.run("true").unwrap());
    }
}
