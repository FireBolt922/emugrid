//! Small, per-user settings file.

use std::ffi::OsString;
use std::io::{self, Write};
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicU64, Ordering};

#[derive(Debug, PartialEq)]
pub struct Settings {
    pub sdk_root: Option<PathBuf>,
    pub tile_width: u32,
    pub sidebar: bool,
}

impl Default for Settings {
    fn default() -> Self {
        Self {
            sdk_root: None,
            tile_width: 200,
            sidebar: true,
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
    fn round_trips_and_replaces_settings() {
        let dir = std::env::temp_dir().join(format!("emugrid-settings-{}", std::process::id()));
        let path = dir.join("settings.conf");
        Settings::default().write(&path).unwrap();
        let settings = Settings {
            sdk_root: Some("/example path/SDK".into()),
            tile_width: 240,
            sidebar: false,
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
