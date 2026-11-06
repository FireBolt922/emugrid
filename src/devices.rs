//! Discovering emulators and devices, and starting / stopping them.

use crate::ios;
use crate::sdk::{Sdk, command};
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
    Running { serial: String },
    /// Seen by adb but not ready yet (booting, offline, unauthorized...).
    Other { serial: String, status: String },
}

#[derive(Debug, Clone)]
pub struct Device {
    /// AVD name for emulators, model/serial for physical devices.
    pub name: String,
    pub platform: Platform,
    pub is_emulator: bool,
    pub state: State,
    /// AVD name, if this is one of the SDK's AVDs (enables start / cold boot / wipe).
    pub avd: Option<String>,
    /// iOS: runtime ("iOS 18.2") and UDID.
    pub runtime: Option<String>,
    pub udid: Option<String>,
}

impl Device {
    pub fn serial(&self) -> Option<&str> {
        match &self.state {
            State::Running { serial } => Some(serial),
            _ => None,
        }
    }
}

fn android(name: String, is_emulator: bool, state: State, avd: Option<String>) -> Device {
    Device {
        name,
        platform: Platform::Android,
        is_emulator,
        state,
        avd,
        runtime: None,
        udid: None,
    }
}

/// One full scan: installed AVDs merged with whatever adb currently sees.
pub fn scan(sdk: &Sdk) -> Vec<Device> {
    let mut by_name: BTreeMap<String, Device> = BTreeMap::new();

    for avd in list_avds(sdk) {
        by_name.insert(
            avd.clone(),
            android(avd.clone(), true, State::Stopped, Some(avd)),
        );
    }

    for (serial, status) in adb_devices(sdk) {
        let ready = status == "device";
        // Emulators started by the SDK use `emulator-NNNN`; ones started by other
        // tools (e.g. Orca) connect over TCP, so ask the device itself.
        let (model, qemu) = if ready {
            props(sdk, &serial)
        } else {
            (None, false)
        };
        let is_emulator = serial.starts_with("emulator-") || qemu;
        let avd = (ready && serial.starts_with("emulator-"))
            .then(|| avd_name(sdk, &serial))
            .flatten();
        let state = if ready {
            State::Running {
                serial: serial.clone(),
            }
        } else {
            State::Other {
                serial: serial.clone(),
                status,
            }
        };
        match avd {
            // Known AVD: replace its "stopped" entry.
            Some(name) => {
                by_name.insert(
                    name.clone(),
                    android(name.clone(), is_emulator, state, Some(name)),
                );
            }
            // Anything else: keyed by serial so identical models don't collide.
            None => {
                let name = model.unwrap_or_else(|| serial.clone());
                by_name.insert(
                    format!("~{serial}"),
                    android(name, is_emulator, state, None),
                );
            }
        }
    }

    let mut list: Vec<Device> = by_name.into_values().collect();
    if sdk.simctl {
        list.extend(ios_simulators());
    }
    // Running first, then alphabetical.
    list.sort_by_key(|d| (matches!(d.state, State::Stopped), d.name.to_lowercase()));
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
        .filter(|s| !s.is_empty())
}

/// Model name and whether the device is an emulator (QEMU), in one adb call.
fn props(sdk: &Sdk, serial: &str) -> (Option<String>, bool) {
    let Some(adb) = &sdk.adb else {
        return (None, false);
    };
    let out = run(
        adb,
        &[
            "-s",
            serial,
            "shell",
            "getprop ro.product.model; getprop ro.kernel.qemu",
        ],
    )
    .unwrap_or_default();
    let mut lines = out.lines().map(str::trim);
    let model = lines.next().filter(|s| !s.is_empty()).map(String::from);
    (model, lines.next() == Some("1"))
}

fn ios_simulators() -> Vec<Device> {
    let out = run("xcrun", &["simctl", "list", "devices", "available"]).unwrap_or_default();
    ios::parse_list(&out)
        .into_iter()
        // Watch / TV / Vision simulators have no phone-like screen worth gridding.
        .filter(|s| s.runtime.starts_with("iOS"))
        .map(|s| Device {
            name: s.name,
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
            runtime: Some(s.runtime),
            udid: Some(s.udid),
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
}

/// Launch an AVD in the background; the emulator outlives this app.
pub fn start(sdk: &Sdk, avd: &str, boot: Boot) -> Result<(), String> {
    let emu = sdk.emulator.as_ref().ok_or("emulator binary not found")?;
    let mut cmd = command(emu);
    cmd.args(["-avd", avd]);
    match boot {
        Boot::Normal => {}
        Boot::Cold => {
            cmd.arg("-no-snapshot-load");
        }
        Boot::Wipe => {
            cmd.arg("-wipe-data");
        }
    }
    cmd.stdout(Stdio::null())
        .stderr(Stdio::null())
        .spawn()
        .map(|_| ())
        .map_err(|e| e.to_string())
}

/// Shut down a running emulator.
pub fn stop(sdk: &Sdk, serial: &str) -> Result<(), String> {
    let adb = sdk.adb.as_ref().ok_or("adb not found")?;
    command(adb)
        .args(["-s", serial, "emu", "kill"])
        .output()
        .map(|_| ())
        .map_err(|e| e.to_string())
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

/// `adb install -r` an APK on one device.
pub fn install(sdk: &Sdk, serial: &str, apk: &Path) -> Result<(), String> {
    let adb = sdk.adb.as_ref().ok_or("adb not found")?;
    let out = command(adb)
        .args(["-s", serial, "install", "-r"])
        .arg(apk)
        .output()
        .map_err(|e| e.to_string())?;
    let text = String::from_utf8_lossy(&out.stdout);
    if out.status.success() && text.contains("Success") {
        Ok(())
    } else {
        let err = String::from_utf8_lossy(&out.stderr);
        let line = err
            .lines()
            .chain(text.lines())
            .find(|l| !l.trim().is_empty());
        Err(line.unwrap_or("install failed").trim().to_string())
    }
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
    let adb = sdk.adb.as_ref().ok_or("adb not found")?;
    command(adb)
        .args(["-s", serial, "emu", "rotate"])
        .output()
        .map(|_| ())
        .map_err(|e| e.to_string())
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
        .name
        .chars()
        .map(|c| if c.is_ascii_alphanumeric() { c } else { '_' })
        .collect();
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

    /// Runs against the local SDK; prints what the app would show.
    #[test]
    #[ignore = "needs a local Android SDK"]
    fn scan_local() {
        for d in scan(&Sdk::detect()) {
            println!("{d:?}");
        }
    }
}
