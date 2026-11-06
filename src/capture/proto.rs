//! Just enough protobuf + gRPC framing to talk to the emulator's
//! `EmulatorController.streamScreenshot`. Hand-rolled so we need no codegen.

/// `ImageFormat.ImgFormat.RGB888`.
pub const RGB888: u64 = 2;

/// A decoded `Image` message.
#[derive(Debug, PartialEq)]
pub struct Image {
    pub width: u32,
    pub height: u32,
    pub pixels: Vec<u8>,
}

/// Encode an `ImageFormat { format, width, height }` request.
pub fn encode_image_format(format: u64, width: u32, height: u32) -> Vec<u8> {
    let mut out = Vec::new();
    for (field, value) in [(1, format), (3, width.into()), (4, height.into())] {
        if value != 0 {
            put_varint(field << 3, &mut out);
            put_varint(value, &mut out);
        }
    }
    out
}

/// Decode an `Image` response.
pub fn decode_image(buf: &[u8]) -> Option<Image> {
    let (mut width, mut height, mut pixels) = (0, 0, None);
    let (mut fmt_w, mut fmt_h) = (0, 0);
    for_each_field(buf, |field, value| {
        match (field, value) {
            (1, Value::Bytes(fmt)) => {
                for_each_field(fmt, |f, v| {
                    match (f, v) {
                        (3, Value::Varint(w)) => fmt_w = w as u32,
                        (4, Value::Varint(h)) => fmt_h = h as u32,
                        _ => {}
                    }
                    Some(())
                })?;
            }
            (2, Value::Varint(w)) => width = w as u32,
            (3, Value::Varint(h)) => height = h as u32,
            (4, Value::Bytes(b)) => pixels = Some(b.to_vec()),
            _ => {}
        }
        Some(())
    })?;
    // Newer emulators report the size inside `format`; older ones at the top level.
    let (width, height) = if fmt_w > 0 && fmt_h > 0 {
        (fmt_w, fmt_h)
    } else {
        (width, height)
    };
    Some(Image {
        width,
        height,
        pixels: pixels?,
    })
}

/// Wrap a message in a gRPC length-prefixed frame (uncompressed).
pub fn grpc_frame(msg: &[u8]) -> Vec<u8> {
    let mut out = Vec::with_capacity(msg.len() + 5);
    out.push(0);
    out.extend((msg.len() as u32).to_be_bytes());
    out.extend(msg);
    out
}

/// Splits a stream of bytes into gRPC messages.
#[derive(Default)]
pub struct FrameReader {
    buf: Vec<u8>,
}

impl FrameReader {
    pub fn push(&mut self, data: &[u8]) {
        self.buf.extend_from_slice(data);
    }

    /// The next complete message, if one has fully arrived.
    pub fn next_message(&mut self) -> Option<Vec<u8>> {
        let header = self.buf.get(..5)?;
        let len = u32::from_be_bytes([header[1], header[2], header[3], header[4]]) as usize;
        if self.buf.len() < 5 + len {
            return None;
        }
        let msg = self.buf[5..5 + len].to_vec();
        self.buf.drain(..5 + len);
        Some(msg)
    }
}

enum Value<'a> {
    Varint(u64),
    Bytes(&'a [u8]),
}

fn put_varint(mut v: u64, out: &mut Vec<u8>) {
    while v >= 0x80 {
        out.push((v as u8) | 0x80);
        v >>= 7;
    }
    out.push(v as u8);
}

fn get_varint(buf: &[u8], pos: &mut usize) -> Option<u64> {
    let mut v = 0u64;
    for shift in (0..64).step_by(7) {
        let b = *buf.get(*pos)?;
        *pos += 1;
        v |= u64::from(b & 0x7f) << shift;
        if b < 0x80 {
            return Some(v);
        }
    }
    None
}

/// Walk the fields of a message; `None` if it is malformed or truncated.
fn for_each_field<'a>(
    buf: &'a [u8],
    mut f: impl FnMut(u64, Value<'a>) -> Option<()>,
) -> Option<()> {
    let mut pos = 0;
    while pos < buf.len() {
        let key = get_varint(buf, &mut pos)?;
        let value = match key & 7 {
            0 => Value::Varint(get_varint(buf, &mut pos)?),
            1 => {
                pos += 8;
                continue;
            }
            2 => {
                let len = get_varint(buf, &mut pos)? as usize;
                let bytes = buf.get(pos..pos.checked_add(len)?)?;
                pos += len;
                Value::Bytes(bytes)
            }
            5 => {
                pos += 4;
                continue;
            }
            _ => return None,
        };
        f(key >> 3, value)?;
    }
    (pos == buf.len()).then_some(())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn varint(mut v: u64, out: &mut Vec<u8>) {
        loop {
            let b = (v & 0x7f) as u8;
            v >>= 7;
            if v == 0 {
                out.push(b);
                break;
            }
            out.push(b | 0x80);
        }
    }

    #[test]
    fn encodes_image_format() {
        // field 1 = 2, field 3 = 300, field 4 = 600
        assert_eq!(
            encode_image_format(RGB888, 300, 600),
            vec![0x08, 2, 0x18, 0xac, 0x02, 0x20, 0xd8, 0x04]
        );
    }

    #[test]
    fn decodes_image_with_nested_format_sizes() {
        let mut fmt = vec![0x08, 2];
        fmt.push(0x18);
        varint(270, &mut fmt);
        fmt.push(0x20);
        varint(600, &mut fmt);
        let mut msg = vec![0x0a];
        varint(fmt.len() as u64, &mut msg);
        msg.extend(&fmt);
        msg.push(0x22);
        varint(6, &mut msg);
        msg.extend([1, 2, 3, 4, 5, 6]);
        msg.push(0x28); // seq, skipped
        varint(99, &mut msg);
        msg.push(0x30); // timestampUs, skipped
        varint(1 << 40, &mut msg);
        assert_eq!(
            decode_image(&msg),
            Some(Image {
                width: 270,
                height: 600,
                pixels: vec![1, 2, 3, 4, 5, 6]
            })
        );
    }

    #[test]
    fn decodes_image_with_legacy_top_level_sizes() {
        let mut msg = vec![0x10];
        varint(2, &mut msg);
        msg.push(0x18);
        varint(1, &mut msg);
        msg.push(0x22);
        varint(6, &mut msg);
        msg.extend([9; 6]);
        let img = decode_image(&msg).unwrap();
        assert_eq!((img.width, img.height), (2, 1));
    }

    #[test]
    fn rejects_truncated_messages() {
        assert_eq!(decode_image(&[0x22, 10, 1, 2]), None);
    }

    #[test]
    fn frames_and_reassembles_split_messages() {
        let a = grpc_frame(b"hello");
        assert_eq!(a, vec![0, 0, 0, 0, 5, b'h', b'e', b'l', b'l', b'o']);
        let b = grpc_frame(b"xy");
        let all: Vec<u8> = a.iter().chain(&b).copied().collect();
        let mut r = FrameReader::default();
        r.push(&all[..3]);
        assert_eq!(r.next_message(), None);
        r.push(&all[3..8]);
        assert_eq!(r.next_message(), None);
        r.push(&all[8..]);
        assert_eq!(r.next_message().as_deref(), Some(&b"hello"[..]));
        assert_eq!(r.next_message().as_deref(), Some(&b"xy"[..]));
        assert_eq!(r.next_message(), None);
    }
}
