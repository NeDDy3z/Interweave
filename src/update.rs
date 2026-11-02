use adw::gio;
use std::ffi::OsStr;

const LATEST_URL: &str = "https://api.github.com/repos/NeDDy3z/Interweave/releases/latest";

pub async fn latest_tag() -> Option<String> {
    let argv = ["curl", "-fsSL", "--max-time", "10", LATEST_URL].map(OsStr::new);
    let proc = gio::Subprocess::newv(&argv, gio::SubprocessFlags::STDOUT_PIPE | gio::SubprocessFlags::STDERR_SILENCE).ok()?;
    let (out, _) = proc.communicate_utf8_future(None).await.ok()?;
    tag_name(&out?)
}

// shortcut: string search instead of a JSON parser, fine for GitHub's flat release object
fn tag_name(json: &str) -> Option<String> {
    let rest = &json[json.find("\"tag_name\"")? + "\"tag_name\"".len()..];
    let rest = &rest[rest.find('"')? + 1..];
    Some(rest[..rest.find('"')?].to_string())
}

pub fn is_newer(tag: &str, current: &str) -> bool {
    let parse = |v: &str| -> Vec<u32> { v.trim_start_matches('v').split('.').map(|p| p.parse().unwrap_or(0)).collect() };
    parse(tag) > parse(current)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn reads_tag_and_compares_versions() {
        let json = r#"{"url": "x", "tag_name": "v0.10.0", "name": "Interweave 0.10.0"}"#;
        assert_eq!(tag_name(json).as_deref(), Some("v0.10.0"));
        assert!(is_newer("v0.10.0", "0.9.1"));
        assert!(!is_newer("v0.1.0", "0.1.0"));
        assert!(tag_name("{}").is_none());
    }
}
