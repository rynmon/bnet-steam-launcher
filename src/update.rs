//! Checks GitHub for a newer release and installs it. Only GitHub's public API and release files
//! are fetched; nothing about the user is sent.

use std::path::{Path, PathBuf};

use anyhow::{Context, Result, bail};
use sha2::{Digest as _, Sha256};

const LATEST: &str = "https://api.github.com/repos/rynmon/bnet-steam-launcher/releases/latest";

/// This platform's file in a release.
pub const ASSET: &str = if cfg!(windows) {
    "bnet-launcher-windows-x86_64.exe"
} else {
    "bnet-launcher-linux-x86_64"
};
/// The release file listing each file's SHA-256, as `sha256sum` writes it.
const SUMS: &str = "SHA256SUMS";

#[derive(Clone, Debug)]
pub struct Release {
    /// Without the "v", e.g. "1.1.0".
    pub version: String,
    /// The release's GitHub page.
    pub page: String,
    /// Download URLs of this platform's file and the checksums; both are needed to update from
    /// inside the app.
    downloads: Option<(String, String)>,
}

impl Release {
    pub fn can_install(&self) -> bool {
        self.downloads.is_some()
    }
}

/// A newer release than this build, if there is one.
pub fn newer_release() -> Result<Option<Release>> {
    let reply = crate::net::get_json(LATEST, &["Accept: application/vnd.github+json".into()])?;
    Ok(parse_release(&reply).filter(|r| is_newer(&r.version, env!("CARGO_PKG_VERSION"))))
}

fn parse_release(reply: &serde_json::Value) -> Option<Release> {
    let tag = reply["tag_name"].as_str()?;
    let page = reply["html_url"].as_str()?;
    let url_of = |name: &str| {
        reply["assets"].as_array()?.iter().find_map(|a| {
            (a["name"].as_str() == Some(name))
                .then(|| a["browser_download_url"].as_str().map(str::to_string))
                .flatten()
        })
    };
    Some(Release {
        version: tag.trim_start_matches('v').to_string(),
        page: page.to_string(),
        downloads: url_of(ASSET).zip(url_of(SUMS)),
    })
}

/// Downloads the release, checks it against its published SHA-256, and puts it in place of the
/// installed copy (the one the Steam shortcuts and the Start menu run). Returns that path.
pub fn install(release: &Release) -> Result<PathBuf> {
    let target = crate::setup::installed_exe_path()?;
    install_to(release, &target)?;
    // Only a convenience, so a failure here mustn't fail the update.
    let _ = crate::setup::install_menu_entry(&target);
    Ok(target)
}

fn install_to(release: &Release, target: &Path) -> Result<()> {
    let Some((asset_url, sums_url)) = &release.downloads else {
        bail!("this release can't be installed from the app: download it from its page");
    };
    let dir = target.parent().expect("has a parent");
    std::fs::create_dir_all(dir).with_context(|| format!("creating {}", dir.display()))?;

    let sums_file = target.with_extension("sums");
    crate::net::download(sums_url, &sums_file, &[])?;
    let sums = std::fs::read_to_string(&sums_file);
    let _ = std::fs::remove_file(&sums_file);
    let expected =
        expected_sum(&sums?, ASSET).with_context(|| format!("{SUMS} doesn't list {ASSET}"))?;

    let download = target.with_extension("download");
    crate::net::download(asset_url, &download, &[])?;
    let actual = sha256_file(&download);
    if actual.as_deref().ok() != Some(expected.as_str()) {
        let _ = std::fs::remove_file(&download);
        bail!("the download doesn't match its published checksum, so it wasn't used");
    }
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        std::fs::set_permissions(&download, std::fs::Permissions::from_mode(0o755))?;
    }
    crate::setup::replace_installed(&download, target)
}

/// The checksum listed for `name` in a `sha256sum` output (`<hex>  <name>`, or `<hex> *<name>`
/// in binary mode), lowercase.
fn expected_sum(sums: &str, name: &str) -> Option<String> {
    sums.lines().find_map(|line| {
        let (hash, file) = line.trim().split_once(char::is_whitespace)?;
        (file.trim_start().trim_start_matches('*') == name && hash.len() == 64)
            .then(|| hash.to_ascii_lowercase())
    })
}

fn sha256_file(path: &Path) -> Result<String> {
    Ok(Sha256::digest(std::fs::read(path)?)
        .iter()
        .map(|b| format!("{b:02x}"))
        .collect())
}

/// "1.2.0" > "1.1.9"; pre-release suffixes ("-beta.2") are ignored.
fn is_newer(candidate: &str, current: &str) -> bool {
    fn parts(v: &str) -> Vec<u64> {
        v.trim_start_matches('v')
            .split(['-', '+'])
            .next()
            .unwrap_or_default()
            .split('.')
            .map(|p| p.parse().unwrap_or(0))
            .collect()
    }
    let (c, cur) = (parts(candidate), parts(current));
    !candidate.is_empty() && c > cur
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn versions() {
        assert!(is_newer("v1.0.1", "1.0.0"));
        assert!(is_newer("1.10.0", "1.9.3"));
        assert!(!is_newer("v1.0.0", "1.0.0"));
        assert!(!is_newer("v0.9.0", "1.0.0"));
        assert!(!is_newer("", "1.0.0"));
    }

    #[test]
    fn checksums() {
        let a = "a".repeat(64);
        let b = "B".repeat(64);
        let sums =
            format!("{a}  bnet-launcher-linux-x86_64\n{b} *bnet-launcher-windows-x86_64.exe\n");
        assert_eq!(expected_sum(&sums, "bnet-launcher-linux-x86_64"), Some(a));
        assert_eq!(
            expected_sum(&sums, "bnet-launcher-windows-x86_64.exe"),
            Some(b.to_ascii_lowercase())
        );
        assert_eq!(expected_sum(&sums, "other"), None);
        assert_eq!(expected_sum("abc  other\n", "other"), None, "not a SHA-256");
    }

    #[test]
    fn hashes_files() {
        let path = std::env::temp_dir().join(format!("bnet-launcher-sha-{}", std::process::id()));
        std::fs::write(&path, b"abc").unwrap();
        assert_eq!(
            sha256_file(&path).unwrap(),
            "ba7816bf8f01cfea414140de5dae2223b00361a396177a9cb410ff61f20015ad"
        );
        std::fs::remove_file(path).unwrap();
    }

    /// Installs the real v1.0.0 release into a temporary folder, over a stand-in old copy, and
    /// checks a tampered checksum is refused: cargo test install_real_release -- --ignored
    #[test]
    #[ignore]
    fn install_real_release() {
        let reply = crate::net::get_json(
            "https://api.github.com/repos/rynmon/bnet-steam-launcher/releases/tags/v1.0.0",
            &[],
        )
        .unwrap();
        let release = parse_release(&reply).unwrap();
        assert!(release.can_install());
        let dir = std::env::temp_dir().join(format!("bnet-launcher-update-{}", std::process::id()));
        let target = dir.join("bnet-launcher.exe");
        std::fs::create_dir_all(&dir).unwrap();
        std::fs::write(&target, b"old version").unwrap();

        install_to(&release, &target).unwrap();
        let data = std::fs::read(&target).unwrap();
        assert!(data.len() > 1_000_000, "the real binary");
        assert_eq!(
            std::fs::read(target.with_extension("old")).unwrap(),
            b"old version",
            "the previous copy moved aside"
        );

        // A checksum that doesn't match: nothing is replaced, nothing left behind.
        let mut bad = release.clone();
        let (asset, _) = bad.downloads.clone().unwrap();
        let sums = dir.join("bad-sums");
        std::fs::write(&sums, format!("{}  {ASSET}\n", "0".repeat(64))).unwrap();
        let url = format!("file:///{}", sums.display().to_string().replace('\\', "/"));
        bad.downloads = Some((asset, url));
        let err = install_to(&bad, &target).unwrap_err();
        assert!(format!("{err:#}").contains("checksum"), "{err:#}");
        assert_eq!(std::fs::read(&target).unwrap(), data, "left alone");
        assert!(!target.with_extension("download").exists());
        std::fs::remove_dir_all(dir).unwrap();
    }

    #[test]
    fn release_files() {
        let reply = serde_json::json!({
            "tag_name": "v1.1.0",
            "html_url": "https://github.com/x/y/releases/tag/v1.1.0",
            "assets": [
                { "name": ASSET, "browser_download_url": "https://x/app" },
                { "name": "SHA256SUMS", "browser_download_url": "https://x/sums" },
            ],
        });
        let r = parse_release(&reply).unwrap();
        assert_eq!(r.version, "1.1.0");
        assert!(r.can_install());

        // An older release without checksums can only be downloaded by hand.
        let mut reply = reply;
        reply["assets"].as_array_mut().unwrap().pop();
        assert!(!parse_release(&reply).unwrap().can_install());
    }
}
