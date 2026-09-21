//! Running work off the request's own task, supervised.
//!
//! `clippy.toml` disallows `tokio::task::spawn_blocking` and `tokio::spawn`
//! and names `spawn_supervised_blocking` and `spawn_supervised` as what to
//! use instead. Nothing in this repository had built either, so the first
//! caller that needed one found a ban pointing at a function that did not
//! exist. These are those functions.
//!
//! The ban's reason is that a dropped `JoinHandle` discards the panic inside
//! it, so a task that died leaves no trace and whatever spawned it waits or
//! succeeds wrongly. Neither wrapper here can do that: each holds the handle
//! of the task it started and turns a `JoinError` into something visible — a
//! value the caller must handle, or a line on stderr naming what died. The
//! `expect`s for the lint therefore live here, at sites whose whole purpose
//! is satisfying the reason behind it, rather than being repeated at every
//! call.

/// The blocking work did not finish: it panicked, or the runtime cancelled it
/// while it ran.
///
/// Carries the message rather than the `JoinError`, because the caller's only
/// use for it is a fault's internal detail and `JoinError` is a tokio type
/// this crate does not otherwise expose.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Unfinished(pub String);

impl core::fmt::Display for Unfinished {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        f.write_str(&self.0)
    }
}

impl core::error::Error for Unfinished {}

/// Runs `work` on a blocking thread and waits for it, so a synchronous span of
/// CPU does not hold an async worker for its duration.
///
/// For work measured in milliseconds rather than microseconds: a search over a
/// compiled-in corpus, a parse, a hash. Not for anything that waits on the
/// network or the database, which have their own async paths.
pub async fn spawn_supervised_blocking<F, T>(work: F) -> Result<T, Unfinished>
where
    F: FnOnce() -> T + Send + 'static,
    T: Send + 'static,
{
    #[expect(
        clippy::disallowed_methods,
        reason = "this is the supervised wrapper the ban names: the handle is awaited below and \
                  a JoinError becomes an error the caller must handle, so no panic is discarded"
    )]
    let handle = tokio::task::spawn_blocking(work);
    handle
        .await
        .map_err(|error| Unfinished(format!("blocking work did not finish: {error}")))
}

/// Runs `work` for as long as it lives, without the caller waiting for it,
/// and says on stderr if it ever stops by panicking.
///
/// For the long-lived background task a start-up spawns and nothing awaits:
/// a drain loop, a sweeper. `what` names it in that line, because a task
/// nobody holds a handle to is otherwise unidentifiable once it is gone.
///
/// The work runs in a task of its own beneath a supervisor task, which is
/// what converts its panic into a `JoinError` the supervisor can report. The
/// supervisor's own body only awaits and prints, so the one handle this
/// function drops belongs to a task that has nothing to panic about.
pub fn spawn_supervised<F>(what: &'static str, work: F)
where
    F: core::future::Future<Output = ()> + Send + 'static,
{
    #[expect(
        clippy::disallowed_methods,
        reason = "this is the supervised wrapper the ban names: the dropped handle is the \
                  supervisor's, whose body cannot panic, and the work's own handle is awaited"
    )]
    let supervisor = tokio::spawn(async move {
        #[expect(
            clippy::disallowed_methods,
            reason = "the supervised task; its handle is awaited on the next line, so a panic \
                      becomes a JoinError rather than silence"
        )]
        let handle = tokio::spawn(work);
        if let Err(error) = handle.await {
            eprintln!("tam-api: {what} did not finish: {error}");
        }
    });
    drop(supervisor);
}

#[cfg(test)]
mod tests {
    use super::{spawn_supervised, spawn_supervised_blocking};

    #[tokio::test]
    async fn work_that_finishes_answers_its_value() {
        let answer = spawn_supervised_blocking(|| 2 + 2)
            .await
            .expect("the work finished");
        assert_eq!(answer, 4);
    }

    /// The whole reason the ban exists: a panic inside blocking work must reach
    /// the caller rather than vanishing with a dropped handle.
    #[tokio::test]
    async fn a_panic_reaches_the_caller_rather_than_vanishing() {
        let outcome = spawn_supervised_blocking(|| panic!("the corpus is on fire")).await;
        let failure = outcome.expect_err("a panicking task is not a success");
        assert!(
            failure.0.contains("did not finish"),
            "the caller is told the work did not finish, rather than being handed a value"
        );
    }

    /// The detached form still runs its work: the caller holds no handle, so
    /// the only evidence it ran is what it did.
    #[tokio::test]
    async fn detached_work_runs_without_the_caller_holding_it() {
        let (sender, receiver) = tokio::sync::oneshot::channel();
        spawn_supervised("the test's work", async move {
            sender.send(4).expect("the receiver is waiting");
        });
        assert_eq!(receiver.await.expect("the work ran"), 4);
    }
}
