//! Shortcuts shared by input routing, tooltips and the help dialog.

use egui::{Event, Key, Modifiers};

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Command {
    Filter,
    Back,
    Home,
    Recents,
    VolumeDown,
    VolumeUp,
    Power,
    Rotate,
    Screenshot,
    Device(usize),
}

pub struct Shortcut {
    pub command: Command,
    pub name: &'static str,
    key: Key,
    shift: bool,
    label: &'static str,
    mac_label: &'static str,
}

macro_rules! shortcut {
    ($command:ident, $name:literal, $key:ident, $shift:literal, $label:literal, $mac:literal) => {
        Shortcut {
            command: Command::$command,
            name: $name,
            key: Key::$key,
            shift: $shift,
            label: $label,
            mac_label: $mac,
        }
    };
}

pub const SHORTCUTS: &[Shortcut] = &[
    shortcut!(Filter, "Filter devices", K, false, "K", "K"),
    shortcut!(Back, "Back", ArrowLeft, false, "Left", "←"),
    shortcut!(Home, "Home", H, true, "H", "H"),
    shortcut!(Recents, "Recents", R, true, "R", "R"),
    shortcut!(VolumeDown, "Volume down", ArrowDown, false, "Down", "↓"),
    shortcut!(VolumeUp, "Volume up", ArrowUp, false, "Up", "↑"),
    shortcut!(Power, "Power", P, false, "P", "P"),
    shortcut!(Rotate, "Rotate", R, false, "R", "R"),
    shortcut!(Screenshot, "Screenshot", S, false, "S", "S"),
];

const DIGITS: [Key; 9] = [
    Key::Num1,
    Key::Num2,
    Key::Num3,
    Key::Num4,
    Key::Num5,
    Key::Num6,
    Key::Num7,
    Key::Num8,
    Key::Num9,
];

impl Shortcut {
    pub fn label(&self) -> String {
        self.label_for(cfg!(target_os = "macos"))
    }

    fn label_for(&self, mac: bool) -> String {
        if mac {
            format!("⌘{}{}", if self.shift { "⇧" } else { "" }, self.mac_label)
        } else {
            format!(
                "Ctrl {}{}",
                if self.shift { "Shift " } else { "" },
                self.label
            )
        }
    }
}

pub fn label(command: Command) -> String {
    if let Command::Device(index) = command {
        return format!(
            "{}{n}",
            if cfg!(target_os = "macos") {
                "⌘"
            } else {
                "Ctrl "
            },
            n = index + 1
        );
    }
    SHORTCUTS
        .iter()
        .find(|s| s.command == command)
        .map_or_else(String::new, Shortcut::label)
}

pub fn digit(key: Key) -> Option<usize> {
    DIGITS.iter().position(|&candidate| candidate == key)
}

pub fn take(events: &mut Vec<Event>, focused: bool) -> Vec<Command> {
    let mut commands = Vec::new();
    let mut consumed = false;
    events.retain(|event| {
        if let Event::Key {
            key,
            modifiers,
            pressed,
            repeat,
            ..
        } = event
        {
            let command = detect(*key, *modifiers, focused);
            consumed = command.is_some();
            if let Some(command) = command {
                if *pressed && !*repeat {
                    commands.push(command);
                }
                return false;
            }
        } else if consumed && matches!(event, Event::Text(_)) {
            // Some backends emit a separate text event after the shortcut key.
            return false;
        }
        true
    });
    commands
}

fn detect(key: Key, mods: Modifiers, focused: bool) -> Option<Command> {
    if !mods.command || mods.alt || (mods.mac_cmd && mods.ctrl) {
        return None;
    }
    SHORTCUTS
        .iter()
        .find(|s| {
            s.key == key && s.shift == mods.shift && (focused || s.command == Command::Filter)
        })
        .map(|s| s.command)
        .or_else(|| {
            if focused && !mods.shift {
                digit(key).map(Command::Device)
            } else {
                None
            }
        })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn labels_match_the_command_and_platform() {
        let home = SHORTCUTS
            .iter()
            .find(|s| s.command == Command::Home)
            .unwrap();
        assert_eq!(home.label_for(true), "⌘⇧H");
        assert_eq!(home.label_for(false), "Ctrl Shift H");
    }

    #[test]
    fn consumes_shortcuts_but_keeps_device_typing() {
        let mut events = vec![
            Event::Key {
                key: Key::S,
                physical_key: None,
                pressed: true,
                repeat: false,
                modifiers: Modifiers::COMMAND,
            },
            Event::Text("s".into()),
            Event::Key {
                key: Key::H,
                physical_key: None,
                pressed: true,
                repeat: false,
                modifiers: Modifiers::NONE,
            },
            Event::Text("h".into()),
        ];
        assert_eq!(take(&mut events, true), [Command::Screenshot]);
        assert_eq!(events.len(), 2);
        assert_eq!(events[1], Event::Text("h".into()));
        assert_eq!(
            detect(Key::Num9, Modifiers::COMMAND, true),
            Some(Command::Device(8))
        );
    }
}
