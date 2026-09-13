use std::path::Path;
use std::process::Command;

use anyhow::{Context, Result, anyhow, bail};
use console::Term;
use self_update::backends::github;
use self_update::{Checksum, Release, ReleaseAsset};
use semver::Version;

const REPOSITORY_OWNER: &str = "HITSZ-WTRobot-Packages";
const REPOSITORY_NAME: &str = "cpkg";
const SUPPORTED_TARGETS: [&str; 6] = [
    "x86_64-unknown-linux-gnu",
    "aarch64-unknown-linux-gnu",
    "x86_64-apple-darwin",
    "aarch64-apple-darwin",
    "x86_64-pc-windows-msvc",
    "aarch64-pc-windows-msvc",
];

#[derive(Clone, Debug, Eq, Ord, PartialEq, PartialOrd)]
struct ReleaseVersion {
    base: Version,
    fix_revision: u64,
}

pub fn update() -> Result<()> {
    let target = self_update::get_target();
    ensure_supported_target(target)?;

    let current_tag = current_release_tag();
    let current_version = parse_release_tag(&current_tag)?;
    let current_bare = current_tag
        .strip_prefix('v')
        .expect("parse_release_tag accepted the current tag")
        .to_owned();

    update_from_github(target, &current_tag, &current_bare, &current_version)
        .context("failed to update cpkg from GitHub Releases")
}

fn update_from_github(
    target: &str,
    current_tag: &str,
    current_bare: &str,
    current_version: &ReleaseVersion,
) -> Result<()> {
    let releases = github::Update::configure()
        .repo_owner(REPOSITORY_OWNER)
        .repo_name(REPOSITORY_NAME)
        .tag_prefix("v")
        .target(target)
        .bin_name("cpkg")
        .current_version(current_bare)
        .build()?
        .get_latest_release()?;
    let latest = releases
        .latest()
        .ok_or_else(|| anyhow!("no cpkg GitHub release was found"))?;
    let latest_version = parse_release_version(latest.version())?;

    if latest_version <= *current_version {
        Term::stdout().write_line(&format!("cpkg is already up to date ({current_tag})"))?;
        return Ok(());
    }

    let asset_name = archive_name(latest.version(), target);
    let asset = select_release_asset(latest, &asset_name)?;
    let asset_digest = asset
        .digest()
        .expect("select_release_asset requires a digest")
        .to_owned();
    let checksum = Checksum::parse_digest(&asset_digest)?;
    let expected_name = asset.name().to_owned();
    let expected_digest = asset_digest.clone();
    let expected_base_version = latest_version.base.to_string();

    github::Update::configure()
        .repo_owner(REPOSITORY_OWNER)
        .repo_name(REPOSITORY_NAME)
        .tag_prefix("v")
        .target(target)
        .bin_name("cpkg")
        .current_version(current_bare)
        .release_tag(format!("v{}", latest.version()))
        .asset_matcher(move |assets| exact_asset(assets, &expected_name, &expected_digest))
        .bin_path_in_archive(binary_path_in_archive(target))
        .verify_checksum(checksum)
        .verify_release_digest(true)
        .no_confirm(true)
        .show_output(false)
        .show_download_progress(true)
        .check_install_path_writable(true)
        .verify_binary(move |path| verify_staged_binary(path, &expected_base_version))
        .build()?
        .update()?;

    Term::stdout().write_line(&format!(
        "Updated cpkg from {current_tag} to v{}",
        latest.version()
    ))?;
    Ok(())
}

fn current_release_tag() -> String {
    match option_env!("CPKG_RELEASE_TAG") {
        Some(tag) if !tag.is_empty() => tag.to_owned(),
        _ => format!("v{}", env!("CARGO_PKG_VERSION")),
    }
}

fn ensure_supported_target(target: &str) -> Result<()> {
    if SUPPORTED_TARGETS.contains(&target) {
        Ok(())
    } else {
        bail!("cpkg update is not supported for target '{target}'")
    }
}

fn parse_release_tag(tag: &str) -> Result<ReleaseVersion> {
    let bare = tag
        .strip_prefix('v')
        .ok_or_else(|| anyhow!("invalid cpkg release tag '{tag}'"))?;
    parse_version_identity(bare).map_err(|_| anyhow!("invalid cpkg release tag '{tag}'"))
}

fn parse_release_version(version: &str) -> Result<ReleaseVersion> {
    if version.starts_with('v') {
        bail!("invalid cpkg release version '{version}'")
    }
    parse_version_identity(version).map_err(|_| anyhow!("invalid cpkg release version '{version}'"))
}

fn parse_version_identity(value: &str) -> std::result::Result<ReleaseVersion, ()> {
    let (base_text, fix_revision) = match value.split_once("-fix") {
        Some((base, revision))
            if !revision.is_empty() && revision.bytes().all(|byte| byte.is_ascii_digit()) =>
        {
            (base, revision.parse::<u64>().map_err(|_| ())?)
        }
        Some(_) => return Err(()),
        None => (value, 0),
    };

    let base = Version::parse(base_text).map_err(|_| ())?;
    if !base.pre.is_empty() || !base.build.is_empty() || base.to_string() != base_text {
        return Err(());
    }

    Ok(ReleaseVersion { base, fix_revision })
}

fn archive_name(version: &str, target: &str) -> String {
    let extension = if target.ends_with("-windows-msvc") {
        "zip"
    } else {
        "tar.gz"
    };
    format!("cpkg-v{version}-{target}.{extension}")
}

fn binary_path_in_archive(target: &str) -> &'static str {
    if target.ends_with("-windows-msvc") {
        "cpkg.exe"
    } else {
        "cpkg-v{{ version }}-{{ target }}/cpkg"
    }
}

fn select_release_asset<'a>(release: &'a Release, expected_name: &str) -> Result<&'a ReleaseAsset> {
    let mut matching = release
        .assets()
        .iter()
        .filter(|asset| asset.name() == expected_name);
    let asset = matching
        .next()
        .ok_or_else(|| anyhow!("release does not contain the required asset '{expected_name}'"))?;
    if matching.next().is_some() {
        bail!("release contains more than one asset named '{expected_name}'")
    }

    let digest = asset
        .digest()
        .ok_or_else(|| anyhow!("release asset '{expected_name}' has no digest"))?;
    if !is_sha256_digest(digest) {
        bail!("release asset '{expected_name}' does not have a valid SHA-256 digest: '{digest}'");
    }
    Ok(asset)
}

fn is_sha256_digest(digest: &str) -> bool {
    let Some((algorithm, hex)) = digest.trim().split_once(':') else {
        return false;
    };
    algorithm.eq_ignore_ascii_case("sha256")
        && hex.len() == 64
        && hex.bytes().all(|byte| byte.is_ascii_hexdigit())
}

fn exact_asset(
    assets: &[ReleaseAsset],
    expected_name: &str,
    expected_digest: &str,
) -> Option<ReleaseAsset> {
    let mut matching = assets
        .iter()
        .filter(|asset| asset.name() == expected_name && asset.digest() == Some(expected_digest));
    let asset = matching.next()?.clone();
    if matching.next().is_some() {
        None
    } else {
        Some(asset)
    }
}

fn verify_staged_binary(path: &Path, expected_base_version: &str) -> self_update::Result<()> {
    let output = Command::new(path)
        .arg("--version")
        .output()
        .map_err(|error| {
            self_update::Error::verification_rejected(format!(
                "failed to execute staged cpkg binary '{}': {error}",
                path.display()
            ))
        })?;
    validate_version_probe(
        output.status.success(),
        &output.stdout,
        expected_base_version,
    )
}

fn validate_version_probe(
    success: bool,
    stdout: &[u8],
    expected_base_version: &str,
) -> self_update::Result<()> {
    if !success {
        return Err(self_update::Error::verification_rejected(
            "staged cpkg --version exited unsuccessfully",
        ));
    }

    let actual = std::str::from_utf8(stdout).map_err(|_| {
        self_update::Error::verification_rejected(
            "staged cpkg --version output was not valid UTF-8",
        )
    })?;
    let expected = format!("cpkg {expected_base_version}");
    if actual.trim() != expected {
        return Err(self_update::Error::verification_rejected(format!(
            "staged cpkg --version output was '{}', expected '{expected}'",
            actual.trim()
        )));
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    const SHA256: &str = "sha256:0123456789abcdef0123456789abcdef0123456789abcdef0123456789abcdef";

    fn asset(name: &str, digest: Option<&str>) -> ReleaseAsset {
        let asset = ReleaseAsset::new(name, "https://example.invalid/download");
        match digest {
            Some(digest) => asset.with_digest(digest),
            None => asset,
        }
    }

    fn release(version: &str, assets: Vec<ReleaseAsset>) -> Release {
        Release::builder()
            .name("cpkg")
            .version(version)
            .date("2026-01-01")
            .assets(assets)
            .build()
            .unwrap()
    }

    #[test]
    fn release_identity_orders_base_versions_and_fix_revisions() {
        assert!(parse_release_tag("v1.2.4").unwrap() > parse_release_tag("v1.2.3-fix9").unwrap());
        assert!(
            parse_release_tag("v1.2.3-fix2").unwrap() > parse_release_tag("v1.2.3-fix1").unwrap()
        );
        assert_eq!(
            parse_release_tag("v1.2.3").unwrap(),
            parse_release_version("1.2.3").unwrap()
        );
        assert_eq!(
            parse_release_tag("v1.2.3-fix1").unwrap(),
            parse_release_version("1.2.3-fix1").unwrap()
        );
    }

    #[test]
    fn release_identity_rejects_noncanonical_tags_and_versions() {
        for tag in [
            "1.2.3",
            "v1.2",
            "v1.2.3-rc1",
            "v1.2.3+meta",
            "v1.2.3-fix",
            "v1.2.3-fixx",
        ] {
            assert!(parse_release_tag(tag).is_err(), "accepted {tag}");
        }
        for version in ["v1.2.3", "1.2", "1.2.3-rc1", "1.2.3+meta", "1.2.3-fix"] {
            assert!(
                parse_release_version(version).is_err(),
                "accepted {version}"
            );
        }
    }

    #[test]
    fn supports_exact_release_matrix() {
        for target in SUPPORTED_TARGETS {
            assert!(ensure_supported_target(target).is_ok());
        }
        assert!(ensure_supported_target("x86_64-unknown-linux-musl").is_err());
    }

    #[test]
    fn builds_exact_asset_names_and_archive_paths() {
        for target in &SUPPORTED_TARGETS[..4] {
            assert_eq!(
                archive_name("1.2.3-fix1", target),
                format!("cpkg-v1.2.3-fix1-{target}.tar.gz")
            );
            assert_eq!(
                binary_path_in_archive(target),
                "cpkg-v{{ version }}-{{ target }}/cpkg"
            );
        }
        for target in &SUPPORTED_TARGETS[4..] {
            assert_eq!(
                archive_name("1.2.3-fix1", target),
                format!("cpkg-v1.2.3-fix1-{target}.zip")
            );
            assert_eq!(binary_path_in_archive(target), "cpkg.exe");
        }
    }

    #[test]
    fn release_asset_requires_one_exact_name_with_sha256_digest() {
        let name = "cpkg-v1.2.3-x86_64-unknown-linux-gnu.tar.gz";
        let good = release("1.2.3", vec![asset(name, Some(SHA256))]);
        assert_eq!(select_release_asset(&good, name).unwrap().name(), name);

        let wrong_name = release("1.2.3", vec![asset("other.tar.gz", Some(SHA256))]);
        assert!(select_release_asset(&wrong_name, name).is_err());
        let missing_digest = release("1.2.3", vec![asset(name, None)]);
        assert!(select_release_asset(&missing_digest, name).is_err());
        let wrong_digest = release("1.2.3", vec![asset(name, Some("sha512:abc"))]);
        assert!(select_release_asset(&wrong_digest, name).is_err());
        let duplicate = release(
            "1.2.3",
            vec![asset(name, Some(SHA256)), asset(name, Some(SHA256))],
        );
        assert!(select_release_asset(&duplicate, name).is_err());
    }

    #[test]
    fn pinned_matcher_requires_unique_name_and_digest() {
        let wanted = asset("cpkg.zip", Some(SHA256));
        let other_digest =
            "sha256:ffffffffffffffffffffffffffffffffffffffffffffffffffffffffffffffff";
        assert!(exact_asset(&[wanted.clone()], "cpkg.zip", SHA256).is_some());
        assert!(exact_asset(&[wanted.clone()], "other.zip", SHA256).is_none());
        assert!(exact_asset(&[wanted.clone()], "cpkg.zip", other_digest).is_none());
        assert!(exact_asset(&[wanted.clone(), wanted], "cpkg.zip", SHA256).is_none());
    }

    #[test]
    fn version_probe_accepts_only_successful_exact_output() {
        assert!(validate_version_probe(true, b"cpkg 1.2.3\n", "1.2.3").is_ok());
        assert!(validate_version_probe(false, b"cpkg 1.2.3\n", "1.2.3").is_err());
        assert!(validate_version_probe(true, b"cpkg 1.2.4\n", "1.2.3").is_err());
        assert!(validate_version_probe(true, &[0xff], "1.2.3").is_err());
    }

    #[test]
    fn release_asset_rejects_malformed_sha256_digest() {
        let name = "cpkg-v1.2.3-x86_64-unknown-linux-gnu.tar.gz";
        for digest in ["sha256:abc", "sha256:zzzzzzzz", "sha512:0123456789abcdef"] {
            let candidate = release("1.2.3", vec![asset(name, Some(digest))]);
            assert!(
                select_release_asset(&candidate, name).is_err(),
                "accepted {digest}"
            );
        }
    }

    #[test]
    fn staged_verifier_rejects_a_missing_binary() {
        let missing =
            std::env::temp_dir().join(format!("cpkg-missing-staged-binary-{}", std::process::id()));
        assert!(verify_staged_binary(&missing, "1.2.3").is_err());
    }
}
