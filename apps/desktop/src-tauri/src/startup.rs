//! Where a start-up failure goes when there is no console to print it to.
//!
//! `main.rs` sets `windows_subsystem = "windows"` for the release build, so
//! the shipping Windows binary has no console attached and everything written
//! to stderr is discarded by the operating system. A failure before the window
//! appears therefore leaves an application that opens and closes with no text
//! anywhere to say why, and no Windows machine here to attach a debugger to.
//! This module gives that failure a file.
//!
//! The protocol is three lines of behaviour and one of interpretation.
//! [`opening`] truncates the log to a single line naming the build, so the
//! file always describes the most recent launch and never an older one.
//! [`catch_panics`] appends a panic, and [`fatal`] appends an error returned
//! by the event loop. The interpretation is what the truncation buys: a log
//! holding only the opening line means the process died after setup began
//! without reaching either failure path.
//!
//! A missing or empty log means less than it appears to. It does not narrow
//! the failure to the data directory, and a reader who takes it that way looks
//! in the wrong place. [`opening`] is the only caller that seeds `DATA_DIR`,
//! and it cannot run before
//! `tauri::Builder::build` returns, because the directory is Tauri's to
//! resolve and there is no application to ask until then. Every failure inside
//! that call therefore reports to stderr and to no file: the runtime, the
//! context, and every plugin's initialisation (`tauri` 2.11.5,
//! `src/app.rs:2440`). On Windows stderr is discarded, so that region is
//! genuinely silent; on Android it is not, because tao redirects the process's
//! stdout and stderr into logcat under the tag `RustStdoutStderr` (`tao`
//! 0.35.3, `src/platform_impl/android/ndk_glue.rs:327` and `:333`).
//!
//! The two failure paths are not interchangeable and the hook is the one that
//! matters. A `setup` failure does not come back as an error from `run()`:
//! Tauri panics on it from inside the event-loop callback (`tauri` 2.11.5,
//! `src/app.rs:1424`), which a Linux reproduction of this exact failure
//! confirmed. Removing the hook would restore the original symptom for the
//! most likely cause.

use std::fs::{self, OpenOptions};
use std::io::Write as _;
use std::path::{Path, PathBuf};
use std::sync::OnceLock;

/// The file, inside the application data directory. `docs/notes/design/desktop-client.md`
/// records where that directory is on each platform.
pub const STARTUP_LOG: &str = "startup.log";

/// The prefix every launch writes, and the string a reader greps for.
pub const OPENING: &str = "starting";

/// The data directory as Tauri resolved it.
///
/// Held rather than recomputed because the failure paths run with no
/// `AppHandle` in scope, and a second, independent resolution of the same
/// directory would eventually disagree with Tauri's and send a reader to a
/// path the log is not in.
static DATA_DIR: OnceLock<PathBuf> = OnceLock::new();

#[must_use]
pub fn log_path(data_dir: &Path) -> PathBuf {
    data_dir.join(STARTUP_LOG)
}

/// Records the data directory and truncates the log to one opening line.
///
/// Called between building the application and running it, which is the
/// earliest point the data directory is resolvable and still before Tauri
/// creates the window. Anything that fails after this call has a log to fail
/// into.
pub fn opening(data_dir: &Path) {
    let data_dir = DATA_DIR.get_or_init(|| data_dir.to_path_buf());
    let line = format!(
        "{OPENING} {} {} {}\n",
        env!("CARGO_PKG_VERSION"),
        std::env::consts::OS,
        std::env::consts::ARCH
    );
    eprint!("{line}");
    let path = log_path(data_dir);
    if let Err(why) = fs::create_dir_all(data_dir) {
        eprintln!("cannot create {}: {why}", data_dir.display());
    }
    if let Err(why) = fs::write(&path, line.as_bytes()) {
        eprintln!("cannot write {}: {why}", path.display());
    }
}

/// Reports an error returned by the event loop, and exits non-zero.
///
/// Deliberately not the only failure path, and not the one that fires most
/// often. Tauri does not return a `setup` failure from `run()` at all: it
/// panics inside the event-loop callback (`tauri` 2.11.5, `src/app.rs:1425`,
/// "Failed to setup app"), which is why [`catch_panics`] exists and why
/// removing it would leave this arm covering almost nothing.
#[expect(
    clippy::exit,
    reason = "the window never appeared and the event loop is gone; returning would leave a \
              process with no interface and no way to acquire one, which on Windows is an \
              invisible process rather than a visible failure"
)]
pub fn fatal(error: &dyn core::error::Error) -> ! {
    record(
        "the Teachouse desktop client failed to start",
        &chain(error),
    );
    std::process::exit(1)
}

/// Sends panics to the log as well as to the stderr nobody can read.
///
/// This is the path a `setup` failure actually takes. Tauri raises one as a
/// panic from inside the event-loop callback rather than as an error out of
/// `run()`, so without this hook a failed `setup` would leave a log holding
/// only the opening line, and a Windows release build would still show
/// nothing at all. The previous hook is kept and called after, so a debug run
/// with a console still prints what it always did.
pub fn catch_panics() {
    let default = std::panic::take_hook();
    std::panic::set_hook(Box::new(move |info| {
        record("the Teachouse desktop client panicked", &info.to_string());
        default(info);
    }));
}

/// Appends one report to the log and to stderr.
///
/// Best effort on both: a report that cannot be filed is still worth printing,
/// and a process already failing has nothing to gain from failing louder.
fn record(headline: &str, detail: &str) {
    let report = format!(
        "\n{headline}\n{detail}\nversion: {}\nos: {} {}\ndata directory: {}\nbacktrace: {}\n",
        env!("CARGO_PKG_VERSION"),
        std::env::consts::OS,
        std::env::consts::ARCH,
        DATA_DIR
            .get()
            .map_or_else(|| "unresolved".to_owned(), |dir| dir.display().to_string()),
        std::backtrace::Backtrace::capture()
    );
    eprint!("{report}");
    let Some(data_dir) = DATA_DIR.get() else {
        return;
    };
    let path = log_path(data_dir);
    match OpenOptions::new().create(true).append(true).open(&path) {
        Ok(mut file) => {
            if let Err(why) = file.write_all(report.as_bytes()) {
                eprintln!("cannot append to {}: {why}", path.display());
            }
        }
        Err(why) => eprintln!("cannot open {}: {why}", path.display()),
    }
}

/// The error and everything under it.
///
/// Tauri reports a `setup` failure as one opaque wrapper around the error the
/// closure returned, so the message a reader needs is never the outermost one.
fn chain(error: &dyn core::error::Error) -> String {
    let mut out = error.to_string();
    let mut cursor = error.source();
    while let Some(next) = cursor {
        out.push_str("\n  caused by: ");
        out.push_str(&next.to_string());
        cursor = next.source();
    }
    out
}

#[cfg(test)]
mod tests {
    use super::{chain, log_path, OPENING, STARTUP_LOG};

    #[derive(Debug)]
    struct Inner;

    impl core::fmt::Display for Inner {
        fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
            f.write_str("the data directory is read-only")
        }
    }

    impl core::error::Error for Inner {}

    #[derive(Debug)]
    struct Outer(Inner);

    impl core::fmt::Display for Outer {
        fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
            f.write_str("setup failed")
        }
    }

    impl core::error::Error for Outer {
        fn source(&self) -> Option<&(dyn core::error::Error + 'static)> {
            Some(&self.0)
        }
    }

    #[test]
    fn the_report_carries_the_cause_and_not_only_the_wrapper() {
        let text = chain(&Outer(Inner));
        assert!(
            text.contains("the data directory is read-only"),
            "Tauri wraps a setup error in an opaque outer error, so a report holding only the \
             outer message names no cause a reader can act on: {text}"
        );
    }

    #[test]
    fn the_log_sits_beside_the_device_identity() {
        let path = log_path(std::path::Path::new("/data/io.teachouse.desktop"));
        assert_eq!(
            path,
            std::path::Path::new("/data/io.teachouse.desktop/startup.log"),
            "the documented Windows path is the data directory joined with {STARTUP_LOG}, and a \
             reader told to look there finds nothing if this moves"
        );
        assert!(
            !OPENING.is_empty(),
            "an empty opening line would make a successful launch indistinguishable from a \
             process that never wrote one"
        );
    }
}
