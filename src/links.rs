use std::collections::HashMap;

pub const CONF_PATH: &str = "/etc/interweave/links.conf";
const LINK_TYPES: [&str; 5] = ["ethernet", "wifi", "gsm", "cdma", "bt"];

#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Limits {
    pub weight: u32,
    pub down_mbit: u32,
    pub up_mbit: u32,
}

impl Default for Limits {
    fn default() -> Self {
        Limits { weight: 1, down_mbit: 0, up_mbit: 0 }
    }
}

/// Parses `nmcli -t -f DEVICE,TYPE device` into (device, type) for links that can reach the internet.
pub fn parse_devices(nmcli: &str) -> Vec<(String, String)> {
    nmcli
        .lines()
        .filter_map(|line| line.split_once(':'))
        .filter(|(_, kind)| LINK_TYPES.contains(kind))
        .map(|(dev, kind)| (dev.to_string(), kind.to_string()))
        .collect()
}

pub fn list_devices() -> Vec<(String, String)> {
    std::process::Command::new("nmcli")
        .args(["-t", "-f", "DEVICE,TYPE", "device"])
        .output()
        .map(|out| parse_devices(&String::from_utf8_lossy(&out.stdout)))
        .unwrap_or_default()
}

pub fn parse_config(text: &str) -> HashMap<String, Limits> {
    text.lines()
        .filter_map(|line| {
            let f: Vec<&str> = line.split_whitespace().collect();
            match f[..] {
                [dev, w, d, u] if !dev.starts_with('#') => Some((
                    dev.to_string(),
                    Limits { weight: w.parse().ok()?, down_mbit: d.parse().ok()?, up_mbit: u.parse().ok()? },
                )),
                _ => None,
            }
        })
        .collect()
}

pub fn read_config() -> HashMap<String, Limits> {
    parse_config(&std::fs::read_to_string(CONF_PATH).unwrap_or_default())
}

/// Everything the user set for one connection, kept in ~/.config/interweave/connections
/// so it survives turning combining off or quitting (which empties the root config).
#[derive(Clone, Default, Debug, PartialEq)]
pub struct LinkPrefs {
    pub name: String,
    pub hidden: bool,
    pub enabled: bool,
    pub limits: Limits,
}

pub fn parse_prefs(text: &str) -> HashMap<String, LinkPrefs> {
    text.lines()
        .filter_map(|line| {
            let f: Vec<&str> = line.splitn(7, ' ').collect();
            let [dev, hidden, enabled, weight, down, up, name] = f[..] else { return None };
            Some((
                dev.to_string(),
                LinkPrefs {
                    name: name.to_string(),
                    hidden: hidden == "1",
                    enabled: enabled == "1",
                    limits: Limits { weight: weight.parse().ok()?, down_mbit: down.parse().ok()?, up_mbit: up.parse().ok()? },
                },
            ))
        })
        .collect()
}

pub fn format_prefs(prefs: &HashMap<String, LinkPrefs>) -> String {
    let mut lines: Vec<String> = prefs
        .iter()
        .map(|(dev, p)| {
            let l = p.limits;
            let (hidden, enabled) = (u8::from(p.hidden), u8::from(p.enabled));
            format!("{dev} {hidden} {enabled} {} {} {} {}\n", l.weight, l.down_mbit, l.up_mbit, p.name)
        })
        .collect();
    lines.sort();
    lines.concat()
}

fn prefs_path() -> std::path::PathBuf {
    adw::glib::user_config_dir().join("interweave").join("connections")
}

/// User prefs, plus links from the active root config that the user file doesn't know yet (0.1.0 installs).
pub fn read_prefs() -> HashMap<String, LinkPrefs> {
    let mut prefs = parse_prefs(&std::fs::read_to_string(prefs_path()).unwrap_or_default());
    for (dev, limits) in read_config() {
        prefs.entry(dev).or_insert(LinkPrefs { enabled: true, limits, ..Default::default() });
    }
    prefs
}

pub fn write_prefs(prefs: &HashMap<String, LinkPrefs>) {
    let path = prefs_path();
    if let Some(dir) = path.parent() {
        let _ = std::fs::create_dir_all(dir);
    }
    let _ = std::fs::write(path, format_prefs(prefs));
}

pub fn format_config(links: &[(String, Limits)]) -> String {
    links
        .iter()
        .map(|(dev, l)| format!("{dev} {} {} {}\n", l.weight, l.down_mbit, l.up_mbit))
        .collect()
}

/// Total (rx, tx) bytes of a device since boot.
pub fn counters(dev: &str) -> Option<(u64, u64)> {
    let read = |name: &str| {
        std::fs::read_to_string(format!("/sys/class/net/{dev}/statistics/{name}")).ok()?.trim().parse().ok()
    };
    Some((read("rx_bytes")?, read("tx_bytes")?))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn keeps_only_internet_links() {
        let out = "eno1:ethernet\nlo:loopback\ndocker0:bridge\nwlp11s0:wifi\np2p-dev-wlp11s0:wifi-p2p\nwwan0:gsm\n";
        let devs: Vec<String> = parse_devices(out).into_iter().map(|(d, _)| d).collect();
        assert_eq!(devs, ["eno1", "wlp11s0", "wwan0"]);
    }

    #[test]
    fn prefs_round_trip() {
        let mut prefs = HashMap::new();
        let fiber = LinkPrefs { name: "Fiber at home".into(), enabled: true, ..Default::default() };
        let limits = Limits { weight: 3, down_mbit: 50, up_mbit: 10 };
        prefs.insert("eno1".to_string(), fiber);
        prefs.insert("wlp11s0".to_string(), LinkPrefs { hidden: true, limits, ..Default::default() });
        let text = format_prefs(&prefs);
        assert_eq!(text, "eno1 0 1 1 0 0 Fiber at home\nwlp11s0 1 0 3 50 10 \n");
        assert_eq!(parse_prefs(&format!("{text}broken\n")), prefs);
    }

    #[test]
    fn config_round_trip() {
        let links = vec![
            ("eno1".to_string(), Limits { weight: 2, down_mbit: 0, up_mbit: 0 }),
            ("wlp11s0".to_string(), Limits { weight: 1, down_mbit: 50, up_mbit: 10 }),
        ];
        let text = format_config(&links);
        assert_eq!(text, "eno1 2 0 0\nwlp11s0 1 50 10\n");
        let parsed = parse_config(&format!("# comment\n\n{text}broken line\n"));
        assert_eq!(parsed.len(), 2);
        assert_eq!(parsed["wlp11s0"], links[1].1);
    }
}
