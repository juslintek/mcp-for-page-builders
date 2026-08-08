use std::path::PathBuf;

pub fn urlencode(s: &str) -> String {
    s.bytes()
        .map(|b| match b {
            b'A'..=b'Z' | b'a'..=b'z' | b'0'..=b'9' | b'-' | b'_' | b'.' | b'~' => {
                String::from(b as char)
            }
            _ => format!("%{b:02X}"),
        })
        .collect()
}

pub fn urldecode(s: &str) -> String {
    let mut out = String::new();
    let mut bytes = s.bytes();
    while let Some(b) = bytes.next() {
        match b {
            b'%' => {
                let h = bytes.next().unwrap_or(0);
                let l = bytes.next().unwrap_or(0);
                if let Ok(v) =
                    u8::from_str_radix(&String::from_utf8(vec![h, l]).unwrap_or_default(), 16)
                {
                    out.push(v as char);
                }
            }
            b'+' => out.push(' '),
            _ => out.push(b as char),
        }
    }
    out
}

pub fn uuid() -> String {
    use rand::Rng;
    let mut r = rand::rng();
    format!(
        "{:08x}-{:04x}-{:04x}-{:04x}-{:012x}",
        r.random::<u32>(),
        r.random::<u16>(),
        r.random::<u16>(),
        r.random::<u16>(),
        r.random::<u64>() & 0xFFFF_FFFF_FFFF
    )
}

pub fn config_path(wp_url: &str) -> PathBuf {
    let host = wp_url
        .trim_end_matches('/')
        .replace("https://", "")
        .replace("http://", "")
        .replace(['/', ':', '.'], "_");
    config_dir().join(format!("{host}.json"))
}

pub fn config_dir() -> PathBuf {
    std::env::var("HOME").map_or_else(|_| PathBuf::from("."), PathBuf::from)
        .join(".config")
        .join("mcp-for-page-builders")
}

/// Directory for this server's own log files, independent of stdout/stderr.
///
/// Defaults to `~/.config/mcp-for-page-builders/logs/`, overridable via
/// `MCP_LOG_DIR` (used by tests to isolate concurrent server instances
/// from sharing — and racing on — the same log directory).
pub fn log_dir() -> PathBuf {
    std::env::var("MCP_LOG_DIR")
        .map_or_else(|_| config_dir().join("logs"), PathBuf::from)
}

/// Returns the project root if running from a local dev build (target/release or target/debug).
pub fn dev_project_root() -> Option<PathBuf> {
    let exe = std::env::current_exe().ok()?;
    let path = exe.to_str()?;
    // Match .../target/release/binary or .../target/debug/binary
    path.find("/target/release/")
        .or_else(|| path.find("/target/debug/"))
        .map(|idx| PathBuf::from(&path[..idx]))
}
