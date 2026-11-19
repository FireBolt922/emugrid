//! Metadata for AVDs that adb has not seen yet.

use std::path::{Path, PathBuf};

#[derive(Default)]
pub struct Metadata {
    pub display_name: Option<String>,
    pub api: Option<u32>,
    pub os: Option<String>,
}

pub fn home() -> Option<PathBuf> {
    let env = |key| {
        std::env::var_os(key)
            .filter(|value| !value.is_empty())
            .map(PathBuf::from)
    };
    env("ANDROID_AVD_HOME")
        .or_else(|| env("ANDROID_USER_HOME").map(|p| p.join("avd")))
        .or_else(|| {
            env("HOME")
                .or_else(|| env("USERPROFILE"))
                .map(|p| p.join(".android/avd"))
        })
}

fn value<'a>(ini: &'a str, key: &str) -> Option<&'a str> {
    ini.lines()
        .filter_map(|line| line.split_once('='))
        .find_map(|(k, v)| (k.trim() == key && !v.trim().is_empty()).then_some(v.trim()))
}

pub fn read(home: &Path, name: &str) -> Metadata {
    let Ok(ini) = std::fs::read_to_string(home.join(format!("{name}.ini"))) else {
        return Metadata::default();
    };
    value(&ini, "path")
        .and_then(|path| std::fs::read_to_string(Path::new(path).join("config.ini")).ok())
        .map_or_else(Metadata::default, |ini| parse(&ini))
}

fn parse(ini: &str) -> Metadata {
    let api = value(ini, "image.sysdir.1").and_then(|image| {
        image
            .split(['/', '\\'])
            .find_map(|part| part.strip_prefix("android-")?.parse().ok())
    });
    let os = if value(ini, "tag.id").is_some_and(|tag| tag.starts_with("android-wear")) {
        Some(
            match api {
                Some(30) => "Wear OS 3",
                Some(33) => "Wear OS 4",
                Some(34) => "Wear OS 5",
                Some(35) => "Wear OS 5.1",
                Some(36) => "Wear OS 6",
                _ => "Wear OS",
            }
            .into(),
        )
    } else {
        api.and_then(android_version)
            .map(|version| format!("Android {version}"))
    };
    Metadata {
        display_name: value(ini, "avd.ini.displayname").map(String::from),
        api,
        os,
    }
}

pub fn android_version(api: u32) -> Option<&'static str> {
    Some(match api {
        21 => "5.0",
        22 => "5.1",
        23 => "6",
        24 => "7.0",
        25 => "7.1",
        26 => "8.0",
        27 => "8.1",
        28 => "9",
        29 => "10",
        30 => "11",
        31 | 32 => "12",
        33 => "13",
        34 => "14",
        35 => "15",
        36 => "16",
        37 => "17",
        _ => return None,
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn reads_display_name_and_wear_runtime() {
        let base = std::env::temp_dir().join(format!("emugrid-avd-{}", std::process::id()));
        let avd = base.join("watch data");
        std::fs::create_dir_all(&avd).unwrap();
        std::fs::write(
            base.join("Watch.ini"),
            format!("path = {}\n", avd.display()),
        )
        .unwrap();
        std::fs::write(avd.join("config.ini"), "avd.ini.displayname=Pixel Watch\r\nimage.sysdir.1=system-images\\android-34\\android-wear\\x86_64\r\ntag.id=android-wear\n").unwrap();
        let metadata = read(&base, "Watch");
        assert_eq!(metadata.display_name.as_deref(), Some("Pixel Watch"));
        assert_eq!(metadata.api, Some(34));
        assert_eq!(metadata.os.as_deref(), Some("Wear OS 5"));
        assert!(read(&base, "Missing").api.is_none());
        std::fs::remove_dir_all(base).unwrap();
    }

    #[test]
    fn maps_known_android_apis_and_ignores_unknown_ones() {
        assert_eq!(android_version(35), Some("15"));
        assert_eq!(android_version(32), Some("12"));
        assert_eq!(android_version(999), None);
    }
}
