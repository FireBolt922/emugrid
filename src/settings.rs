//! Small, per-user settings file.

use std::collections::BTreeMap;
use std::ffi::OsString;
use std::io::{self, Write};
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicU64, Ordering};

#[derive(Debug, PartialEq)]
pub struct Settings {
    pub sdk_root: Option<PathBuf>,
    pub tile_width: u32,
    pub sidebar: bool,
    pub ios_streams: Vec<(u16, String)>,
    pub ios_controls: BTreeMap<u16, u16>,
    pub ios_low_latency: bool,
}

impl Default for Settings {
    fn default() -> Self {
        Self {
            sdk_root: None,
            tile_width: 200,
            sidebar: true,
            ios_streams: Vec::new(),
            ios_controls: BTreeMap::new(),
            ios_low_latency: false,
        }
    }
}

impl Settings {
    pub fn load() -> Self {
        path(|key| std::env::var_os(key))
            .and_then(|path| std::fs::read_to_string(path).ok())
            .map_or_else(Self::default, |text| Self::parse(&text))
    }

    pub fn save(&self) -> io::Result<()> {
        let path = path(|key| std::env::var_os(key))
            .ok_or_else(|| io::Error::new(io::ErrorKind::NotFound, "no user config directory"))?;
        self.write(&path)
    }

    fn parse(text: &str) -> Self {
        let mut settings = Self::default();
        for (key, value) in text.lines().filter_map(|line| line.split_once('=')) {
            match key.trim() {
                "sdk_root" if !value.trim().is_empty() => settings.sdk_root = Some(value.into()),
                "tile_width" => {
                    if let Ok(width) = value.trim().parse::<u32>() {
                        settings.tile_width = width.clamp(140, 320);
                    }
                }
                "sidebar" => {
                    if let Ok(show) = value.trim().parse() {
                        settings.sidebar = show;
                    }
                }
                "ios_stream" => {
                    if let Some((port, name)) = value.split_once(',')
                        && let Ok(port) = port.trim().parse::<u16>()
                        && port != 0
                        && !name.trim().is_empty()
                        && !settings.ios_streams.iter().any(|(p, _)| *p == port)
                    {
                        settings.ios_streams.push((port, name.trim().into()));
                    }
                }
                "ios_control" => {
                    if let Some((stream, control)) = value.split_once(',')
                        && let (Ok(stream), Ok(control)) =
                            (stream.trim().parse(), control.trim().parse())
                        && stream != 0
                        && control != 0
                        && stream != control
                    {
                        settings.ios_controls.entry(stream).or_insert(control);
                    }
                }
                "ios_low_latency" => {
                    settings.ios_low_latency = value.trim() == "true";
                }
                _ => {}
            }
        }
        settings
    }

    fn write(&self, path: &Path) -> io::Result<()> {
        static NEXT: AtomicU64 = AtomicU64::new(0);
        let root = self
            .sdk_root
            .as_ref()
            .map(|root| root.to_str().filter(|s| !s.contains(['\r', '\n'])))
            .unwrap_or(Some(""))
            .ok_or_else(|| {
                io::Error::new(io::ErrorKind::InvalidInput, "invalid SDK folder path")
            })?;
        let dir = path.parent().ok_or(io::ErrorKind::InvalidInput)?;
        std::fs::create_dir_all(dir)?;
        let temp = dir.join(format!(
            ".settings.{}.{}.tmp",
            std::process::id(),
            NEXT.fetch_add(1, Ordering::Relaxed)
        ));
        let mut file = std::fs::File::create_new(&temp)?;
        let result = (|| {
            writeln!(
                file,
                "sdk_root={root}\ntile_width={}\nsidebar={}",
                self.tile_width, self.sidebar
            )?;
            for (port, name) in &self.ios_streams {
                writeln!(file, "ios_stream={port},{name}")?;
            }
            for (stream, control) in &self.ios_controls {
                writeln!(file, "ios_control={stream},{control}")?;
            }
            if self.ios_low_latency {
                writeln!(file, "ios_low_latency=true")?;
            }
            file.sync_all()?;
            drop(file);
            std::fs::rename(&temp, path)
        })();
        if result.is_err() {
            let _ = std::fs::remove_file(temp);
        }
        result
    }
}

fn path(env: impl Fn(&str) -> Option<OsString>) -> Option<PathBuf> {
    let get = |key| {
        env(key)
            .filter(|value| !value.is_empty())
            .map(PathBuf::from)
    };
    let home = || get("HOME");
    let dir = if cfg!(target_os = "macos") {
        home()?.join("Library/Application Support")
    } else if cfg!(windows) {
        get("APPDATA")?
    } else {
        get("XDG_CONFIG_HOME")
            .filter(|p| p.is_absolute())
            .or_else(|| home().map(|p| p.join(".config")))?
    };
    Some(dir.join("emugrid/settings.conf"))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn tolerates_unknown_keys_and_invalid_values() {
        let parsed = Settings::parse(
            "future=anything\nmalformed\ntile_width=1000\nsidebar=no\nsdk_root=/my/sdk=2\n",
        );
        assert_eq!(parsed.tile_width, 320);
        assert!(parsed.sidebar);
        assert_eq!(parsed.sdk_root, Some("/my/sdk=2".into()));
        assert_eq!(Settings::parse("tile_width=invalid"), Settings::default());
    }

    #[test]
    fn parses_named_ios_streams_with_unique_valid_ports() {
        let settings = Settings::parse(
            "ios_stream=9111, iPhone 1\nios_stream=9112,iPhone 2\nios_stream=9111,Duplicate\nios_stream=0,Bad\nios_stream=9999, \nios_stream=bad,Invalid",
        );
        assert_eq!(
            settings.ios_streams,
            [(9111, "iPhone 1".into()), (9112, "iPhone 2".into())]
        );
    }

    #[test]
    fn pairs_ios_control_with_explicit_stream_port() {
        let settings = Settings::parse(
            "ios_control=9111,9101\nios_control=9112,9102\nios_control=9111,9123\nios_control=9115,9115\nios_control=9116,0\nios_control=9117,invalid",
        );
        assert_eq!(
            settings.ios_controls,
            BTreeMap::from([(9111, 9101), (9112, 9102)])
        );
    }

    #[test]
    fn round_trips_and_replaces_settings() {
        let dir = std::env::temp_dir().join(format!("emugrid-settings-{}", std::process::id()));
        let path = dir.join("settings.conf");
        Settings::default().write(&path).unwrap();
        let settings = Settings {
            sdk_root: Some("/example path/SDK".into()),
            tile_width: 240,
            sidebar: false,
            ios_streams: vec![(9111, "iPhone 1".into())],
            ios_controls: BTreeMap::from([(9111, 9101)]),
            ios_low_latency: true,
        };
        settings.write(&path).unwrap();
        assert_eq!(
            Settings::parse(&std::fs::read_to_string(path).unwrap()),
            settings
        );
        assert_eq!(std::fs::read_dir(&dir).unwrap().count(), 1);
        std::fs::remove_dir_all(dir).unwrap();
    }
}
