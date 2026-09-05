//! Update checking.
//!
//! emede notifies; it never replaces its own binary. That is a distribution
//! constraint, not a preference: the app ships through three channels and only
//! one of them is ours to write to.
//!
//! | Channel              | Binary lands in     | Owned by   |
//! |----------------------|---------------------|------------|
//! | `scripts/install.sh` | `~/.local/bin`      | the user   |
//! | `.deb` / `.rpm`      | `/usr/bin`          | dpkg / rpm |
//! | `npm run tauri build`| `target/release`    | the dev    |
//!
//! Overwriting the second desynchronizes the package database (and needs root);
//! overwriting the third clobbers a build in progress. So the check resolves
//! which channel this binary came from and hands back the right *instruction*
//! for it — `install.sh` for a user-local install, "use your package manager"
//! for a system package, and silence for a dev build, which should never nag.
//!
//! Every failure here is non-fatal by construction. No network, a rate-limited
//! API, a laptop offline on a plane: the reader behaves exactly as it does
//! without this module. The one thing an update check may never do is get
//! between the user and the document.

use serde::{Deserialize, Serialize};
use std::path::{Path, PathBuf};
use std::time::{SystemTime, UNIX_EPOCH};

/// GitHub API endpoint for the newest non-prerelease. Matches what
/// `scripts/install.sh` resolves, so the version we advertise is the version
/// the suggested command actually installs.
const LATEST_RELEASE_API: &str = "https://api.github.com/repos/jmejia8/emede/releases/latest";

/// The command `install.sh` users re-run to upgrade. Deliberately the same
/// published one-liner as the README: it verifies a SHA256 and is the only
/// upgrade path we ask anyone to trust.
pub const INSTALL_COMMAND: &str =
    "curl -fsSL https://raw.githubusercontent.com/jmejia8/emede/main/scripts/install.sh | sh";

const RELEASES_PAGE: &str = "https://github.com/jmejia8/emede/releases/latest";

/// How long a check is considered fresh. Multiple emede windows are separate
/// processes sharing this cache, so opening ten documents costs one request.
const CHECK_INTERVAL_SECS: u64 = 24 * 60 * 60;

/// Cap on the API response body. The payload we want is a few KB of JSON.
const MAX_BODY_BYTES: u64 = 512 * 1024;

// ── Install channel ────────────────────────────────────────────────────────────

/// Where this binary came from, and therefore how it should be upgraded.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum InstallChannel {
    /// Installed by `scripts/install.sh` into a user-writable prefix.
    User,
    /// A distribution package (`.deb` / `.rpm`) under a system prefix.
    System,
    /// A local `cargo`/`tauri` build. Never advertise updates to a dev build.
    Source,
    /// Somewhere else entirely — point at the releases page and say no more.
    Unknown,
}

impl InstallChannel {
    /// The upgrade instruction to show, or `None` when we should stay quiet.
    fn hint(self) -> Option<&'static str> {
        match self {
            InstallChannel::User => Some(INSTALL_COMMAND),
            InstallChannel::System => None,
            InstallChannel::Source => None,
            InstallChannel::Unknown => None,
        }
    }
}

/// Classify `exe` — the path of the running binary — into an [`InstallChannel`].
///
/// Order matters: a source build usually lives *under* `$HOME`, so the
/// `target/{debug,release}` test has to run before the home-prefix test or every
/// `npm run tauri dev` session would be told to re-run the installer.
fn classify_exe(exe: &Path, home: Option<&Path>) -> InstallChannel {
    let components: Vec<String> = exe
        .components()
        .map(|c| c.as_os_str().to_string_lossy().into_owned())
        .collect();

    // A cargo build artifact: ".../target/debug/emede" or ".../target/release/emede".
    if components
        .windows(2)
        .any(|w| w[0] == "target" && (w[1] == "debug" || w[1] == "release"))
    {
        return InstallChannel::Source;
    }

    if let Some(home) = home {
        // An empty or root home would match everything; ignore it.
        if home.parent().is_some() && exe.starts_with(home) {
            return InstallChannel::User;
        }
    }

    for prefix in ["/usr", "/opt", "/bin", "/sbin", "/snap", "/var/lib/flatpak"] {
        if exe.starts_with(prefix) {
            return InstallChannel::System;
        }
    }

    InstallChannel::Unknown
}

/// Classify the currently running binary. Resolves symlinks first so that a
/// `~/.local/bin/emede` symlink into `/usr` is judged by its target.
pub fn detect_channel() -> InstallChannel {
    let exe = match std::env::current_exe() {
        Ok(p) => std::fs::canonicalize(&p).unwrap_or(p),
        Err(_) => return InstallChannel::Unknown,
    };
    classify_exe(&exe, dirs::home_dir().as_deref())
}

// ── Version comparison ─────────────────────────────────────────────────────────

/// A release version reduced to what we need to order two of them.
///
/// `pre` records whether the version carried a prerelease suffix (`0.3.0-rc1`),
/// which sorts *below* the same numeric triple without one — so shipping
/// `0.3.0` correctly reads as an update over `0.3.0-rc1`.
#[derive(Debug, PartialEq, Eq)]
struct Version {
    parts: (u64, u64, u64),
    pre: bool,
}

/// Parse `major.minor.patch` from a tag, tolerating a leading `v` and any
/// `-suffix` / `+build` metadata. Returns `None` if the numeric core is absent.
fn parse_version(raw: &str) -> Option<Version> {
    let core = raw.trim().trim_start_matches(['v', 'V']);
    let (core, pre) = match core.find(['-', '+']) {
        Some(i) => (&core[..i], true),
        None => (core, false),
    };

    let mut it = core.split('.');
    let major = it.next()?.parse().ok()?;
    // A missing minor/patch is a well-formed "1" or "1.2"; treat it as zero
    // rather than rejecting the whole tag.
    let minor = it.next().map_or(Some(0), |s| s.parse().ok())?;
    let patch = it.next().map_or(Some(0), |s| s.parse().ok())?;

    Some(Version {
        parts: (major, minor, patch),
        pre,
    })
}

/// Is `latest` a newer release than `current`?
///
/// Compares field by field, never lexicographically — `0.2.10` is newer than
/// `0.2.9`, and a string compare says the opposite. An unparseable version on
/// either side yields `false`: we would rather miss an update than invent one.
pub fn is_newer(latest: &str, current: &str) -> bool {
    match (parse_version(latest), parse_version(current)) {
        (Some(l), Some(c)) => match l.parts.cmp(&c.parts) {
            std::cmp::Ordering::Greater => true,
            std::cmp::Ordering::Less => false,
            // Same numbers: only a final release beats a prerelease.
            std::cmp::Ordering::Equal => c.pre && !l.pre,
        },
        _ => false,
    }
}

// ── Cache ──────────────────────────────────────────────────────────────────────

/// The last successful check, so N windows over N days make one request.
#[derive(Debug, Default, Clone, Serialize, Deserialize)]
struct CheckCache {
    /// Unix seconds of the last successful fetch. 0 = never.
    #[serde(default)]
    last_checked: u64,
    #[serde(default)]
    latest_version: String,
    #[serde(default)]
    release_url: String,
}

fn cache_path() -> PathBuf {
    dirs::config_dir()
        .unwrap_or_else(|| PathBuf::from("."))
        .join("emede")
        .join("update_check.json")
}

fn now_secs() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|d| d.as_secs())
        .unwrap_or(0)
}

impl CheckCache {
    /// A cache entry counts as fresh only if it holds a version *and* was
    /// written within the interval. A clock that has moved backwards makes
    /// `last_checked` look like the future; treat that as stale and re-check.
    fn is_fresh(&self, now: u64) -> bool {
        !self.latest_version.is_empty()
            && self.last_checked <= now
            && now - self.last_checked < CHECK_INTERVAL_SECS
    }
}

// ── Status reported to callers ─────────────────────────────────────────────────

/// What the frontend (and `--check-update`) needs to render the result.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct UpdateStatus {
    /// The running version.
    pub current_version: String,
    /// The newest published release, when a check has succeeded at some point.
    pub latest_version: Option<String>,
    pub update_available: bool,
    /// Release page for the new version, for a "What's new" link.
    pub release_url: Option<String>,
    pub channel: InstallChannel,
    /// The command to run to upgrade, when this channel has one.
    pub install_command: Option<String>,
    /// False when no check ran: disabled in settings, or a dev build.
    pub checked: bool,
}

impl UpdateStatus {
    fn quiet(channel: InstallChannel) -> Self {
        Self {
            current_version: env!("CARGO_PKG_VERSION").to_string(),
            latest_version: None,
            update_available: false,
            release_url: None,
            channel,
            install_command: None,
            checked: false,
        }
    }

    fn from_latest(channel: InstallChannel, latest: &str, release_url: &str) -> Self {
        let current = env!("CARGO_PKG_VERSION");
        let available = is_newer(latest, current);
        Self {
            current_version: current.to_string(),
            latest_version: Some(latest.to_string()),
            update_available: available,
            release_url: Some(if release_url.is_empty() {
                RELEASES_PAGE.to_string()
            } else {
                release_url.to_string()
            }),
            channel,
            install_command: if available {
                channel.hint().map(str::to_string)
            } else {
                None
            },
            checked: true,
        }
    }
}

// ── Fetch ──────────────────────────────────────────────────────────────────────

/// Ask GitHub for the newest release. Returns `(tag, html_url)`.
///
/// Reuses the shared agent from `markdown`, which already carries hard timeouts
/// and the SSRF-blocking resolver — there is no reason for this request to have
/// weaker limits than a user-initiated one.
fn fetch_latest_release() -> Result<(String, String), String> {
    let response = crate::markdown::http_agent()
        .get(LATEST_RELEASE_API)
        .set("Accept", "application/vnd.github+json")
        // GitHub rejects API requests without a User-Agent.
        .set(
            "User-Agent",
            concat!("emede/", env!("CARGO_PKG_VERSION")),
        )
        .call()
        .map_err(|e| match e {
            // 403 here is almost always the unauthenticated 60/hour rate limit.
            // Say so plainly instead of retrying into the same wall.
            ureq::Error::Status(403, _) => {
                "GitHub API rate limit reached; try again later.".to_string()
            }
            other => format!("could not reach GitHub: {other}"),
        })?;

    let mut body = String::new();
    use std::io::Read;
    response
        .into_reader()
        .take(MAX_BODY_BYTES)
        .read_to_string(&mut body)
        .map_err(|e| format!("could not read the response: {e}"))?;

    let json: serde_json::Value =
        serde_json::from_str(&body).map_err(|e| format!("unexpected response: {e}"))?;

    let tag = json
        .get("tag_name")
        .and_then(|v| v.as_str())
        .ok_or_else(|| "release payload has no tag_name".to_string())?;
    let url = json
        .get("html_url")
        .and_then(|v| v.as_str())
        .unwrap_or(RELEASES_PAGE);

    Ok((tag.trim_start_matches(['v', 'V']).to_string(), url.to_string()))
}

// ── Entry points ───────────────────────────────────────────────────────────────

/// Resolve the update status, consulting the cache unless `force`.
///
/// `force` means the user asked directly ("Check again", `--check-update`), so
/// it bypasses both the cache and the `update_check` setting: an explicit
/// request is its own consent.
pub fn check(force: bool) -> Result<UpdateStatus, String> {
    let channel = detect_channel();

    // A dev build already has the source tree; it never needs telling.
    if channel == InstallChannel::Source && !force {
        return Ok(UpdateStatus::quiet(channel));
    }

    if !force && !crate::settings::load_settings().update_check {
        return Ok(UpdateStatus::quiet(channel));
    }

    let path = cache_path();
    let cache: CheckCache = crate::persist::load_json_or_backup(&path);
    let now = now_secs();

    if !force && cache.is_fresh(now) {
        return Ok(UpdateStatus::from_latest(
            channel,
            &cache.latest_version,
            &cache.release_url,
        ));
    }

    let (latest, release_url) = fetch_latest_release()?;

    let json = serde_json::to_string_pretty(&CheckCache {
        last_checked: now,
        latest_version: latest.clone(),
        release_url: release_url.clone(),
    })
    .map_err(|e| e.to_string())?;
    // A cache we could not persist costs one extra request tomorrow; it is not
    // worth failing a successful check over.
    let _ = crate::persist::write_json_atomic(&path, &json);

    Ok(UpdateStatus::from_latest(channel, &latest, &release_url))
}

/// Frontend entry point. Called once after boot with `force = false`, and by
/// the "Check again" button with `force = true`.
#[tauri::command]
pub fn check_for_update(force: bool) -> Result<UpdateStatus, String> {
    check(force)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_plain_and_prefixed_versions() {
        assert_eq!(parse_version("0.2.6").unwrap().parts, (0, 2, 6));
        assert_eq!(parse_version("v0.2.6").unwrap().parts, (0, 2, 6));
        assert_eq!(parse_version(" v1.0.0 ").unwrap().parts, (1, 0, 0));
        assert_eq!(parse_version("2").unwrap().parts, (2, 0, 0));
        assert_eq!(parse_version("2.1").unwrap().parts, (2, 1, 0));
    }

    #[test]
    fn parses_prerelease_and_build_metadata() {
        let v = parse_version("0.3.0-rc1").unwrap();
        assert_eq!(v.parts, (0, 3, 0));
        assert!(v.pre);
        assert!(!parse_version("0.3.0").unwrap().pre);
        assert_eq!(parse_version("0.3.0+build7").unwrap().parts, (0, 3, 0));
    }

    #[test]
    fn rejects_junk() {
        assert!(parse_version("").is_none());
        assert!(parse_version("nightly").is_none());
        assert!(parse_version("1.x.0").is_none());
    }

    /// The reason this comparison is numeric rather than lexicographic.
    #[test]
    fn double_digit_patch_beats_single_digit() {
        assert!(is_newer("0.2.10", "0.2.9"));
        assert!(!is_newer("0.2.9", "0.2.10"));
        assert!("0.2.10" < "0.2.9", "string compare would get this wrong");
    }

    #[test]
    fn orders_across_fields() {
        assert!(is_newer("1.0.0", "0.9.9"));
        assert!(is_newer("0.3.0", "0.2.99"));
        assert!(!is_newer("0.2.6", "0.2.6"));
        assert!(!is_newer("0.2.5", "0.2.6"));
    }

    #[test]
    fn final_release_beats_its_prerelease() {
        assert!(is_newer("0.3.0", "0.3.0-rc1"));
        assert!(!is_newer("0.3.0-rc1", "0.3.0"));
        assert!(!is_newer("0.3.0-rc2", "0.3.0-rc1"));
    }

    #[test]
    fn unparseable_version_never_claims_an_update() {
        assert!(!is_newer("garbage", "0.2.6"));
        assert!(!is_newer("0.3.0", "garbage"));
    }

    #[test]
    fn tag_prefix_is_accepted_on_either_side() {
        assert!(is_newer("v0.2.7", "0.2.6"));
        assert!(is_newer("0.2.7", "v0.2.6"));
    }

    // ── channel detection ──────────────────────────────────────────────────

    #[test]
    fn installer_prefix_is_a_user_install() {
        let home = PathBuf::from("/home/jesus");
        assert_eq!(
            classify_exe(Path::new("/home/jesus/.local/bin/emede"), Some(&home)),
            InstallChannel::User
        );
    }

    #[test]
    fn system_prefixes_are_packages() {
        for p in ["/usr/bin/emede", "/usr/local/bin/emede", "/opt/emede/emede"] {
            assert_eq!(
                classify_exe(Path::new(p), Some(Path::new("/home/jesus"))),
                InstallChannel::System,
                "{p}"
            );
        }
    }

    /// A dev build lives under `$HOME` too, so this ordering is the whole point:
    /// `npm run tauri dev` must never be told to re-run the installer.
    #[test]
    fn build_artifacts_beat_the_home_prefix() {
        let home = PathBuf::from("/home/jesus");
        for p in [
            "/home/jesus/develop/repos/emede/src-tauri/target/debug/emede",
            "/home/jesus/develop/repos/emede/src-tauri/target/release/emede",
        ] {
            assert_eq!(
                classify_exe(Path::new(p), Some(&home)),
                InstallChannel::Source,
                "{p}"
            );
        }
    }

    #[test]
    fn a_root_home_matches_nothing() {
        assert_eq!(
            classify_exe(Path::new("/srv/emede"), Some(Path::new("/"))),
            InstallChannel::Unknown
        );
    }

    #[test]
    fn only_user_installs_get_a_command() {
        assert_eq!(InstallChannel::User.hint(), Some(INSTALL_COMMAND));
        assert!(InstallChannel::System.hint().is_none());
        assert!(InstallChannel::Source.hint().is_none());
        assert!(InstallChannel::Unknown.hint().is_none());
    }

    // ── reported status ────────────────────────────────────────────────────

    #[test]
    fn a_newer_release_carries_the_command_for_a_user_install() {
        let s = UpdateStatus::from_latest(InstallChannel::User, "99.0.0", "https://example/rel");
        assert!(s.update_available);
        assert!(s.checked);
        assert_eq!(s.install_command.as_deref(), Some(INSTALL_COMMAND));
        assert_eq!(s.release_url.as_deref(), Some("https://example/rel"));
    }

    /// A package install learns that an update exists but is never handed a
    /// command — replacing a dpkg/rpm-owned binary is not emede's business.
    #[test]
    fn a_package_install_gets_no_command() {
        let s = UpdateStatus::from_latest(InstallChannel::System, "99.0.0", "");
        assert!(s.update_available);
        assert!(s.install_command.is_none());
        assert_eq!(s.release_url.as_deref(), Some(RELEASES_PAGE));
    }

    #[test]
    fn being_current_offers_nothing_to_run() {
        let current = env!("CARGO_PKG_VERSION");
        let s = UpdateStatus::from_latest(InstallChannel::User, current, "");
        assert!(!s.update_available);
        assert!(s.install_command.is_none());
    }

    #[test]
    fn a_quiet_status_claims_no_check_happened() {
        let s = UpdateStatus::quiet(InstallChannel::Source);
        assert!(!s.checked);
        assert!(!s.update_available);
        assert!(s.latest_version.is_none());
    }

    // ── cache freshness ────────────────────────────────────────────────────

    #[test]
    fn cache_freshness_window() {
        let c = CheckCache {
            last_checked: 1_000_000,
            latest_version: "0.2.7".into(),
            release_url: String::new(),
        };
        assert!(c.is_fresh(1_000_000));
        assert!(c.is_fresh(1_000_000 + CHECK_INTERVAL_SECS - 1));
        assert!(!c.is_fresh(1_000_000 + CHECK_INTERVAL_SECS));
    }

    #[test]
    fn empty_or_future_cache_is_stale() {
        let empty = CheckCache {
            last_checked: 1_000_000,
            ..Default::default()
        };
        assert!(!empty.is_fresh(1_000_000));

        // A clock moved backwards must re-check, not wait out a bogus window.
        let future = CheckCache {
            last_checked: 2_000_000,
            latest_version: "0.2.7".into(),
            release_url: String::new(),
        };
        assert!(!future.is_fresh(1_000_000));
    }
}
