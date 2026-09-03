//! The `!Send` boundary, crossed once.
//!
//! Every marketplace transport in this workspace returns a future bound by
//! `+ Send` (`crates/tam-marketplace/src/transport.rs:293-297`). A browser
//! request cannot satisfy that bound directly, because `JsFuture` holds an
//! `Rc<RefCell<_>>` and is therefore `!Send` however `Send` its output is.
//!
//! The bridge answers that without editing the shared bound. The `!Send`
//! future is spawned on the thread that created it, which never moves it, and
//! the caller receives a `futures_channel::oneshot::Receiver`, which is `Send`
//! whenever its payload is. The `!Send` half never crosses the bound; only its
//! answer does.
//!
//! Taken in preference to the `MaybeSend` cfg shim as decision 4 of
//! `docs/research/rethink/oxichrome-extension-client.md`: this touches no
//! shared crate, where the shim would edit 42 sites that every server binary
//! compiles.

use core::future::Future;
use core::pin::Pin;
use core::task::{Context, Poll};
use futures_channel::oneshot;

/// A future the browser drives on the thread that created it.
///
/// Boxed because the spawner is a parameter rather than a fixed call: wasm
/// passes `wasm_bindgen_futures::spawn_local` and the tests pass a hand-driven
/// executor, so the two have to agree on one concrete argument type.
pub type LocalTask = Pin<Box<dyn Future<Output = ()>>>;

/// The spawned task was dropped before it answered.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Cancelled;

/// Runs `work` on the current thread and returns a `Send` future for its
/// output.
///
/// The `Send` half of the bridge: a future for an answer computed elsewhere.
///
/// Named rather than `impl Future`, and that is the load the type carries. A
/// return-position `impl Trait` captures every type parameter in scope, so it
/// would capture both the spawner and the `!Send` future — precisely what must
/// not be reachable through the value handed back. This struct holds one
/// receiver and can hold nothing else.
#[derive(Debug)]
pub struct Answer<T>(oneshot::Receiver<T>);

impl<T> Future for Answer<T> {
    type Output = Result<T, Cancelled>;

    fn poll(mut self: Pin<&mut Self>, context: &mut Context<'_>) -> Poll<Self::Output> {
        Pin::new(&mut self.0)
            .poll(context)
            .map(|received| received.map_err(|_dropped| Cancelled))
    }
}

/// `spawn` is a parameter so the pattern is exercisable on the host, where
/// `spawn_local` does not exist; that is what makes the tests below possible
/// without a browser.
pub fn bridge<T, F, S>(spawn: S, work: F) -> Answer<T>
where
    T: Send + 'static,
    F: Future<Output = T> + 'static,
    S: FnOnce(LocalTask),
{
    let (answer, answered) = oneshot::channel();
    spawn(Box::pin(async move {
        let outcome = work.await;
        // A gone receiver is the caller having stopped waiting: there is no
        // one to answer and nothing to report.
        let _unanswered = answer.send(outcome);
    }));
    Answer(answered)
}

#[cfg(test)]
mod tests {
    use super::{bridge, Cancelled, LocalTask};
    use core::future::Future;
    use core::pin::Pin;
    use core::task::{Context, Poll, Waker};
    use std::cell::Cell;
    use std::rc::Rc;

    /// Polls a future on this thread until it is ready.
    ///
    /// Every future driven here is ready on its first poll or made ready by an
    /// earlier one, so a waker that records nothing suffices and the bound
    /// exists to fail the test rather than hang it.
    fn drive<T>(mut future: Pin<Box<dyn Future<Output = T>>>) -> Option<T> {
        let mut context = Context::from_waker(Waker::noop());
        for _ in 0..8_u8 {
            if let Poll::Ready(value) = future.as_mut().poll(&mut context) {
                return Some(value);
            }
        }
        None
    }

    fn assert_send<T: Send>(_value: &T) {}

    #[test]
    fn a_non_send_future_answers_through_a_send_receiver() {
        let mut spawned: Option<LocalTask> = None;
        let answered = bridge(|task| spawned = Some(task), async {
            // `Rc` is exactly why the real future is `!Send`: `JsFuture`
            // holds one.
            let counter = Rc::new(Cell::new(0_u32));
            counter.set(counter.get() + 1);
            counter.get()
        });

        assert_send(&answered);

        let task = spawned.expect("the bridge spawns before it returns");
        assert!(
            drive(task).is_some(),
            "the spawned local task should run to completion"
        );
        assert_eq!(
            drive(Box::pin(answered)),
            Some(Ok(1)),
            "the answer should cross the boundary unchanged"
        );
    }

    #[test]
    fn a_dropped_task_reports_cancellation_rather_than_hanging() {
        let mut spawned: Option<LocalTask> = None;
        let answered = bridge(|task| spawned = Some(task), async { 7_u32 });

        // The browser evicted the worker before the task was driven.
        drop(spawned);

        assert_eq!(
            drive(Box::pin(answered)),
            Some(Err(Cancelled)),
            "a dropped sender should resolve as cancelled"
        );
    }
}
