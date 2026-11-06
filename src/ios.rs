//! iOS simulators via `xcrun simctl` (macOS with Xcode only). View-only:
//! simctl can boot, shut down and screenshot, but cannot inject touches.

#[derive(Debug, Clone, PartialEq)]
pub struct Simulator {
    pub name: String,
    pub udid: String,
    pub runtime: String,
    pub booted: bool,
}

/// Parse the text output of `xcrun simctl list devices available`.
pub fn parse_list(out: &str) -> Vec<Simulator> {
    let mut sims = Vec::new();
    let mut runtime: Option<String> = None;
    for line in out.lines().map(str::trim) {
        if let Some(section) = line.strip_prefix("-- ").and_then(|l| l.strip_suffix(" --")) {
            runtime = (!section.starts_with("Unavailable")).then(|| section.to_string());
            continue;
        }
        let Some(runtime) = &runtime else { continue };
        // "<name> (<udid>) (<state>)"; names may contain parentheses too.
        let Some((rest, state)) = line.strip_suffix(')').and_then(|l| l.rsplit_once(" (")) else {
            continue;
        };
        let Some((name, udid)) = rest.strip_suffix(')').and_then(|l| l.rsplit_once(" (")) else {
            continue;
        };
        sims.push(Simulator {
            name: name.to_string(),
            udid: udid.to_string(),
            runtime: runtime.clone(),
            booted: state == "Booted",
        });
    }
    sims
}

#[cfg(test)]
mod tests {
    use super::*;

    const SAMPLE: &str = "== Devices ==
-- iOS 18.2 --
    iPhone 16 Pro (5A0E1B2C-1111-2222-3333-444455556666) (Booted) 
    iPad Air 11-inch (M2) (AAAA1B2C-1111-2222-3333-444455556666) (Shutdown) 
-- watchOS 11.2 --
    Apple Watch Series 10 (46mm) (BBBB1B2C-1111-2222-3333-444455556666) (Shutdown) 
-- Unavailable: com.apple.CoreSimulator.SimRuntime.iOS-16-0 --
    iPhone 8 (CCCC1B2C-1111-2222-3333-444455556666) (Shutdown) (unavailable, runtime profile not found)
";

    #[test]
    fn parses_names_with_parentheses_and_skips_unavailable() {
        let sims = parse_list(SAMPLE);
        assert_eq!(sims.len(), 3);
        assert_eq!(
            sims[0],
            Simulator {
                name: "iPhone 16 Pro".into(),
                udid: "5A0E1B2C-1111-2222-3333-444455556666".into(),
                runtime: "iOS 18.2".into(),
                booted: true,
            }
        );
        assert_eq!(sims[1].name, "iPad Air 11-inch (M2)");
        assert!(!sims[1].booted);
        assert_eq!(sims[2].runtime, "watchOS 11.2");
    }

    #[test]
    fn empty_output() {
        assert!(parse_list("").is_empty());
    }
}
