//! Sending touches and keys to Android devices with `adb shell input`.

/// Quote text for `adb shell input text`: spaces become `%s`, and the whole
/// thing is single-quoted for the device shell.
pub fn escape_text(text: &str) -> String {
    format!("'{}'", text.replace('\'', r"'\''").replace(' ', "%s"))
}

/// Parse `adb shell wm size` (an override wins over the physical size).
pub fn parse_wm_size(out: &str) -> Option<(u32, u32)> {
    let size = |prefix: &str| {
        out.lines()
            .find_map(|l| l.trim().strip_prefix(prefix))
            .and_then(|s| s.trim().split_once('x'))
            .and_then(|(w, h)| Some((w.trim().parse().ok()?, h.trim().parse().ok()?)))
    };
    size("Override size:").or_else(|| size("Physical size:"))
}

/// Map a point in the shown image (0..1, 0..1) to device pixels. The screen
/// size from `wm size` is always portrait, so follow the frame's orientation.
pub fn to_device(uv: (f32, f32), screen: (u32, u32), frame_landscape: bool) -> (u32, u32) {
    let (short, long) = (screen.0.min(screen.1), screen.0.max(screen.1));
    let (w, h) = if frame_landscape {
        (long, short)
    } else {
        (short, long)
    };
    let axis =
        |t: f32, len: u32| ((t.clamp(0.0, 1.0) * len as f32) as u32).min(len.saturating_sub(1));
    (axis(uv.0, w), axis(uv.1, h))
}

/// Android key code for an egui key we forward.
pub fn keycode(key: eframe::egui::Key) -> Option<u32> {
    use eframe::egui::Key;
    Some(match key {
        Key::Backspace => 67,
        Key::Enter => 66,
        Key::Tab => 61,
        Key::Delete => 112,
        Key::ArrowUp => 19,
        Key::ArrowDown => 20,
        Key::ArrowLeft => 21,
        Key::ArrowRight => 22,
        _ => return None,
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use eframe::egui::Key;

    #[test]
    fn escapes_spaces_and_quotes() {
        assert_eq!(escape_text("it's a b"), r"'it'\''s%sa%sb'");
        assert_eq!(escape_text("$HOME;&|"), "'$HOME;&|'");
    }

    #[test]
    fn parses_wm_size() {
        assert_eq!(
            parse_wm_size("Physical size: 1080x2400\n"),
            Some((1080, 2400))
        );
        assert_eq!(
            parse_wm_size("Physical size: 1080x2400\nOverride size: 720x1600\n"),
            Some((720, 1600))
        );
        assert_eq!(parse_wm_size("error"), None);
    }

    #[test]
    fn maps_coordinates_with_rotation() {
        assert_eq!(to_device((0.5, 0.5), (1080, 2400), false), (540, 1200));
        assert_eq!(to_device((1.0, 0.0), (1080, 2400), true), (2399, 0));
        assert_eq!(to_device((-0.2, 1.5), (1080, 2400), false), (0, 2399));
    }

    #[test]
    fn maps_keys() {
        assert_eq!(keycode(Key::Backspace), Some(67));
        assert_eq!(keycode(Key::Enter), Some(66));
        assert_eq!(keycode(Key::A), None);
    }
}
