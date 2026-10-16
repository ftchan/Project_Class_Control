//! Canonical on-disk locations for config and logs.

use std::path::PathBuf;

/// Program data directory, e.g. `C:\ProgramData\LabCtl` on Windows.
/// Falls back to a per-user data dir on other platforms (for dev builds).
pub fn data_dir() -> PathBuf {
    #[cfg(windows)]
    {
        if let Ok(pd) = std::env::var("ProgramData") {
            return PathBuf::from(pd).join("LabCtl");
        }
    }
    directories::ProjectDirs::from("org", "Lab", "LabCtl")
        .map(|d| d.data_dir().to_path_buf())
        .unwrap_or_else(|| PathBuf::from("./labctl-data"))
}

/// Path to the persisted policy file.
pub fn policy_file() -> PathBuf {
    data_dir().join("policy.json")
}

/// Drop a leading UTF-8 byte-order mark. Windows PowerShell 5.1 writes one
/// with `Set-Content -Encoding UTF8`, and serde_json rejects it.
pub fn strip_bom(bytes: &[u8]) -> &[u8] {
    bytes.strip_prefix(b"\xEF\xBB\xBF").unwrap_or(bytes)
}

#[cfg(test)]
mod tests {
    use super::strip_bom;

    #[test]
    fn strips_utf8_bom_only() {
        assert_eq!(strip_bom(b"\xEF\xBB\xBF{}"), b"{}");
        assert_eq!(strip_bom(b"{}"), b"{}");
    }
}

/// Directory for rotating log files.
pub fn log_dir() -> PathBuf {
    data_dir().join("logs")
}
