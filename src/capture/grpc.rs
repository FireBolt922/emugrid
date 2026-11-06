//! Live screen stream from SDK-started emulators over their gRPC endpoint
//! (`EmulatorController.streamScreenshot`), the same API Android Studio uses.

use super::proto::{self, FrameReader};
use super::{Control, Frame};
use std::collections::HashMap;
use std::path::PathBuf;
use std::sync::atomic::Ordering;
use std::time::{Duration, Instant};

#[derive(Debug, Clone, PartialEq)]
pub struct Endpoint {
    /// Console port, i.e. the `5554` in `emulator-5554`.
    pub console_port: u16,
    pub grpc_port: u16,
    pub token: Option<String>,
}

impl Endpoint {
    pub fn serial(&self) -> String {
        format!("emulator-{}", self.console_port)
    }
}

/// Parse one emulator discovery file (`pid_NNNN.ini`).
pub fn parse_discovery(text: &str) -> Option<Endpoint> {
    let get = |key: &str| {
        text.lines()
            .filter_map(|l| l.trim().split_once('='))
            .find(|(k, _)| k.trim() == key)
            .map(|(_, v)| v.trim().to_string())
    };
    Some(Endpoint {
        console_port: get("port.serial")?.parse().ok()?,
        grpc_port: get("grpc.port")?.parse().ok()?,
        token: get("grpc.token").filter(|t| !t.is_empty()),
    })
}

/// Every running emulator that exposes gRPC, keyed by adb serial.
pub fn discover() -> HashMap<String, Endpoint> {
    discover_in(&discovery_dirs())
}

/// Like `discover`, over given folders. When several files name the same
/// emulator (leftovers from a crash), the most recently written one wins.
pub fn discover_in(dirs: &[PathBuf]) -> HashMap<String, Endpoint> {
    let mut found: HashMap<String, (std::time::SystemTime, Endpoint)> = HashMap::new();
    for dir in dirs {
        let Ok(entries) = std::fs::read_dir(dir) else {
            continue;
        };
        for entry in entries.flatten() {
            let path = entry.path();
            let is_ini = path.extension().is_some_and(|e| e == "ini");
            if !is_ini {
                continue;
            }
            let Some(ep) = std::fs::read_to_string(&path)
                .ok()
                .and_then(|t| parse_discovery(&t))
            else {
                continue;
            };
            let modified = entry
                .metadata()
                .and_then(|m| m.modified())
                .unwrap_or(std::time::UNIX_EPOCH);
            let newer = found.get(&ep.serial()).is_none_or(|(t, _)| modified > *t);
            if newer {
                found.insert(ep.serial(), (modified, ep));
            }
        }
    }
    found.into_iter().map(|(k, (_, ep))| (k, ep)).collect()
}

/// Where the emulator writes its discovery files on each OS.
fn discovery_dirs() -> Vec<PathBuf> {
    let mut dirs = Vec::new();
    let home = std::env::var_os("HOME").map(PathBuf::from);
    if let Some(h) = &home {
        dirs.push(h.join("Library/Caches/TemporaryItems/avd/running")); // macOS
    }
    if let Some(x) = std::env::var_os("XDG_RUNTIME_DIR") {
        dirs.push(PathBuf::from(x).join("avd/running")); // Linux
    }
    if let Some(l) = std::env::var_os("LOCALAPPDATA") {
        dirs.push(PathBuf::from(l).join("Temp/avd/running")); // Windows
    }
    let tmp = std::env::temp_dir();
    dirs.push(tmp.join("avd/running"));
    if let Some(user) = std::env::var_os("USER").or_else(|| std::env::var_os("USERNAME")) {
        dirs.push(
            tmp.join(format!("android-{}", user.to_string_lossy()))
                .join("avd/running"),
        );
    }
    dirs
}

/// Stream frames until `control` says stop, the size changes, or the
/// connection drops. Returns `Ok` when the caller should simply reconnect.
pub fn stream(ep: &Endpoint, control: &Control, mut send: impl FnMut(Frame)) -> Result<(), String> {
    let rt = tokio::runtime::Builder::new_current_thread()
        .enable_all()
        .build()
        .map_err(|e| e.to_string())?;
    rt.block_on(stream_async(ep, control, &mut send))
}

/// How long connecting and getting response headers may take.
const SETUP_TIMEOUT: Duration = Duration::from_secs(5);

type Opened = (h2::client::SendRequest<bytes::Bytes>, h2::RecvStream);

/// Connect, send the `streamScreenshot` request and wait for its headers.
async fn open(ep: &Endpoint, size: u32) -> Result<Opened, String> {
    let err = |e: &dyn std::fmt::Display| e.to_string();
    let tcp = tokio::net::TcpStream::connect(("127.0.0.1", ep.grpc_port))
        .await
        .map_err(|e| err(&e))?;
    tcp.set_nodelay(true).ok();
    let (client, conn) = h2::client::Builder::new()
        .initial_window_size(8 << 20)
        .initial_connection_window_size(16 << 20)
        .max_frame_size(1 << 20)
        .handshake::<_, bytes::Bytes>(tcp)
        .await
        .map_err(|e| err(&e))?;
    tokio::spawn(async move {
        let _ = conn.await;
    });
    let mut client = client.ready().await.map_err(|e| err(&e))?;

    let mut req = http::Request::builder()
        .method("POST")
        .uri(format!(
            "http://127.0.0.1:{}/android.emulation.control.EmulatorController/streamScreenshot",
            ep.grpc_port
        ))
        .header("content-type", "application/grpc")
        .header("te", "trailers");
    if let Some(token) = &ep.token {
        req = req.header("authorization", format!("Bearer {token}"));
    }
    let req = req.body(()).map_err(|e| err(&e))?;
    let (resp, mut body_tx) = client.send_request(req, false).map_err(|e| err(&e))?;
    let msg = proto::encode_image_format(proto::RGB888, size, size);
    body_tx
        .send_data(proto::grpc_frame(&msg).into(), true)
        .map_err(|e| err(&e))?;

    let resp = resp.await.map_err(|e| err(&e))?;
    if let Some(status) = resp.headers().get("grpc-status")
        && status != "0"
    {
        let message = resp
            .headers()
            .get("grpc-message")
            .and_then(|m| m.to_str().ok())
            .unwrap_or("");
        return Err(format!("grpc-status {status:?} {message}"));
    }
    Ok((client, resp.into_body()))
}

async fn stream_async(
    ep: &Endpoint,
    control: &Control,
    send: &mut impl FnMut(Frame),
) -> Result<(), String> {
    let err = |e: &dyn std::fmt::Display| e.to_string();
    let size = control.size.load(Ordering::Relaxed);
    // An emulator that is paused, or a stale port held by something else,
    // must not hang the worker.
    let (_client, mut body) = tokio::time::timeout(SETUP_TIMEOUT, open(ep, size))
        .await
        .map_err(|_| "timed out connecting".to_string())??;
    let mut reader = FrameReader::default();
    let mut last_sent: Option<Instant> = None;
    // The emulator only pushes when the screen changes, so a frame skipped by
    // the rate cap is kept here and flushed later instead of being lost.
    let mut pending: Option<Vec<u8>> = None;
    loop {
        // Hidden tile, new size or shutdown: hang up so the emulator stops encoding.
        if control.stop.load(Ordering::Relaxed)
            || !control.active.load(Ordering::Relaxed)
            || control.size.load(Ordering::Relaxed) != size
        {
            return Ok(());
        }
        let min_gap = Duration::from_secs(1) / control.max_fps.load(Ordering::Relaxed).max(1);
        let wait = match (&pending, last_sent) {
            (Some(_), Some(t)) => min_gap.saturating_sub(t.elapsed()),
            (Some(_), None) => Duration::ZERO,
            (None, _) => Duration::from_millis(250),
        };
        match tokio::time::timeout(wait, body.data()).await {
            Err(_) => {} // nothing new; maybe flush `pending` below
            Ok(None) => return Err("stream ended".into()),
            Ok(Some(chunk)) => {
                let chunk = chunk.map_err(|e| err(&e))?;
                let _ = body.flow_control().release_capacity(chunk.len());
                reader.push(&chunk);
                while let Some(m) = reader.next_message() {
                    pending = Some(m);
                }
            }
        }
        if last_sent.is_some_and(|t| t.elapsed() < min_gap) {
            continue;
        }
        if let Some(img) = pending.take().and_then(|m| proto::decode_image(&m))
            && img.width > 0
            && img.pixels.len() == (img.width * img.height * 3) as usize
        {
            last_sent = Some(Instant::now());
            control.live.store(true, Ordering::Relaxed);
            send(Frame {
                width: img.width,
                height: img.height,
                rgb: img.pixels,
            });
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const SAMPLE: &str = "emulator.build=15917651\nport.serial=5554\nport.adb=5555\navd.name=Medium Phone\ngrpc.token=abc123\ngrpc.port=8554\n";

    #[test]
    fn parses_discovery_file() {
        assert_eq!(
            parse_discovery(SAMPLE),
            Some(Endpoint {
                console_port: 5554,
                grpc_port: 8554,
                token: Some("abc123".into())
            })
        );
    }

    #[test]
    fn discovery_without_grpc_port_is_ignored() {
        assert_eq!(parse_discovery("port.serial=5554\n"), None);
    }

    #[test]
    fn discovery_without_token_and_with_crlf() {
        let ep = parse_discovery("port.serial=5556\r\ngrpc.port=8556\r\n").unwrap();
        assert_eq!(ep.serial(), "emulator-5556");
        assert_eq!(ep.token, None);
    }

    /// A leftover file from a crashed emulator must not shadow the live one.
    #[test]
    fn newest_discovery_file_wins() {
        let dir = std::env::temp_dir().join(format!("emugrid-disc-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        // Live emulator written first, stale one after (so "last read wins" picks it).
        std::fs::write(
            dir.join("pid_200.ini"),
            "port.serial=5554\ngrpc.port=8600\n",
        )
        .unwrap();
        let stale = dir.join("pid_050.ini");
        std::fs::write(&stale, "port.serial=5554\ngrpc.port=8554\n").unwrap();
        let old = std::time::SystemTime::now() - Duration::from_secs(3600);
        std::fs::File::options()
            .write(true)
            .open(&stale)
            .unwrap()
            .set_modified(old)
            .unwrap();
        let found = discover_in(std::slice::from_ref(&dir));
        let _ = std::fs::remove_dir_all(&dir);
        assert_eq!(found["emulator-5554"].grpc_port, 8600);
    }

    /// A port that accepts TCP but never speaks HTTP/2 must not hang the worker.
    #[test]
    fn silent_server_times_out() {
        let listener = std::net::TcpListener::bind("127.0.0.1:0").unwrap();
        let port = listener.local_addr().unwrap().port();
        let _hold = std::thread::spawn(move || {
            let conn = listener.accept();
            std::thread::sleep(Duration::from_secs(30));
            drop(conn);
        });
        let ep = Endpoint {
            console_port: 5554,
            grpc_port: port,
            token: None,
        };
        let (tx, rx) = std::sync::mpsc::channel();
        std::thread::spawn(move || {
            let control = Control::new(320);
            let _ = tx.send(stream(&ep, &control, |_| {}));
        });
        let result = rx.recv_timeout(Duration::from_secs(12));
        assert!(
            matches!(result, Ok(Err(_))),
            "stream hung or succeeded: {result:?}"
        );
    }

    /// Spike: connect to the first local emulator and measure frame rate.
    #[test]
    #[ignore = "needs a running SDK emulator"]
    fn spike_stream_local() {
        let ep = discover().into_values().next().expect("no emulator");
        let control = Control::new(400);
        let start = Instant::now();
        let mut n = 0;
        let stop = &control.stop;
        let _ = std::thread::scope(|s| {
            s.spawn(|| {
                std::thread::sleep(Duration::from_secs(5));
                stop.store(true, Ordering::Relaxed);
            });
            stream(&ep, &control, |f| {
                n += 1;
                if n == 1 {
                    println!(
                        "first frame {}x{} after {:?}",
                        f.width,
                        f.height,
                        start.elapsed()
                    );
                }
            })
        });
        println!("{n} frames in {:?}", start.elapsed());
        assert!(n > 0);
    }
}
