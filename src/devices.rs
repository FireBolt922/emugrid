//! Discovering emulators and devices, and starting / stopping them.

use crate::capture::mjpeg;
use crate::sdk::{Sdk, command};
use crate::{avd, ios};
use std::collections::BTreeMap;
use std::path::Path;
use std::process::Stdio;

#[derive(Debug, Clone, Copy, PartialEq)]
pub enum Platform {
    Android,
    Ios,
}

#[derive(Debug, Clone, PartialEq)]
pub enum State {
    /// Created in the SDK but not running.
    Stopped,
    /// Running and reachable (adb serial like `emulator-5554`, or an iOS UDID).
    Running {
        serial: String,
    },
    Booting {
        serial: Option<String>,
    },
    /// Seen by adb but offline, unauthorized, or otherwise unavailable.
    Other {
        serial: String,
        status: String,
    },
}

#[derive(Debug, Clone)]
pub struct Device {
    pub display_name: String,
    pub api: Option<u32>,
    pub os: Option<String>,
    pub platform: Platform,
    pub is_emulator: bool,
    pub state: State,
    /// AVD name, if this is one of the SDK's AVDs (enables start / cold boot / wipe).
    pub avd: Option<String>,
    pub udid: Option<String>,
    pub ios_stream_port: Option<u16>,
}

impl Device {
    pub fn serial(&self) -> Option<&str> {
        match &self.state {
            State::Running { serial } => Some(serial),
            _ => None,
        }
    }
    pub fn known_serial(&self) -> Option<&str> {
        match &self.state {
            State::Booting { serial } => serial.as_deref(),
            State::Other { serial, .. } => Some(serial),
            _ => self.serial(),
        }
    }

    pub fn title(&self) -> String {
        match self.api {
            Some(api) => format!("{} · API {api}", self.display_name),
            None => self.display_name.clone(),
        }
    }
}

fn android(name: String, is_emulator: bool, state: State, avd: Option<String>) -> Device {
    Device {
        display_name: name,
        api: None,
        os: None,
        platform: Platform::Android,
        is_emulator,
        state,
        avd,
        udid: None,
        ios_stream_port: None,
    }
}

/// One full scan: installed AVDs merged with whatever adb currently sees.
pub fn scan(sdk: &Sdk, ios_streams: &[(u16, String)]) -> Vec<Device> {
    let mut by_name: BTreeMap<String, Device> = BTreeMap::new();

    let avd_home = avd::home();
    for name in list_avds(sdk) {
        let metadata = avd_home
            .as_deref()
            .map_or_else(avd::Metadata::default, |home| avd::read(home, &name));
        let mut device = android(
            metadata.display_name.unwrap_or_else(|| name.clone()),
            true,
            State::Stopped,
            Some(name.clone()),
        );
        device.api = metadata.api;
        device.os = metadata.os;
        by_name.insert(name, device);
    }

    for (serial, status) in adb_devices(sdk) {
        let ready = status == "device";
        let props = if ready {
            props(sdk, &serial)
        } else {
            Props::default()
        };
        let sdk_emulator = serial.starts_with("emulator-");
        let avd = sdk_emulator.then(|| avd_name(sdk, &serial)).flatten();
        let state = if !ready {
            State::Other {
                serial: serial.clone(),
                status,
            }
        } else if props.booted {
            State::Running {
                serial: serial.clone(),
            }
        } else {
            State::Booting {
                serial: Some(serial.clone()),
            }
        };
        let key = avd.clone().unwrap_or_else(|| format!("~{serial}"));
        let mut device = by_name.remove(&key).unwrap_or_else(|| {
            let name = props
                .model
                .clone()
                .or_else(|| avd.clone())
                .unwrap_or(serial);
            android(name, sdk_emulator || props.qemu, State::Stopped, avd)
        });
        if device.avd.as_deref() == Some(device.display_name.as_str())
            && let Some(model) = props.model
        {
            device.display_name = model;
        }
        device.state = state;
        device.api = props.api.or(device.api);
        if !device
            .os
            .as_deref()
            .is_some_and(|os| os.starts_with("Wear OS"))
        {
            device.os = props
                .release
                .map(|v| format!("Android {v}"))
                .or_else(|| {
                    device
                        .api
                        .and_then(crate::avd::android_version)
                        .map(|v| format!("Android {v}"))
                })
                .or(device.os);
        }
        by_name.insert(key, device);
    }

    let mut list: Vec<Device> = by_name.into_values().collect();
    if sdk.simctl {
        list.extend(ios_simulators());
    }
    list.extend(ios_streams.iter().map(|&(port, ref name)| {
        let serial = format!("ios-stream:{port}");
        Device {
            display_name: name.clone(),
            api: None,
            os: None,
            platform: Platform::Ios,
            is_emulator: true,
            state: if mjpeg::available(port) {
                State::Running { serial }
            } else {
                State::Other {
                    serial,
                    status: "Stream offline".into(),
                }
            },
            avd: None,
            udid: None,
            ios_stream_port: Some(port),
        }
    }));
    // Sort by readiness, then name.
    list.sort_by_key(|d| {
        let order = match d.state {
            State::Running { .. } => 0,
            State::Booting { .. } => 1,
            State::Other { .. } => 2,
            State::Stopped => 3,
        };
        (order, d.display_name.to_lowercase())
    });
    list
}

fn run(program: impl AsRef<std::ffi::OsStr>, args: &[&str]) -> Option<String> {
    let out = command(program)
        .args(args)
        .stderr(Stdio::null())
        .output()
        .ok()?;
    out.status
        .success()
        .then(|| String::from_utf8_lossy(&out.stdout).into_owned())
}

fn list_avds(sdk: &Sdk) -> Vec<String> {
    let Some(emu) = &sdk.emulator else {
        return vec![];
    };
    run(emu, &["-list-avds"])
        .unwrap_or_default()
        .lines()
        .map(str::trim)
        // The emulator sometimes prints INFO/WARNING lines before the list.
        .filter(|l| !l.is_empty() && !l.contains(' ') && !l.starts_with("INFO"))
        .map(String::from)
        .collect()
}

fn adb_devices(sdk: &Sdk) -> Vec<(String, String)> {
    let Some(adb) = &sdk.adb else { return vec![] };
    run(adb, &["devices"])
        .unwrap_or_default()
        .lines()
        .skip(1)
        .filter_map(|l| {
            let mut parts = l.split_whitespace();
            Some((parts.next()?.to_string(), parts.next()?.to_string()))
        })
        .collect()
}

/// Ask a running emulator which AVD it is (`adb -s <serial> emu avd name`).
fn avd_name(sdk: &Sdk, serial: &str) -> Option<String> {
    let out = run(sdk.adb.as_ref()?, &["-s", serial, "emu", "avd", "name"])?;
    out.lines()
        .next()
        .map(|s| s.trim().to_string())
        .filter(|s| !s.is_empty() && !s.starts_with("KO:") && *s != "OK")
}

#[derive(Default)]
struct Props {
    model: Option<String>,
    qemu: bool,
    release: Option<String>,
    api: Option<u32>,
    booted: bool,
}

fn parse_props(out: &str) -> Props {
    let mut lines = out.split('\n').map(str::trim);
    let model = lines.next().filter(|s| !s.is_empty()).map(String::from);
    let qemu = lines.next() == Some("1");
    let release = lines.next().filter(|s| !s.is_empty()).map(String::from);
    let api = lines.next().and_then(|s| s.parse().ok());
    let booted = lines.next() == Some("1");
    Props {
        model,
        qemu,
        release,
        api,
        booted,
    }
}

/// Get model, system and boot state in one adb call.
fn props(sdk: &Sdk, serial: &str) -> Props {
    const COMMAND: &str = concat!(
        "getprop ro.product.model; getprop ro.kernel.qemu; ",
        "getprop ro.build.version.release; getprop ro.build.version.sdk; getprop sys.boot_completed"
    );
    sdk.adb
        .as_ref()
        .and_then(|adb| run(adb, &["-s", serial, "shell", COMMAND]))
        .map_or_else(Props::default, |out| parse_props(&out))
}

fn ios_simulators() -> Vec<Device> {
    let out = run("xcrun", &["simctl", "list", "devices", "available"]).unwrap_or_default();
    ios::parse_list(&out)
        .into_iter()
        // Watch / TV / Vision simulators have no phone-like screen worth gridding.
        .filter(|s| s.runtime.starts_with("iOS"))
        .map(|s| Device {
            display_name: s.name,
            api: None,
            os: Some(s.runtime),
            platform: Platform::Ios,
            is_emulator: true,
            state: if s.booted {
                State::Running {
                    serial: s.udid.clone(),
                }
            } else {
                State::Stopped
            },
            avd: None,
            udid: Some(s.udid),
            ios_stream_port: None,
        })
        .collect()
}

/// How to boot an AVD.
#[derive(Debug, Clone, Copy, PartialEq)]
pub enum Boot {
    Normal,
    /// Ignore the quick-boot snapshot.
    Cold,
    /// Factory reset the AVD first.
    Wipe,
    /// A normal boot that also opens the emulator's own window.
    Window,
}

/// Launch an AVD in the background; the emulator outlives this app.
pub fn start(sdk: &Sdk, avd: &str, boot: Boot) -> Result<std::process::Child, String> {
    let emu = sdk.emulator.as_ref().ok_or("emulator binary not found")?;
    command(emu)
        .args(start_args(avd, boot))
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .spawn()
        .map_err(|e| e.to_string())
}

/// `emulator` arguments. emugrid shows the screen, so no window by default.
fn start_args(avd: &str, boot: Boot) -> Vec<&str> {
    let mut args = vec!["-avd", avd];
    if boot != Boot::Window {
        args.push("-no-window");
    }
    match boot {
        Boot::Normal | Boot::Window => {}
        Boot::Cold => args.push("-no-snapshot-load"),
        Boot::Wipe => args.push("-wipe-data"),
    }
    args
}

/// Shut down a running emulator.
pub fn stop(sdk: &Sdk, serial: &str) -> Result<(), String> {
    emulator_command(sdk, serial, "kill")
}

fn emulator_command(sdk: &Sdk, serial: &str, action: &str) -> Result<(), String> {
    let adb = sdk.adb.as_ref().ok_or("adb not found")?;
    let output = command(adb)
        .args(["-s", serial, "emu", action])
        .output()
        .map_err(|error| error.to_string())?;
    let stdout = String::from_utf8_lossy(&output.stdout);
    let stderr = String::from_utf8_lossy(&output.stderr);
    let rejection = stdout.lines().find(|line| line.starts_with("KO:"));
    if output.status.success() && rejection.is_none() {
        return Ok(());
    }
    let detail = rejection
        .or_else(|| stderr.lines().find(|line| !line.trim().is_empty()))
        .or_else(|| stdout.lines().find(|line| !line.trim().is_empty()));
    Err(format!(
        "Could not {action} {serial}: {}",
        detail.unwrap_or("adb command failed")
    ))
}

pub fn ios_boot(udid: &str) -> Result<(), String> {
    simctl(&["boot", udid])
}

pub fn ios_shutdown(udid: &str) -> Result<(), String> {
    simctl(&["shutdown", udid])
}

/// Bring up Simulator.app showing this device (for interacting with it).
pub fn ios_open(udid: &str) -> Result<(), String> {
    command("open")
        .args(["-a", "Simulator", "--args", "-CurrentDeviceUDID", udid])
        .spawn()
        .map(|_| ())
        .map_err(|e| e.to_string())
}

fn simctl(args: &[&str]) -> Result<(), String> {
    let out = command("xcrun")
        .arg("simctl")
        .args(args)
        .output()
        .map_err(|e| e.to_string())?;
    if out.status.success() {
        Ok(())
    } else {
        Err(String::from_utf8_lossy(&out.stderr).trim().to_string())
    }
}

#[derive(Debug)]
pub struct InstallFailure {
    pub code: Option<String>,
    pub detail: String,
}

impl InstallFailure {
    pub fn allows_downgrade(&self) -> bool {
        self.code.as_deref() == Some("INSTALL_FAILED_VERSION_DOWNGRADE")
    }
}

fn install_failure(detail: String) -> InstallFailure {
    // adb prints the APK path before the failure. Only accept a status code
    // from the failure itself, so a filename cannot trigger the downgrade retry.
    let code = detail.lines().find_map(|line| {
        let text = line
            .split_once("Failure [")
            .map_or(line.trim(), |(_, error)| error);
        let code = text
            .split(|ch: char| !ch.is_ascii_uppercase() && ch != '_' && !ch.is_ascii_digit())
            .next()?;
        (code.starts_with("INSTALL_FAILED_") && code.len() > "INSTALL_FAILED_".len())
            .then(|| code.to_owned())
    });
    InstallFailure { code, detail }
}

/// Install or replace an APK on one device.
pub fn install(sdk: &Sdk, serial: &str, apk: &Path, downgrade: bool) -> Result<(), InstallFailure> {
    let adb = sdk
        .adb
        .as_ref()
        .ok_or_else(|| install_failure("adb not found".into()))?;
    let mut cmd = command(adb);
    cmd.args(["-s", serial, "install", "-r"]);
    if downgrade {
        cmd.arg("-d");
    }
    let out = cmd
        .arg(apk)
        .output()
        .map_err(|err| install_failure(err.to_string()))?;
    let stdout = String::from_utf8_lossy(&out.stdout);
    if out.status.success() && stdout.lines().any(|line| line.trim() == "Success") {
        return Ok(());
    }
    let stderr = String::from_utf8_lossy(&out.stderr);
    let detail = format!("{}\n{}", stderr.trim(), stdout.trim());
    let detail = detail.trim();
    Err(install_failure(if detail.is_empty() {
        format!("adb install exited with {}", out.status)
    } else {
        detail.into()
    }))
}

/// Run `adb -s <serial> shell <cmd>`; returns stdout.
pub fn shell(sdk: &Sdk, serial: &str, cmd: &str) -> Result<String, String> {
    let adb = sdk.adb.as_ref().ok_or("adb not found")?;
    let out = command(adb)
        .args(["-s", serial, "shell", cmd])
        .output()
        .map_err(|e| e.to_string())?;
    Ok(String::from_utf8_lossy(&out.stdout).into_owned())
}

/// Rotate an emulator's screen 90°.
pub fn rotate(sdk: &Sdk, serial: &str) -> Result<(), String> {
    emulator_command(sdk, serial, "rotate")
}

/// Save a full-resolution screenshot to the Desktop (or home). Returns the path.
pub fn save_screenshot(sdk: &Sdk, device: &Device) -> Result<std::path::PathBuf, String> {
    let serial = device.serial().ok_or("device not running")?;
    let home = std::env::var_os("HOME")
        .or_else(|| std::env::var_os("USERPROFILE"))
        .map(std::path::PathBuf::from)
        .ok_or("no home folder")?;
    let dir = [home.join("Desktop"), home.clone()]
        .into_iter()
        .find(|d| d.is_dir())
        .unwrap_or(home);
    let stamp = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_secs())
        .unwrap_or(0);
    let safe: String = device
        .display_name
        .chars()
        .map(|c| if c.is_ascii_alphanumeric() { c } else { '_' })
        .collect();
    if let Some(port) = device.ios_stream_port {
        let path = dir.join(format!("emugrid-{safe}-{stamp}.jpg"));
        let bytes = mjpeg::screenshot(port).map_err(|error| format!("iOS stream: {error}"))?;
        std::fs::write(&path, bytes).map_err(|error| error.to_string())?;
        return Ok(path);
    }
    let path = dir.join(format!("emugrid-{safe}-{stamp}.png"));
    match device.platform {
        Platform::Android => {
            let adb = sdk.adb.as_ref().ok_or("adb not found")?;
            let out = command(adb)
                .args(["-s", serial, "exec-out", "screencap", "-p"])
                .output()
                .map_err(|e| e.to_string())?;
            if out.stdout.is_empty() {
                return Err("screencap returned nothing".into());
            }
            std::fs::write(&path, out.stdout).map_err(|e| e.to_string())?;
        }
        Platform::Ios => {
            let p = path.to_string_lossy().into_owned();
            simctl(&["io", serial, "screenshot", "--type=png", &p])?;
        }
    }
    Ok(path)
}

#[cfg(test)]
mod tests {
    use super::*;

    /// emugrid shows the screen itself, so emulators start without their
    /// own window unless asked for one.
    #[test]
    fn emulators_start_headless_unless_a_window_is_wanted() {
        assert_eq!(
            start_args("pixel", Boot::Normal),
            ["-avd", "pixel", "-no-window"]
        );
        assert_eq!(
            start_args("pixel", Boot::Cold),
            ["-avd", "pixel", "-no-window", "-no-snapshot-load"]
        );
        assert_eq!(
            start_args("pixel", Boot::Wipe),
            ["-avd", "pixel", "-no-window", "-wipe-data"]
        );
        assert_eq!(start_args("pixel", Boot::Window), ["-avd", "pixel"]);
    }

    #[test]
    fn parses_boot_props_with_empty_fields_and_crlf() {
        let props = parse_props("Pixel 8\r\n\r\n15\r\n35\r\n1\r\n");
        assert_eq!(props.model.as_deref(), Some("Pixel 8"));
        assert!(!props.qemu);
        assert_eq!(props.release.as_deref(), Some("15"));
        assert_eq!(props.api, Some(35));
        assert!(props.booted);
        assert!(!parse_props("Pixel\n1\n15\n35\n\n").booted);
    }

    #[test]
    fn extracts_install_failure_code_from_multiline_output() {
        let failure = install_failure("Performing Streamed Install\nadb: failed to install app-debug.apk: Failure [INSTALL_FAILED_VERSION_DOWNGRADE: version 5 is older than 10]".into());
        assert_eq!(
            failure.code.as_deref(),
            Some("INSTALL_FAILED_VERSION_DOWNGRADE")
        );
        assert!(failure.allows_downgrade());
        assert_eq!(
            install_failure("Failure [INSTALL_FAILED_INVALID_APK]".into())
                .code
                .as_deref(),
            Some("INSTALL_FAILED_INVALID_APK")
        );
        let misleading_path = install_failure("adb: failed to install INSTALL_FAILED_VERSION_DOWNGRADE.apk: Failure [INSTALL_FAILED_INVALID_APK]".into());
        assert_eq!(
            misleading_path.code.as_deref(),
            Some("INSTALL_FAILED_INVALID_APK")
        );
        assert!(!misleading_path.allows_downgrade());
        assert_eq!(install_failure("device offline".into()).code, None);
    }

    /// Runs against the local SDK; prints what the app would show.
    #[test]
    #[ignore = "needs a local Android SDK"]
    fn scan_local() {
        for d in scan(&Sdk::detect(None), &[]) {
            println!("{d:?}");
        }
    }
}
