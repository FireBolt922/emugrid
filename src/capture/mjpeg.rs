//! Live iOS screens from WebDriverAgent over an SSH loopback forward.

use super::{Control, Frame, NATIVE_SIZE, png};
use std::io::{self, BufRead, BufReader, Cursor, Read, Write};
use std::net::{Ipv4Addr, SocketAddr, TcpStream};
use std::sync::atomic::Ordering;
use std::time::{Duration, Instant};

const IO_TIMEOUT: Duration = Duration::from_secs(1);
const MAX_FRAME_BYTES: usize = 8 * 1024 * 1024;

fn invalid(message: &'static str) -> io::Error {
    io::Error::new(io::ErrorKind::InvalidData, message)
}

fn line(reader: &mut impl BufRead) -> io::Result<String> {
    let mut text = String::new();
    reader.take(1024).read_line(&mut text)?;
    if !text.ends_with('\n') {
        return Err(invalid("Incomplete MJPEG header"));
    }
    Ok(text.trim_end().to_owned())
}

fn headers(reader: &mut impl BufRead, name: &str) -> io::Result<String> {
    let mut value = None;
    for _ in 0..32 {
        let text = line(reader)?;
        if text.is_empty() {
            return value.ok_or_else(|| invalid("Missing MJPEG header"));
        }
        if let Some((key, field)) = text.split_once(':')
            && key.eq_ignore_ascii_case(name)
        {
            value = Some(field.trim().to_owned());
        }
    }
    Err(invalid("Too many MJPEG headers"))
}

struct Stream<R> {
    reader: R,
    boundary: String,
}

impl<R: BufRead> Stream<R> {
    fn parse(mut reader: R) -> io::Result<Self> {
        let status = line(&mut reader)?;
        if status.split_whitespace().nth(1) != Some("200") {
            return Err(invalid("MJPEG HTTP request failed"));
        }
        let content_type = headers(&mut reader, "content-type")?;
        let boundary = content_type
            .strip_prefix("multipart/x-mixed-replace;")
            .and_then(|value| value.trim().strip_prefix("boundary="))
            .map(|value| value.trim_matches('"'))
            .filter(|value| !value.is_empty())
            .ok_or_else(|| invalid("Missing MJPEG boundary"))?;
        Ok(Self {
            reader,
            boundary: boundary.to_owned(),
        })
    }

    fn next_frame(&mut self, jpeg: &mut Vec<u8>) -> io::Result<()> {
        let mut boundary = line(&mut self.reader)?;
        // WDA writes two CRLF separators after each JPEG.
        for _ in 0..4 {
            if !boundary.is_empty() {
                break;
            }
            boundary = line(&mut self.reader)?;
        }
        // WDA includes the leading dashes in the boundary response header.
        if boundary != self.boundary && boundary.strip_prefix("--") != Some(&self.boundary) {
            return Err(invalid("Unexpected MJPEG boundary"));
        }
        let length = headers(&mut self.reader, "content-length")?
            .parse::<usize>()
            .ok()
            .filter(|length| (1..=MAX_FRAME_BYTES).contains(length))
            .ok_or_else(|| invalid("Invalid MJPEG frame length"))?;
        jpeg.resize(length, 0);
        self.reader.read_exact(jpeg)
    }
}

fn connect(port: u16) -> io::Result<Stream<BufReader<TcpStream>>> {
    let address = SocketAddr::from((Ipv4Addr::LOCALHOST, port));
    let mut socket = TcpStream::connect_timeout(&address, IO_TIMEOUT)?;
    socket.set_read_timeout(Some(IO_TIMEOUT))?;
    socket.set_write_timeout(Some(IO_TIMEOUT))?;
    socket.write_all(b"GET / HTTP/1.0\r\nHost: localhost\r\n\r\n")?;
    Stream::parse(BufReader::new(socket))
}

pub fn available(port: u16) -> bool {
    connect(port).is_ok()
}

pub fn screenshot(port: u16) -> io::Result<Vec<u8>> {
    let mut jpeg = Vec::new();
    connect(port)?.next_frame(&mut jpeg)?;
    Ok(jpeg)
}

fn decode(jpeg: &[u8]) -> Option<Frame> {
    let mut reader = image::ImageReader::with_format(Cursor::new(jpeg), image::ImageFormat::Jpeg);
    let mut limits = image::Limits::default();
    limits.max_image_width = Some(4096);
    limits.max_image_height = Some(4096);
    reader.limits(limits);
    let rgb = reader.decode().ok()?.into_rgb8();
    let (width, height) = rgb.dimensions();
    Some(Frame {
        width,
        height,
        rgb: rgb.into_raw(),
    })
}

pub fn stream(port: u16, control: &Control, send: &impl Fn(Frame)) -> io::Result<()> {
    let mut stream = connect(port)?;
    let mut jpeg = Vec::new();
    let mut next_frame: Option<(u32, Instant)> = None;
    while control.active.load(Ordering::Acquire) && !control.stop.load(Ordering::Acquire) {
        stream.next_frame(&mut jpeg)?;
        let fps = control.max_fps.load(Ordering::Relaxed).max(1);
        let interval = Duration::from_secs(1) / fps;
        let now = Instant::now();
        let deadline = next_frame.filter(|&(previous_fps, _)| previous_fps == fps);
        if deadline.is_some_and(|(_, due)| now < due) {
            continue;
        }
        if let Some(frame) = decode(&jpeg) {
            // Stay on the existing FPS schedule so a 20 FPS source capped at
            // 15 FPS does not always skip every second frame.
            let base = deadline
                .map(|(_, due)| due)
                .filter(|&due| now.duration_since(due) < interval)
                .unwrap_or(now);
            next_frame = Some((fps, base + interval));
            control.record_size(&frame);
            let size = control.size.load(Ordering::Relaxed);
            send(if size == NATIVE_SIZE {
                frame
            } else {
                png::downscale(frame, size)
            });
        }
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    const HTTP: &[u8] = b"HTTP/1.0 200 OK\r\nContent-Type: multipart/x-mixed-replace; boundary=--BoundaryString\r\n\r\n";

    #[test]
    fn reads_and_decodes_multiple_wda_frames() {
        let mut jpeg = Vec::new();
        image::codecs::jpeg::JpegEncoder::new(&mut jpeg)
            .encode(&[0, 0, 0], 1, 1, image::ExtendedColorType::Rgb8)
            .unwrap();
        let mut response = HTTP.to_vec();
        for _ in 0..2 {
            write!(
                response,
                "--BoundaryString\r\nContent-type: image/jpeg\r\nContent-Length: {}\r\n\r\n",
                jpeg.len()
            )
            .unwrap();
            response.extend_from_slice(&jpeg);
            response.extend_from_slice(b"\r\n\r\n");
        }
        let mut stream = Stream::parse(response.as_slice()).unwrap();
        let mut frame = Vec::new();
        for _ in 0..2 {
            stream.next_frame(&mut frame).unwrap();
            let frame = decode(&frame).unwrap();
            assert_eq!(
                (frame.width, frame.height, frame.rgb),
                (1, 1, vec![0, 0, 0])
            );
        }
    }

    #[test]
    fn rejects_oversized_and_truncated_frames() {
        for size in [MAX_FRAME_BYTES + 1, 10] {
            let mut response = HTTP.to_vec();
            write!(
                response,
                "--BoundaryString\r\nContent-Length: {size}\r\n\r\nabc"
            )
            .unwrap();
            let mut stream = Stream::parse(response.as_slice()).unwrap();
            assert!(stream.next_frame(&mut Vec::new()).is_err());
        }
    }
}
