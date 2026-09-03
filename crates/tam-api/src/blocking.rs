//! Running synchronous work without holding an async worker, supervised.
//!
//! `clippy.toml` disallows `tokio::task::spawn_blocking` and names
//! `spawn_supervised_blocking` as what to use instead. Nothing in this
//! repository had built it, so the first caller that needed it found a ban
//! pointing at a function that did not exist. This is that function.
//!
//! The ban's reason is that a dropped `JoinHandle` discards the panic inside
//! it, so a task that died leaves no trace and the request that spawned it
//! waits or succeeds wrongly. This wrapper cannot do that: it awaits the
//! handle and turns a `JoinError` into a value the caller must handle, so a
//! panic surfaces as a fault rather than as silence. The one `expect` for the
//! lint therefore lives here, at a site whose whole purpose is satisfying the
//! reason behind it, rather than being repeated at every call.

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

#[cfg(test)]
mod tests {
    use super::spawn_supervised_blocking;

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
}
