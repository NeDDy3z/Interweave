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
