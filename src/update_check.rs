use std::ffi::OsStr;
use std::fs;
use std::io::{self, IsTerminal, Write};
use std::path::Path;
use std::process::{Command, Stdio};
use std::time::{SystemTime, UNIX_EPOCH};

use anyhow::{Result, bail};

use crate::config::{cpkg_home_dir, load_global_config};
use crate::update::{current_release_tag, latest_release_tag, release_is_newer};

const WORKER_ARGUMENT: &str = "__check-update";
const ATTEMPT_FILENAME: &str = "update-check-attempt";
const RELEASE_FILENAME: &str = "update-check-release";
const NOTIFIED_FILENAME: &str = "update-check-notified";
const CHECK_INTERVAL: u64 = 24 * 60 * 60;

/// Optionally show a cached notice and start a metadata-only background check.
/// Command eligibility is determined by the caller; redirected stderr never participates.
pub fn after_command() {
    if !io::stderr().is_terminal() {
        return;
    }
    let _ = (|| -> Result<()> {
        let home = checked_home()?;
        let enabled = load_global_config()?.update_check.enabled;
        after_command_at(
            &home,
            enabled,
            epoch_seconds()?,
            &current_release_tag(),
            |tag| {
                writeln!(
                    io::stderr().lock(),
                    "A new cpkg release is available: {tag}. Run `cpkg update` to install it."
                )?;
                Ok(())
            },
            spawn_worker,
        )
    })();
}

/// Handle only the exact private worker invocation, before normal CLI parsing.
/// Worker errors intentionally never reach normal command output or exit status.
pub fn run_if_worker() -> bool {
    let mut args = std::env::args_os();
    if args.next().is_none()
        || args.next().as_deref() != Some(OsStr::new(WORKER_ARGUMENT))
        || args.next().is_some()
    {
        return false;
    }
    let _ = (|| -> Result<()> {
        let home = checked_home()?;
        // Re-read preferences in the child: an opt-out between spawn and execution wins.
        let enabled = load_global_config()?.update_check.enabled;
        check_at(&home, enabled, epoch_seconds()?, latest_release_tag)
    })();
    true
}

fn checked_home() -> Result<std::path::PathBuf> {
    let home = cpkg_home_dir()?;
    // A relative HOME must not redirect optional cache writes into the current project.
    if !home.is_absolute() || !home.parent().is_some_and(Path::is_dir) {
        bail!("invalid cpkg home directory");
    }
    Ok(home)
}

fn epoch_seconds() -> Result<u64> {
    Ok(SystemTime::now().duration_since(UNIX_EPOCH)?.as_secs())
}

fn spawn_worker() -> Result<()> {
    let mut command = Command::new(std::env::current_exe()?);
    command
        .arg(WORKER_ARGUMENT)
        .stdin(Stdio::null())
        .stdout(Stdio::null())
        .stderr(Stdio::null());
    #[cfg(unix)]
    {
        use std::os::unix::process::CommandExt;
        command.process_group(0);
    }
    #[cfg(windows)]
    {
        use std::os::windows::process::CommandExt;
        const CREATE_NO_WINDOW: u32 = 0x0800_0000;
        command.creation_flags(CREATE_NO_WINDOW);
    }
    // Dropping Child does not wait. The child owns its bounded HTTP request.
    command.spawn()?;
    Ok(())
}

#[derive(Default)]
struct Cache {
    last_attempt: Option<u64>,
    latest_tag: Option<String>,
    notified_tag: Option<String>,
}

impl Cache {
    fn load(home: &Path) -> Result<Self> {
        let last_attempt = read_optional(&home.join(ATTEMPT_FILENAME))?
            .map(|stamp| stamp.parse())
            .transpose()?;
        let latest_tag = read_optional(&home.join(RELEASE_FILENAME))?;
        let notified_tag = read_optional(&home.join(NOTIFIED_FILENAME))?;
        for tag in latest_tag.iter().chain(notified_tag.iter()) {
            release_is_newer(tag, tag)?;
        }
        Ok(Self {
            last_attempt,
            latest_tag,
            notified_tag,
        })
    }

    fn is_due(&self, now: u64) -> bool {
        self.last_attempt
            .is_none_or(|last| now.saturating_sub(last) >= CHECK_INTERVAL)
    }
}

fn read_optional(path: &Path) -> Result<Option<String>> {
    match fs::read_to_string(path) {
        Ok(value) => Ok(Some(value.trim().to_owned())),
        Err(error) if error.kind() == io::ErrorKind::NotFound => Ok(None),
        Err(error) => Err(error.into()),
    }
}

fn after_command_at(
    home: &Path,
    enabled: bool,
    now: u64,
    current_tag: &str,
    notify: impl FnOnce(&str) -> Result<()>,
    spawn: impl FnOnce() -> Result<()>,
) -> Result<()> {
    if !enabled {
        return Ok(());
    }
    let cache = Cache::load(home)?;
    // Validate even when the cache is empty, before reserving any work.
    release_is_newer(current_tag, current_tag)?;
    if let Some(tag) = cache.latest_tag.as_deref() {
        if cache.notified_tag.as_deref() != Some(tag) && release_is_newer(tag, current_tag)? {
            notify(tag)?;
            // Do not consume a notice if writing it to stderr failed.
            fs::write(home.join(NOTIFIED_FILENAME), tag)?;
        }
    }
    if cache.is_due(now) {
        fs::create_dir_all(home)?;
        // Reserve the attempt before spawning: failed spawns and failed requests both throttle.
        fs::write(home.join(ATTEMPT_FILENAME), now.to_string())?;
        spawn()?;
    }
    Ok(())
}

fn check_at(
    home: &Path,
    enabled: bool,
    now: u64,
    fetch: impl FnOnce() -> Result<String>,
) -> Result<()> {
    if !enabled {
        return Ok(());
    }
    Cache::load(home)?;
    fs::create_dir_all(home)?;
    // Also reserve here so a directly invoked worker records failures before network access.
    fs::write(home.join(ATTEMPT_FILENAME), now.to_string())?;
    let tag = fetch()?;
    release_is_newer(&tag, &tag)?;
    // Separate files prevent the child from overwriting the parent's consumed-notice marker.
    fs::write(home.join(RELEASE_FILENAME), tag)?;
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::cell::{Cell, RefCell};
    use std::path::PathBuf;
    use std::sync::atomic::{AtomicU64, Ordering};

    struct TestHome(PathBuf);

    impl TestHome {
        fn new() -> Self {
            static NEXT: AtomicU64 = AtomicU64::new(0);
            let path = std::env::temp_dir().join(format!(
                "cpkg-update-check-{}-{}",
                std::process::id(),
                NEXT.fetch_add(1, Ordering::Relaxed)
            ));
            fs::create_dir(&path).unwrap();
            Self(path)
        }
    }

    impl Drop for TestHome {
        fn drop(&mut self) {
            let _ = fs::remove_dir_all(&self.0);
        }
    }

    #[test]
    fn unsuccessful_check_and_failed_spawn_throttle_until_the_next_day() {
        let home = TestHome::new();
        let now = 1_000_000;
        assert!(check_at(&home.0, true, now, || bail!("unreachable GitHub")).is_err());
        let spawned = Cell::new(0);
        for time in [now, now + 1, now + CHECK_INTERVAL - 1] {
            after_command_at(
                &home.0,
                true,
                time,
                "v1.2.3",
                |_| panic!("failed checks must not produce notices"),
                || {
                    spawned.set(spawned.get() + 1);
                    Ok(())
                },
            )
            .unwrap();
        }
        assert_eq!(spawned.get(), 0);
        assert!(
            after_command_at(
                &home.0,
                true,
                now + CHECK_INTERVAL,
                "v1.2.3",
                |_| panic!("failed checks must not produce notices"),
                || bail!("spawn failed"),
            )
            .is_err()
        );
        after_command_at(
            &home.0,
            true,
            now + CHECK_INTERVAL + 1,
            "v1.2.3",
            |_| panic!("unexpected notice"),
            || panic!("failed spawn must also throttle"),
        )
        .unwrap();
        after_command_at(
            &home.0,
            true,
            now + 2 * CHECK_INTERVAL,
            "v1.2.3",
            |_| panic!("unexpected notice"),
            || {
                spawned.set(spawned.get() + 1);
                Ok(())
            },
        )
        .unwrap();
        assert_eq!(spawned.get(), 1);
    }

    #[test]
    fn successful_cache_notifies_once_per_newer_release_and_survives_refresh() {
        let home = TestHome::new();
        let now = 1_000_000;
        let notices = RefCell::new(Vec::new());
        for (time, tag) in [(now, "v1.2.3-fix1"), (now + CHECK_INTERVAL, "v1.2.3-fix2")] {
            check_at(&home.0, true, time, || Ok(tag.to_owned())).unwrap();
            for invocation in 0..2 {
                after_command_at(
                    &home.0,
                    true,
                    time + invocation,
                    "v1.2.3",
                    |tag| {
                        notices.borrow_mut().push(tag.to_owned());
                        Ok(())
                    },
                    || panic!("fresh metadata must throttle requests"),
                )
                .unwrap();
            }
            // A worker refreshing metadata cannot erase the parent's consumed notice.
            check_at(&home.0, true, time + 2, || Ok(tag.to_owned())).unwrap();
            after_command_at(
                &home.0,
                true,
                time + 3,
                "v1.2.3",
                |_| panic!("refresh repeated the same release notice"),
                || panic!("unexpected request"),
            )
            .unwrap();
        }
        assert_eq!(*notices.borrow(), ["v1.2.3-fix1", "v1.2.3-fix2"]);
    }

    #[test]
    fn current_or_older_release_cache_never_notifies_including_fix_revisions() {
        for (cached, current) in [
            ("v1.2.3", "v1.2.3"),
            ("v1.2.3", "v1.2.3-fix1"),
            ("v1.2.3-fix1", "v1.2.3-fix1"),
            ("v1.2.3-fix2", "v1.2.3-fix10"),
            ("v1.2.3-fix10", "v1.2.4"),
        ] {
            let home = TestHome::new();
            check_at(&home.0, true, 100, || Ok(cached.to_owned())).unwrap();
            after_command_at(
                &home.0,
                true,
                101,
                current,
                |_| panic!("obsolete notice for {cached} on {current}"),
                || panic!("unexpected request"),
            )
            .unwrap();
        }
    }

    #[test]
    fn invalid_state_skips_notices_and_requests() {
        for (filename, value) in [
            (ATTEMPT_FILENAME, "not a timestamp"),
            (RELEASE_FILENAME, "v1.2.4-rc1"),
            (RELEASE_FILENAME, "1.2.4"),
            (NOTIFIED_FILENAME, "v1.2.3+build"),
        ] {
            let home = TestHome::new();
            fs::write(home.0.join(filename), value).unwrap();
            assert!(
                after_command_at(
                    &home.0,
                    true,
                    1_000_000,
                    "v1.2.3",
                    |_| panic!("invalid state must not notify"),
                    || panic!("invalid state must not launch work"),
                )
                .is_err()
            );
        }
    }

    #[test]
    fn opt_out_prevents_cached_notice_spawn_fetch_and_cache_creation() {
        let home = TestHome::new();
        check_at(&home.0, true, 100, || Ok("v1.2.4".to_owned())).unwrap();
        after_command_at(
            &home.0,
            false,
            1_000_000,
            "v1.2.3",
            |_| panic!("disabled checker must not notify"),
            || panic!("disabled checker must not spawn"),
        )
        .unwrap();
        let missing = home.0.join("missing");
        check_at(&missing, false, 1_000_000, || {
            panic!("disabled worker must not fetch")
        })
        .unwrap();
        assert!(!missing.exists());
        assert_eq!(
            fs::read_to_string(home.0.join(ATTEMPT_FILENAME)).unwrap(),
            "100"
        );
    }

    #[test]
    fn failed_notice_write_does_not_consume_the_release() {
        let home = TestHome::new();
        check_at(&home.0, true, 100, || Ok("v1.2.4".to_owned())).unwrap();
        assert!(
            after_command_at(
                &home.0,
                true,
                101,
                "v1.2.3",
                |_| bail!("stderr unavailable"),
                || panic!("unexpected request"),
            )
            .is_err()
        );
        let notified = Cell::new(false);
        after_command_at(
            &home.0,
            true,
            102,
            "v1.2.3",
            |_| {
                notified.set(true);
                Ok(())
            },
            || panic!("unexpected request"),
        )
        .unwrap();
        assert!(notified.get());
    }
}
