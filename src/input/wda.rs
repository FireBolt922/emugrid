//! Ordered WebDriverAgent input through a local SSH forward.

use serde_json::{Value, json};
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::mpsc::{SyncSender, TrySendError, sync_channel};
use std::thread;
use std::time::Duration;

type Point = (f32, f32);

#[derive(Debug)]
pub enum Gesture {
    Tap(Point),
    Hold(Point),
    Swipe(Point, Point, u32),
}

#[derive(Debug)]
pub enum Command {
    Touch { gesture: Gesture, landscape: bool },
    Text(String),
    Home,
}

pub struct Worker {
    sender: SyncSender<Command>,
    stopped: Arc<AtomicBool>,
}

impl Worker {
    pub fn start(port: u16, low_latency: bool, report: impl Fn(String) + Send + 'static) -> Self {
        let (sender, receive) = sync_channel(32);
        let stopped = Arc::new(AtomicBool::new(false));
        let stop = stopped.clone();
        thread::spawn(move || {
            let mut client = Client::new(port, low_latency);
            if let Err(error) = client.prepare(&report) {
                report(error);
            }
            while let Ok(command) = receive.recv() {
                if stop.load(Ordering::Acquire) {
                    break;
                }
                if let Err(error) = client.execute(command, &report) {
                    client.session = None;
                    client.screen_size = None;
                    report(error);
                }
            }
        });
        Self { sender, stopped }
    }

    pub fn send(&self, command: Command) -> Result<(), &'static str> {
        self.sender.try_send(command).map_err(|error| match error {
            TrySendError::Full(_) => "Input queue is full",
            TrySendError::Disconnected(_) => "Input worker is unavailable",
        })
    }
}

impl Drop for Worker {
    fn drop(&mut self) {
        self.stopped.store(true, Ordering::Release);
    }
}

struct Client {
    agent: ureq::Agent,
    base: String,
    session: Option<String>,
    screen_size: Option<(u32, u32)>,
    low_latency: bool,
}

impl Client {
    fn new(port: u16, low_latency: bool) -> Self {
        let config = ureq::Agent::config_builder()
            .proxy(None)
            .max_redirects(0)
            .http_status_as_error(false)
            .timeout_connect(Some(Duration::from_secs(1)))
            .timeout_global(Some(Duration::from_secs(8)))
            .build();
        Self {
            agent: ureq::Agent::new_with_config(config),
            base: format!("http://127.0.0.1:{port}"),
            session: None,
            screen_size: None,
            low_latency,
        }
    }

    fn request(&self, path: &str, body: Option<Value>) -> Result<Value, String> {
        let url = format!("{}{path}", self.base);
        let mut response = match body {
            Some(body) => self.agent.post(&url).send_json(body),
            None => self.agent.get(&url).call(),
        }
        .map_err(|error| format!("WebDriverAgent request failed: {error}"))?;
        let status = response.status();
        if !status.is_success() {
            return Err(format!("WebDriverAgent returned HTTP {status}"));
        }
        response
            .body_mut()
            .with_config()
            .limit(64 * 1024)
            .read_json()
            .map_err(|_| "Invalid WebDriverAgent response".into())
    }

    fn session_path(&mut self, report: &impl Fn(String)) -> Result<String, String> {
        if self.session.is_none() {
            let status = self.request("/status", None)?;
            let session = if status["sessionId"]
                .as_str()
                .is_some_and(|id| !id.is_empty())
            {
                status
            } else {
                self.request(
                    "/session",
                    Some(json!({"capabilities": {"alwaysMatch": {}}})),
                )?
            };
            let id = session["sessionId"]
                .as_str()
                .or_else(|| session["value"]["sessionId"].as_str())
                .filter(|id| {
                    !id.is_empty()
                        && id.len() <= 128
                        && id.bytes().all(|c| c.is_ascii_alphanumeric() || c == b'-')
                })
                .ok_or("WebDriverAgent did not return a valid session ID")?;
            let path = format!("/session/{id}");
            self.session = Some(path.clone());
            if self.low_latency
                && let Err(error) = self.request(
                    &format!("{path}/appium/settings"),
                    Some(json!({"settings": {
                        "animationCoolOffTimeout": 0,
                        "waitForIdleTimeout": 0,
                        "mjpegServerFramerate": 30,
                        "mjpegScalingFactor": 50,
                    }})),
                )
            {
                report(format!("Could not apply low-latency settings: {error}"));
            }
        }
        Ok(self.session.as_ref().unwrap().clone())
    }

    fn prepare(&mut self, report: &impl Fn(String)) -> Result<(), String> {
        let session = self.session_path(report)?;
        self.point_size(&session).map(|_| ())
    }

    fn point_size(&mut self, session: &str) -> Result<(u32, u32), String> {
        if let Some(size) = self.screen_size {
            return Ok(size);
        }
        // Screen points are independent of MJPEG resolution or device rotation.
        // Older WebDriverAgent builds can report them through the session instead.
        let screen = self.request("/wda/screen", None);
        let size = screen
            .ok()
            .and_then(|response| point_size(&response["value"]["screenSize"]));
        let size = match size {
            Some(size) => size,
            None => point_size(&self.request(&format!("{session}/window/size"), None)?["value"])
                .ok_or("WebDriverAgent did not return the iPhone's screen size")?,
        };
        self.screen_size = Some(size);
        Ok(size)
    }

    fn execute(&mut self, command: Command, report: &impl Fn(String)) -> Result<(), String> {
        if matches!(command, Command::Home) {
            return self.request("/wda/homescreen", Some(json!({}))).map(|_| ());
        }
        let session = self.session_path(report)?;
        match command {
            Command::Text(text) => self.request(
                &format!("{session}/wda/keys"),
                Some(json!({"value": [text]})),
            )?,
            Command::Touch { gesture, landscape } => {
                let size = self.point_size(&session)?;
                self.request(
                    &format!("{session}/actions"),
                    Some(touch(gesture, size, landscape)),
                )?
            }
            Command::Home => unreachable!(),
        };
        Ok(())
    }
}

fn point_size(size: &Value) -> Option<(u32, u32)> {
    Some((dimension(&size["width"])?, dimension(&size["height"])?))
}

fn dimension(value: &Value) -> Option<u32> {
    let number = value.as_f64()?;
    (1.0..=4096.0)
        .contains(&number)
        .then_some(number.round() as u32)
}

fn touch(gesture: Gesture, size: (u32, u32), landscape: bool) -> Value {
    let (from, to, millis) = match gesture {
        Gesture::Tap(point) => (point, point, 0),
        Gesture::Hold(point) => (point, point, 650),
        // A swipe is already drawn locally before WDA replays it on release.
        Gesture::Swipe(from, to, millis) => (from, to, millis.min(250)),
    };
    let move_to = |point, duration| {
        let (x, y) = super::to_device(point, size, landscape);
        json!({"type": "pointerMove", "duration": duration, "origin": "viewport", "x": x, "y": y})
    };
    let mut actions = vec![
        move_to(from, 0),
        json!({"type": "pointerDown", "button": 0}),
    ];
    if from != to {
        actions.push(move_to(to, millis));
    } else if millis > 0 {
        actions.push(json!({"type": "pause", "duration": millis}));
    }
    actions.push(json!({"type": "pointerUp", "button": 0}));
    json!({"actions": [{"type": "pointer", "id": "finger", "parameters": {"pointerType": "touch"}, "actions": actions}]})
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::io::{BufRead, BufReader, Read, Write};
    use std::net::TcpListener;

    #[test]
    fn prewarms_existing_session_and_sends_only_one_request_per_touch() {
        let listener = TcpListener::bind((std::net::Ipv4Addr::LOCALHOST, 0)).unwrap();
        let port = listener.local_addr().unwrap().port();
        let server = thread::spawn(move || {
            let replies = [
                ("GET /status", json!({"sessionId":"existing"})),
                (
                    "POST /session/existing/appium/settings",
                    json!({"value":null}),
                ),
                (
                    "GET /wda/screen",
                    json!({"value":{"screenSize":{"width":390,"height":844}}}),
                ),
                ("POST /session/existing/actions", json!({"value":null})),
                ("POST /session/existing/actions", json!({"value":null})),
            ];
            for (index, (expected, response)) in replies.into_iter().enumerate() {
                let (mut socket, _) = listener.accept().unwrap();
                socket
                    .set_read_timeout(Some(Duration::from_secs(2)))
                    .unwrap();
                let mut reader = BufReader::new(&socket);
                let mut line = String::new();
                reader.read_line(&mut line).unwrap();
                assert!(line.starts_with(&format!("{expected} HTTP/")), "{line}");
                let mut content_length = 0;
                loop {
                    line.clear();
                    assert_ne!(reader.read_line(&mut line).unwrap(), 0);
                    if line == "\r\n" {
                        break;
                    }
                    if let Some((key, value)) = line.split_once(':')
                        && key.eq_ignore_ascii_case("content-length")
                    {
                        content_length = value.trim().parse().unwrap();
                    }
                }
                let mut body = vec![0; content_length];
                reader.read_exact(&mut body).unwrap();
                if index == 1 {
                    let request: Value = serde_json::from_slice(&body).unwrap();
                    assert_eq!(request["settings"]["animationCoolOffTimeout"], 0);
                    assert_eq!(request["settings"]["mjpegServerFramerate"], 30);
                    assert_eq!(request["settings"]["mjpegScalingFactor"], 50);
                } else if index >= 3 {
                    let request: Value = serde_json::from_slice(&body).unwrap();
                    let move_to = &request["actions"][0]["actions"][0];
                    assert_eq!(move_to["x"], if index == 3 { 195 } else { 422 });
                    assert_eq!(move_to["y"], if index == 3 { 422 } else { 195 });
                }
                write!(
                    socket,
                    "HTTP/1.1 200 OK\r\nConnection: close\r\nContent-Length: {}\r\n\r\n{response}",
                    response.to_string().len()
                )
                .unwrap();
            }
        });
        let mut client = Client::new(port, true);
        let report = |error| panic!("{error}");
        client.prepare(&report).unwrap();
        for landscape in [false, true] {
            client
                .execute(
                    Command::Touch {
                        gesture: Gesture::Tap((0.5, 0.5)),
                        landscape,
                    },
                    &report,
                )
                .unwrap();
        }
        server.join().unwrap();
    }

    #[test]
    fn maps_preview_coordinates_to_wda_points_and_builds_ordered_gestures() {
        let swipe = touch(
            Gesture::Swipe((0.5, 0.8), (0.5, 0.2), 600),
            (402, 874),
            false,
        );
        let actions = swipe["actions"][0]["actions"].as_array().unwrap();
        assert_eq!(
            (actions[0]["x"].as_u64(), actions[0]["y"].as_u64()),
            (Some(201), Some(699))
        );
        assert_eq!(actions[1]["type"], "pointerDown");
        assert_eq!(actions[2]["duration"], 250);
        assert_eq!(actions[3]["type"], "pointerUp");
        let hold = touch(Gesture::Hold((1.0, 1.0)), (402, 874), true);
        let actions = hold["actions"][0]["actions"].as_array().unwrap();
        assert_eq!(
            (actions[0]["x"].as_u64(), actions[0]["y"].as_u64()),
            (Some(873), Some(401))
        );
        assert_eq!(actions[2], json!({"type": "pause", "duration": 650}));
    }
}
