//! Fallback capture: poll a screenshot as PNG. Works for every Android device
//! (`adb exec-out screencap -p`) and for iOS simulators (`simctl io screenshot`).

use super::Frame;

/// Decode a PNG into tightly packed RGB.
pub fn decode(bytes: &[u8]) -> Option<Frame> {
    let mut decoder = png::Decoder::new(std::io::Cursor::new(bytes));
    decoder.set_transformations(png::Transformations::EXPAND | png::Transformations::STRIP_16);
    let mut reader = decoder.read_info().ok()?;
    let mut buf = vec![0; reader.output_buffer_size()?];
    let info = reader.next_frame(&mut buf).ok()?;
    let buf = &buf[..info.buffer_size()];
    let rgb = match info.color_type {
        png::ColorType::Rgb => buf.to_vec(),
        png::ColorType::Rgba => buf
            .as_chunks::<4>()
            .0
            .iter()
            .flat_map(|p| [p[0], p[1], p[2]])
            .collect(),
        png::ColorType::Grayscale => buf.iter().flat_map(|&g| [g, g, g]).collect(),
        png::ColorType::GrayscaleAlpha => buf
            .as_chunks::<2>()
            .0
            .iter()
            .flat_map(|p| [p[0]; 3])
            .collect(),
        png::ColorType::Indexed => return None, // EXPAND turns this into RGB(A)
    };
    Some(Frame {
        width: info.width,
        height: info.height,
        rgb,
    })
}

/// Box-filter `frame` so its longest edge is at most `max_edge`.
pub fn downscale(frame: Frame, max_edge: u32) -> Frame {
    let (w, h) = (frame.width as usize, frame.height as usize);
    let longest = w.max(h);
    if longest <= max_edge as usize || longest == 0 {
        return frame;
    }
    let scale = max_edge as f64 / longest as f64;
    let nw = ((w as f64 * scale).round() as usize).max(1);
    let nh = ((h as f64 * scale).round() as usize).max(1);
    let span = |i: usize, src: usize, dst: usize| {
        let a = i * src / dst;
        (a, ((i + 1) * src / dst).max(a + 1))
    };
    let mut rgb = Vec::with_capacity(nw * nh * 3);
    for y in 0..nh {
        let (y0, y1) = span(y, h, nh);
        for x in 0..nw {
            let (x0, x1) = span(x, w, nw);
            let mut sum = [0u32; 3];
            for sy in y0..y1 {
                let row = &frame.rgb[(sy * w + x0) * 3..(sy * w + x1) * 3];
                for p in row.as_chunks::<3>().0 {
                    sum[0] += u32::from(p[0]);
                    sum[1] += u32::from(p[1]);
                    sum[2] += u32::from(p[2]);
                }
            }
            let n = ((y1 - y0) * (x1 - x0)) as u32;
            rgb.extend(sum.map(|s| (s / n) as u8));
        }
    }
    Frame {
        width: nw as u32,
        height: nh as u32,
        rgb,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn png(width: u32, height: u32, color: png::ColorType, data: &[u8]) -> Vec<u8> {
        let mut out = Vec::new();
        let mut enc = png::Encoder::new(&mut out, width, height);
        enc.set_color(color);
        enc.set_depth(png::BitDepth::Eight);
        enc.write_header().unwrap().write_image_data(data).unwrap();
        out
    }

    #[test]
    fn decodes_rgba_to_rgb() {
        let bytes = png(
            2,
            1,
            png::ColorType::Rgba,
            &[255, 0, 0, 255, 0, 0, 255, 128],
        );
        let f = decode(&bytes).unwrap();
        assert_eq!((f.width, f.height), (2, 1));
        assert_eq!(f.rgb, vec![255, 0, 0, 0, 0, 255]);
    }

    #[test]
    fn decodes_rgb_and_gray() {
        let f = decode(&png(1, 1, png::ColorType::Rgb, &[1, 2, 3])).unwrap();
        assert_eq!(f.rgb, vec![1, 2, 3]);
        let f = decode(&png(1, 1, png::ColorType::Grayscale, &[7])).unwrap();
        assert_eq!(f.rgb, vec![7, 7, 7]);
    }

    #[test]
    fn rejects_garbage() {
        assert!(decode(b"not a png").is_none());
    }

    #[test]
    fn downscale_averages_blocks_and_keeps_aspect() {
        // 4x2 image: left half black, right half white -> 2x1 = [black, white]
        let mut rgb = Vec::new();
        for _ in 0..2 {
            rgb.extend([0; 6]);
            rgb.extend([255; 6]);
        }
        let f = downscale(
            Frame {
                width: 4,
                height: 2,
                rgb,
            },
            2,
        );
        assert_eq!((f.width, f.height), (2, 1));
        assert_eq!(f.rgb, vec![0, 0, 0, 255, 255, 255]);
    }

    #[test]
    fn downscale_never_upscales() {
        let f = downscale(
            Frame {
                width: 3,
                height: 1,
                rgb: vec![9; 9],
            },
            100,
        );
        assert_eq!((f.width, f.height), (3, 1));
    }

    #[test]
    fn downscale_real_phone_ratio() {
        let f = downscale(
            Frame {
                width: 1080,
                height: 2400,
                rgb: vec![0; 1080 * 2400 * 3],
            },
            600,
        );
        assert_eq!((f.width, f.height), (270, 600));
        assert_eq!(f.rgb.len(), 270 * 600 * 3);
    }
}
