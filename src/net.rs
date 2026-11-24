//! Downloads, through the `curl` that ships with Windows 10+ and SteamOS (no HTTP library needed).

use std::io::Write as _;
use std::path::Path;
use std::process::{Command, Stdio};

use anyhow::{Context, Result, bail};

const USER_AGENT: &str = concat!("bnet-launcher/", env!("CARGO_PKG_VERSION"));

fn curl() -> Command {
    // System32's curl on Windows, so a different curl earlier on PATH can't change behaviour.
    let exe = if cfg!(windows) {
        std::env::var_os("SystemRoot")
            .map(|r| Path::new(&r).join("System32").join("curl.exe"))
            .filter(|p| p.is_file())
            .map_or_else(|| "curl.exe".into(), |p| p.into_os_string())
    } else {
        "curl".into()
    };
    let mut cmd = Command::new(exe);
    cmd.args([
        "--fail",
        "--silent",
        "--show-error",
        "--location",
        "--retry",
        "2",
        "--max-time",
        "300",
        "--user-agent",
        USER_AGENT,
    ]);
    #[cfg(windows)]
    {
        use std::os::windows::process::CommandExt;
        const CREATE_NO_WINDOW: u32 = 0x0800_0000;
        cmd.creation_flags(CREATE_NO_WINDOW);
    }
    cmd
}

/// Runs curl, passing extra request headers on stdin (`-H @-`) so secrets like an API key never
/// appear in the process list.
fn run(mut cmd: Command, headers: &[String]) -> Result<Vec<u8>> {
    if !headers.is_empty() {
        cmd.args(["--header", "@-"]);
    }
    let mut child = cmd
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .context("couldn't run curl")?;
    if let Some(mut stdin) = child.stdin.take() {
        for h in headers {
            writeln!(stdin, "{h}")?;
        }
    }
    let out = child.wait_with_output()?;
    if !out.status.success() {
        bail!("{}", String::from_utf8_lossy(&out.stderr).trim());
    }
    Ok(out.stdout)
}

/// Downloads `url` to `dest` (via a temporary file, so a failed download leaves nothing behind).
pub fn download(url: &str, dest: &Path, headers: &[String]) -> Result<()> {
    let tmp = dest.with_extension("part");
    let mut cmd = curl();
    cmd.arg("--output").arg(&tmp).arg(url);
    let result = run(cmd, headers);
    if let Err(e) = result {
        let _ = std::fs::remove_file(&tmp);
        return Err(e).with_context(|| format!("downloading {url}"));
    }
    std::fs::rename(&tmp, dest).with_context(|| format!("saving {}", dest.display()))
}

pub fn get_json(url: &str, headers: &[String]) -> Result<serde_json::Value> {
    let mut cmd = curl();
    cmd.arg(url);
    let body = run(cmd, headers).with_context(|| format!("requesting {url}"))?;
    serde_json::from_slice(&body).with_context(|| format!("reading the reply from {url}"))
}

/// Percent-encodes a URL path segment.
pub fn encode(s: &str) -> String {
    s.bytes()
        .map(|b| match b {
            b'A'..=b'Z' | b'a'..=b'z' | b'0'..=b'9' | b'-' | b'_' | b'.' | b'~' => {
                (b as char).to_string()
            }
            _ => format!("%{b:02X}"),
        })
        .collect()
}

#[cfg(test)]
mod tests {
    /// Downloads Battle.net's real installer (about 5 MB): cargo test -- --ignored
    #[test]
    #[ignore]
    fn downloads_bnet_installer() {
        let dest = std::env::temp_dir().join("bnet-launcher-test-Battle.net-Setup.exe");
        super::download(
            "https://www.battle.net/download/getInstallerForGame?os=win&gameProgram=BATTLENET_APP&version=Live",
            &dest,
            &[],
        )
        .unwrap();
        let data = std::fs::read(&dest).unwrap();
        assert!(
            data.len() > 1_000_000 && data.starts_with(b"MZ"),
            "a Windows program"
        );
        std::fs::remove_file(dest).unwrap();
    }

    /// SteamGridDB answers, and a bad key is reported as an error: cargo test -- --ignored
    #[test]
    #[ignore]
    fn steamgriddb_rejects_bad_key() {
        let err = super::get_json(
            "https://www.steamgriddb.com/api/v2/search/autocomplete/Diablo",
            &["Authorization: Bearer not-a-real-key".to_string()],
        )
        .unwrap_err();
        assert!(format!("{err:#}").contains("401"), "{err:#}");
    }

    #[test]
    fn encodes() {
        assert_eq!(super::encode("Diablo IV: x&y"), "Diablo%20IV%3A%20x%26y");
    }
}
