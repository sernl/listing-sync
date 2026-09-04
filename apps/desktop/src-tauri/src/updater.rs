//! The start-up update check.
//!
//! The release pipeline signs updater artefacts and publishes them to the
//! channel compiled into the endpoint in `tauri.conf.json`, and `lib.rs`
//! registers `tauri-plugin-updater` on every desktop build, but until now
//! nothing ever asked the plugin for an update. Version 0.1.3 therefore
//! shipped an update path it had no code to take. This module is that code.
//!
//! It runs once, at start-up, and the sync schedule's first tick waits on it.
//! The ordering is the point rather than a convenience, because installing
//! ends the process. On Windows the plugin hands the installer to
//! `ShellExecuteW` and calls `std::process::exit(0)` from inside the install
//! itself (tauri-plugin-updater 2.11.0, `src/updater.rs:862-876`); on macOS
//! and Linux it moves the new application into place and returns, leaving the
//! relaunch to the caller (`src/updater.rs:1288-1381` and `:1039-1045`), which
//! is what [`check_at_startup`] does. Either way the process ends, so it has
//! to end before a marketplace request is in flight rather than during one.
//!
//! Nothing here draws anything. The installer shows the only progress the
//! seller sees: the updater configuration names no `installMode`, so the
//! plugin's default `Passive` applies and the NSIS installer is launched with
//! `/P`, `/UPDATE` and `/R` (`src/config.rs:21-22` and `:41-58`,
//! `src/updater.rs:884-908`).
//!
//! One line goes to stderr, through the only reporting this crate has:
//! `startup` writes there too, and adding a logging dependency is a founder
//! decision rather than a detail of this change. That inherits `startup`'s own
//! limitation, stated there and worth restating here — a release build on
//! Windows is linked with `windows_subsystem = "windows"` and has no console,
//! so the line reaches a developer running the binary from a terminal and
//! nobody else. Giving it the durability `startup.log` has would mean an
//! appending entry point in `startup`, which owns that file's protocol.
//!
//! The decision to check and the reporting of what a check came to are pure
//! and tested below. The check itself is not: it is one network request to a
//! CDN followed by a signed installer that ends the process, and there is no
//! seam in the plugin to substitute either. What the tests cover is therefore
//! the policy and the log line, and what they do not cover is stated here
//! rather than implied by their absence.

use core::time::Duration;

#[cfg(desktop)]
use tauri::AppHandle;
#[cfg(desktop)]
use tauri_plugin_updater::UpdaterExt as _;

/// How long the check may take before start-up proceeds without it.
///
/// The sync schedule waits on this, so it also bounds how late the first cycle
/// can be when the endpoint neither answers nor fails.
pub const CHECK_TIMEOUT: Duration = Duration::from_secs(15);

/// How long the download of an offered update may take before start-up gives
/// up on it.
///
/// Two orders of magnitude larger than the check, because this is a whole
/// installer over the seller's own connection rather than one JSON response.
/// It exists because nothing else bounds it: the plugin applies a request
/// timeout to the download only when the `Update` carries one, and `check`
/// leaves that field `None` (tauri-plugin-updater 2.11.0,
/// `src/updater.rs:595` and `:698-700`). A connection that stalls rather than
/// resets would otherwise hold the schedule's first tick for the life of the
/// process, which is the one failure this whole module must not cause. A
/// seller whose connection cannot fetch the installer inside it gets a failed
/// update and a working schedule, and the next start-up tries again.
pub const DOWNLOAD_TIMEOUT: Duration = Duration::from_mins(10);

/// Why a build does not check for an update.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Skip {
    /// A development build. The endpoint answers for published releases, and a
    /// build from the working copy is not one.
    Development,
    /// A build with no updater plugin registered. `lib.rs` registers it under
    /// `#[cfg(desktop)]` because the plugin's own manifest declares Android
    /// support level `none`, and asking for an unregistered plugin's state
    /// panics rather than failing (tauri 2.11.5, `src/lib.rs:733-738`).
    Unsupported,
}

/// Whether this build checks for an update at start-up.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Decision {
    Check,
    Skip(Skip),
}

/// The policy, over the two facts it turns on rather than over the build it is
/// compiled into.
///
/// `updater_registered` is what keeps the panic above out of reach: the only
/// caller passes `cfg!(desktop)`, so widening this module to a platform where
/// `lib.rs` registers no updater refuses the check instead of asking a plugin
/// that is not there.
#[must_use]
pub const fn decide(development: bool, updater_registered: bool) -> Decision {
    if development {
        Decision::Skip(Skip::Development)
    } else if updater_registered {
        Decision::Check
    } else {
        Decision::Skip(Skip::Unsupported)
    }
}

/// What one start-up check came to.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Outcome {
    /// This build does not check.
    Skipped(Skip),
    /// The endpoint offered nothing newer, which it reports as `204 No
    /// Content` (tauri-plugin-updater 2.11.0, `src/updater.rs:529-534`).
    UpToDate,
    /// An update was downloaded, verified and installed. The process ends
    /// next, either inside the install or through the restart that follows it.
    Installed { version: String },
    /// The check did not answer within [`CHECK_TIMEOUT`].
    TimedOut,
    /// The endpoint, the download, the signature or the install failed. An
    /// unreachable endpoint arrives here as the plugin's `Reqwest` variant
    /// (`src/error.rs:39-41`), raised from the send in
    /// `src/updater.rs:560-563`.
    Failed(String),
}

/// The one line an outcome is reported as.
///
/// Collapsing whitespace is not cosmetic. The text in [`Outcome::Failed`] is
/// whatever the plugin's error chain rendered, and a multi-line one would put
/// a second line into a log whose reader is told to expect one.
#[must_use]
pub fn line(outcome: &Outcome) -> String {
    match outcome {
        Outcome::Skipped(Skip::Development) => "update skipped: development build".to_owned(),
        Outcome::Skipped(Skip::Unsupported) => {
            "update skipped: no updater on this platform".to_owned()
        }
        Outcome::UpToDate => "update none: this version is current".to_owned(),
        Outcome::Installed { version } => {
            format!("update installed: {}, restarting", one_line(version))
        }
        Outcome::TimedOut => format!(
            "update check timed out after {}s, continuing",
            CHECK_TIMEOUT.as_secs()
        ),
        Outcome::Failed(why) => format!("update failed: {}, continuing", one_line(why)),
    }
}

/// The text with every run of whitespace reduced to one space.
fn one_line(text: &str) -> String {
    text.split_whitespace().collect::<Vec<_>>().join(" ")
}

/// Checks for an update, installs one if it is offered, and reports what
/// happened on stderr.
///
/// Returns on every path the process survives, so the caller's schedule starts
/// whether the endpoint answered, refused or was never reached. It does not
/// return when an update installed: Windows exits inside the install and the
/// other platforms exit through [`AppHandle::restart`], which from a task
/// thread requests the exit and parks until the event loop takes it (tauri
/// 2.11.5, `src/app.rs:588-609` and `:1430-1436`).
#[cfg(desktop)]
pub async fn check_at_startup(app: &AppHandle) {
    let outcome = match decide(cfg!(debug_assertions), cfg!(desktop)) {
        Decision::Skip(why) => Outcome::Skipped(why),
        Decision::Check => check_and_install(app).await,
    };
    eprintln!("{}", line(&outcome));
    if matches!(outcome, Outcome::Installed { .. }) {
        app.restart();
    }
}

/// The network path: the check under [`CHECK_TIMEOUT`], then the download
/// under [`DOWNLOAD_TIMEOUT`] and the install, if an update is offered.
///
/// The download and the install are taken separately rather than through the
/// plugin's `download_and_install`, which is exactly those two in sequence
/// (tauri-plugin-updater 2.11.0, `src/updater.rs:766-767`). Separating them is
/// what confines the deadline to the download: `install` is synchronous
/// (`src/updater.rs:751-753`), so it holds no await point for a cancellation
/// to land on, and no timeout of ours can interrupt an installer mid-write.
#[cfg(desktop)]
async fn check_and_install(app: &AppHandle) -> Outcome {
    let updater = match app.updater() {
        Ok(updater) => updater,
        Err(why) => return Outcome::Failed(why.to_string()),
    };
    let offered = match tokio::time::timeout(CHECK_TIMEOUT, updater.check()).await {
        Err(_elapsed) => return Outcome::TimedOut,
        Ok(Err(why)) => return Outcome::Failed(why.to_string()),
        Ok(Ok(None)) => return Outcome::UpToDate,
        Ok(Ok(Some(offered))) => offered,
    };
    let version = offered.version.clone();
    let bytes = match tokio::time::timeout(
        DOWNLOAD_TIMEOUT,
        offered.download(|_chunk, _total| {}, || {}),
    )
    .await
    {
        Err(_elapsed) => {
            return Outcome::Failed(format!(
                "the download did not finish within {}s",
                DOWNLOAD_TIMEOUT.as_secs()
            ))
        }
        Ok(Err(why)) => return Outcome::Failed(why.to_string()),
        Ok(Ok(bytes)) => bytes,
    };
    match offered.install(bytes) {
        Ok(()) => Outcome::Installed { version },
        Err(why) => Outcome::Failed(why.to_string()),
    }
}

#[cfg(test)]
mod tests {
    use super::{decide, line, Decision, Outcome, Skip, CHECK_TIMEOUT, DOWNLOAD_TIMEOUT};

    #[test]
    fn a_development_build_never_checks() {
        assert_eq!(
            decide(true, true),
            Decision::Skip(Skip::Development),
            "a build from the working copy has a version the endpoint does not publish, so \
             checking would at best waste a request and at worst install over the build under \
             test"
        );
        assert_eq!(
            decide(true, false),
            Decision::Skip(Skip::Development),
            "the development reason is the one a developer can act on, so it is reported ahead \
             of the platform one"
        );
    }

    #[test]
    fn a_release_build_checks_only_where_the_plugin_is_registered() {
        assert_eq!(
            decide(false, true),
            Decision::Check,
            "this is the whole point of the module: a released desktop build must ask"
        );
        assert_eq!(
            decide(false, false),
            Decision::Skip(Skip::Unsupported),
            "asking for the state of a plugin that was never registered panics rather than \
             failing (tauri 2.11.5, src/lib.rs:733-738), so the policy must refuse first"
        );
    }

    #[test]
    fn every_outcome_reports_as_exactly_one_line() {
        let outcomes = [
            Outcome::Skipped(Skip::Development),
            Outcome::Skipped(Skip::Unsupported),
            Outcome::UpToDate,
            Outcome::Installed {
                version: "0.2.1".to_owned(),
            },
            Outcome::TimedOut,
            Outcome::Failed("error sending request\n  caused by: dns error".to_owned()),
        ];
        for outcome in &outcomes {
            let text = line(outcome);
            assert!(
                !text.is_empty(),
                "an empty line would make {outcome:?} indistinguishable from no report at all"
            );
            assert!(
                !text.contains('\n'),
                "the plugin's error chain renders across several lines, and one of them in the \
                 log would read as a second event: {text}"
            );
        }
    }

    #[test]
    fn the_line_names_what_a_reader_has_to_act_on() {
        assert!(
            line(&Outcome::Installed {
                version: "0.2.1".to_owned()
            })
            .contains("0.2.1"),
            "the installed version is the one fact that says which release the seller now has"
        );
        assert!(
            line(&Outcome::Failed("dns error".to_owned())).contains("dns error"),
            "a failure with the cause stripped out cannot be diagnosed from the log"
        );
        assert!(
            line(&Outcome::TimedOut).contains(&CHECK_TIMEOUT.as_secs().to_string()),
            "the bound is what distinguishes a slow endpoint from an unreachable one"
        );
    }

    #[test]
    fn the_timeout_stays_short_enough_to_precede_the_first_sync() {
        assert!(
            !CHECK_TIMEOUT.is_zero() && CHECK_TIMEOUT <= core::time::Duration::from_secs(30),
            "the schedule's first cycle waits on this check, so a long bound delays every \
             seller's first sync by that much on a start-up where the endpoint never answers"
        );
    }

    #[test]
    fn every_wait_the_schedule_sits_behind_is_bounded() {
        assert!(
            !DOWNLOAD_TIMEOUT.is_zero() && DOWNLOAD_TIMEOUT.as_secs() < 3600,
            "nothing else bounds the download, so an unbounded one here is a stalled connection \
             that stops this device syncing for the life of the process"
        );
        assert!(
            DOWNLOAD_TIMEOUT > CHECK_TIMEOUT,
            "an installer over a seller's own connection is not one JSON response, and a \
             download bound at the check's deadline would fail every real update"
        );
    }
}
