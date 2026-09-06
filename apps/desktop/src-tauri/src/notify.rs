//! The one notification a cycle earns: what this device tells the seller, on
//! the seller's own screen, about the work it just did.
//!
//! Raised by the device about its own run and never by the server about
//! somebody else's, which is what keeps it truthful when a seller has two
//! machines: the completion mail from the server covers the account, and this
//! covers the machine. One per cycle that settled anything and never per item,
//! so a fifty-item run raises one.
//!
//! [`Notifier`] is the seam, exactly as [`crate::session::SessionStore`] and
//! [`crate::heartbeat::ControlPlane`] are. [`Silent`] raises nothing and is
//! what a state built without one gets; [`DeviceNotifier`] is the plugin,
//! reached through [`Surface`] so the permission path can be driven on the
//! host, where the plugin answers `Granted` without asking and nothing but a
//! phone can answer otherwise.

use core::future::Future;
use core::pin::Pin;
use core::sync::atomic::{AtomicBool, Ordering};

use tam_domain::ItemOutcome;
use tam_types::Marketplace;
use tauri::plugin::PermissionState;
use tauri_plugin_notification::NotificationExt as _;

use crate::scheduler::TickReport;
use crate::state::WorkEvent;

pub type NotifyFuture<'a> = Pin<Box<dyn Future<Output = Result<(), NotifyError>> + Send + 'a>>;

/// The platform's refusal, as a sentence. Never fatal to the cycle that met
/// it: what the device did is already recorded by then, and a notification
/// that could not be shown is one the seller reads on the console instead.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct NotifyError(pub String);

impl core::fmt::Display for NotifyError {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        f.write_str(&self.0)
    }
}

impl core::error::Error for NotifyError {}

/// What raises the notification, given what a cycle settled.
pub trait Notifier: Send + Sync {
    fn notify<'a>(&'a self, summary: &'a CycleSummary) -> NotifyFuture<'a>;
}

/// The notifier a state built without one gets: it raises nothing.
///
/// Silence rather than an error, unlike [`crate::heartbeat::Offline`]: a
/// notification is a courtesy the cycle owes nobody, and a build with no
/// surface to show one on has done nothing wrong.
#[derive(Debug, Default)]
pub struct Silent;

impl Notifier for Silent {
    fn notify<'a>(&'a self, _summary: &'a CycleSummary) -> NotifyFuture<'a> {
        Box::pin(core::future::ready(Ok(())))
    }
}

/// How many items settled as each outcome, in the ledger's own vocabulary.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub struct OutcomeCounts {
    pub succeeded: u32,
    pub degraded: u32,
    pub failed: u32,
    pub ambiguous: u32,
    pub skipped: u32,
    pub blocked: u32,
}

impl OutcomeCounts {
    fn count(&mut self, outcome: ItemOutcome) {
        match outcome {
            ItemOutcome::Succeeded => self.succeeded += 1,
            ItemOutcome::Degraded => self.degraded += 1,
            ItemOutcome::Failed => self.failed += 1,
            ItemOutcome::Ambiguous => self.ambiguous += 1,
            ItemOutcome::Skipped => self.skipped += 1,
            ItemOutcome::Blocked => self.blocked += 1,
        }
    }

    #[must_use]
    pub const fn total(self) -> u32 {
        self.succeeded + self.degraded + self.failed + self.ambiguous + self.skipped + self.blocked
    }

    /// The counts that are not zero, each under the console's own word for
    /// the outcome and in the console's own order: the first six segments of
    /// `web/src/lib/outcome.ts`.
    fn stated(self) -> Vec<(&'static str, u32)> {
        [
            ("succeeded", self.succeeded),
            ("degraded", self.degraded),
            ("failed", self.failed),
            ("ambiguous", self.ambiguous),
            ("skipped", self.skipped),
            ("blocked", self.blocked),
        ]
        .into_iter()
        .filter(|(_, count)| *count > 0)
        .collect()
    }

    /// "10 succeeded, 2 failed".
    fn sentence(self) -> String {
        self.stated()
            .into_iter()
            .map(|(word, count)| format!("{count} {word}"))
            .collect::<Vec<_>>()
            .join(", ")
    }
}

/// What one cycle settled, per marketplace, in the order the tick worked them.
///
/// Built from the tick's finished report rather than from the events as they
/// happen, which is what makes "one per cycle" a property of the type: there
/// is no way to hold one before the loop is over.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CycleSummary {
    settled: Vec<(Marketplace, OutcomeCounts)>,
}

impl CycleSummary {
    /// `None` for a cycle that settled nothing, which has nothing to say. A
    /// pull that found nothing due, an item that parked or was abandoned, and
    /// a marketplace refused before the pull all leave the seller's screen
    /// alone.
    #[must_use]
    pub fn of(report: &TickReport) -> Option<Self> {
        let settled_items = report
            .events
            .iter()
            .filter_map(|(marketplace, event)| match *event {
                WorkEvent::Settled { outcome, .. } => Some((*marketplace, outcome)),
                WorkEvent::Idle
                | WorkEvent::Held { .. }
                | WorkEvent::Started { .. }
                | WorkEvent::Parked { .. }
                | WorkEvent::Abandoned { .. }
                | WorkEvent::Blocked { .. }
                | WorkEvent::Failed { .. } => None,
            });
        let mut settled: Vec<(Marketplace, OutcomeCounts)> = Vec::new();
        for (marketplace, outcome) in settled_items {
            if let Some((_, counts)) = settled.iter_mut().find(|(place, _)| *place == marketplace) {
                counts.count(outcome);
            } else {
                let mut counts = OutcomeCounts::default();
                counts.count(outcome);
                settled.push((marketplace, counts));
            }
        }
        (!settled.is_empty()).then_some(Self { settled })
    }

    #[must_use]
    pub fn settled(&self) -> &[(Marketplace, OutcomeCounts)] {
        &self.settled
    }

    /// The notice, in the words the server's completion mail uses, so a seller
    /// reading both reads one thing. The figure rule is the mail's: a cycle
    /// where everything succeeded states one figure, and a cycle where
    /// anything did not states both, because "11 resources updated" over nine
    /// succeeded and two failed reads as an achievement and is not one. The
    /// body is the counts, each under the console's own word for the outcome.
    #[must_use]
    pub fn notice(&self) -> Notice {
        let total: u32 = self.settled.iter().map(|(_, counts)| counts.total()).sum();
        let done: u32 = self
            .settled
            .iter()
            .map(|(_, counts)| counts.succeeded)
            .sum();
        let word = if total == 1 { "resource" } else { "resources" };
        let names: Vec<&str> = self
            .settled
            .iter()
            .map(|(marketplace, _)| marketplace_name(*marketplace))
            .collect();
        let place = joined(&names);
        let title = if done == total {
            format!("Your {place} sync finished — {total} {word} updated")
        } else {
            format!("Your {place} sync finished — {done} of {total} {word} updated")
        };
        let body = match self.settled.as_slice() {
            [(_, counts)] => counts.sentence(),
            several => several
                .iter()
                .map(|(marketplace, counts)| {
                    let name = marketplace_name(*marketplace);
                    let counts = counts.sentence();
                    format!("{name}: {counts}")
                })
                .collect::<Vec<_>>()
                .join(". "),
        };
        Notice { title, body }
    }
}

/// A title and a body, which is what a notification carries on every platform
/// this builds for.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Notice {
    pub title: String,
    pub body: String,
}

/// What the seller calls the marketplace: the acronym
/// `web/src/lib/platforms.ts` renders on every tile, rather than the enum's
/// own spelling.
const fn marketplace_name(marketplace: Marketplace) -> &'static str {
    match marketplace {
        Marketplace::Tes => "TES",
        Marketplace::Etsy => "Etsy",
        Marketplace::Tpt => "TPT",
    }
}

/// "TPT", "TPT and TES", "TPT, TES and Etsy".
fn joined(names: &[&str]) -> String {
    match names {
        [] => String::new(),
        [one] => (*one).to_owned(),
        [init @ .., last] => {
            let init = init.join(", ");
            format!("{init} and {last}")
        }
    }
}

/// The three plugin calls a notice needs, behind a seam so the permission path
/// can be driven on the host.
pub trait Surface: Send + Sync {
    fn permission(&self) -> Result<PermissionState, NotifyError>;
    fn request_permission(&self) -> Result<PermissionState, NotifyError>;
    fn show(&self, notice: &Notice) -> Result<(), NotifyError>;
}

/// The plugin's notifier, asking for the permission at the first cycle that
/// has something to say rather than at launch, so the prompt arrives with a
/// reason attached.
///
/// Asked once per process. A seller who dismissed the prompt without
/// answering leaves a platform that still says `Prompt`, and asking again at
/// every cycle would turn one courtesy into an hourly nag; a seller who
/// refused is `Denied` and is not asked at all. Either leaves the cycle
/// exactly as it was: the work is done and recorded, and the completion mail
/// is the server's. On every desktop the plugin answers `Granted` without
/// asking, so the path below is the phone's and costs the desktop one call.
pub struct DeviceNotifier<S> {
    surface: S,
    asked: AtomicBool,
}

impl<S: Surface> DeviceNotifier<S> {
    #[must_use]
    pub const fn new(surface: S) -> Self {
        Self {
            surface,
            asked: AtomicBool::new(false),
        }
    }

    fn raise(&self, summary: &CycleSummary) -> Result<(), NotifyError> {
        let notice = summary.notice();
        match self.surface.permission()? {
            PermissionState::Granted => self.surface.show(&notice),
            PermissionState::Denied => Ok(()),
            PermissionState::Prompt | PermissionState::PromptWithRationale => {
                if self.asked.swap(true, Ordering::SeqCst) {
                    return Ok(());
                }
                match self.surface.request_permission()? {
                    PermissionState::Granted => self.surface.show(&notice),
                    PermissionState::Denied
                    | PermissionState::Prompt
                    | PermissionState::PromptWithRationale => Ok(()),
                }
            }
        }
    }
}

impl<S: Surface> Notifier for DeviceNotifier<S> {
    fn notify<'a>(&'a self, summary: &'a CycleSummary) -> NotifyFuture<'a> {
        Box::pin(core::future::ready(self.raise(summary)))
    }
}

/// The plugin itself, reached through the application handle on every call
/// because the plugin's own handle is managed state rather than a value this
/// can hold.
pub struct PluginSurface<R: tauri::Runtime>(tauri::AppHandle<R>);

impl<R: tauri::Runtime> PluginSurface<R> {
    #[must_use]
    pub const fn new(app: tauri::AppHandle<R>) -> Self {
        Self(app)
    }
}

impl<R: tauri::Runtime> Surface for PluginSurface<R> {
    fn permission(&self) -> Result<PermissionState, NotifyError> {
        self.0
            .notification()
            .permission_state()
            .map_err(|why| NotifyError(why.to_string()))
    }

    fn request_permission(&self) -> Result<PermissionState, NotifyError> {
        self.0
            .notification()
            .request_permission()
            .map_err(|why| NotifyError(why.to_string()))
    }

    fn show(&self, notice: &Notice) -> Result<(), NotifyError> {
        self.0
            .notification()
            .builder()
            .title(notice.title.clone())
            .body(notice.body.clone())
            .show()
            .map_err(|why| NotifyError(why.to_string()))
    }
}

#[cfg(test)]
pub(crate) mod testing {
    use super::{CycleSummary, Notice, Notifier, NotifyFuture};

    /// Keeps every notice it was handed, so a test can read what the seller
    /// would have.
    #[derive(Debug, Default)]
    pub(crate) struct RecordingNotifier {
        notices: tokio::sync::Mutex<Vec<Notice>>,
    }

    impl RecordingNotifier {
        pub(crate) fn new() -> Self {
            Self::default()
        }

        pub(crate) async fn notices(&self) -> Vec<Notice> {
            self.notices.lock().await.clone()
        }
    }

    impl Notifier for RecordingNotifier {
        fn notify<'a>(&'a self, summary: &'a CycleSummary) -> NotifyFuture<'a> {
            Box::pin(async move {
                self.notices.lock().await.push(summary.notice());
                Ok(())
            })
        }
    }
}

#[cfg(test)]
mod tests {
    use super::{
        CycleSummary, DeviceNotifier, Notice, Notifier, NotifyError, OutcomeCounts, Surface,
    };
    use crate::scheduler::TickReport;
    use crate::state::{BlockReason, WorkEvent};
    use core::sync::atomic::{AtomicUsize, Ordering};
    use tam_domain::ItemOutcome;
    use tam_types::Marketplace;
    use tauri::plugin::PermissionState;

    fn settled(item: &str, outcome: ItemOutcome) -> WorkEvent {
        WorkEvent::Settled {
            item: item.to_owned(),
            outcome,
        }
    }

    /// Every event a tick can record, with three settlements among them.
    fn a_busy_report() -> TickReport {
        TickReport {
            events: vec![
                (
                    Marketplace::Tpt,
                    WorkEvent::Started {
                        item: "a".to_owned(),
                    },
                ),
                (Marketplace::Tpt, settled("a", ItemOutcome::Succeeded)),
                (
                    Marketplace::Tpt,
                    WorkEvent::Started {
                        item: "b".to_owned(),
                    },
                ),
                (Marketplace::Tpt, settled("b", ItemOutcome::Failed)),
                (
                    Marketplace::Tpt,
                    WorkEvent::Parked {
                        item: "c".to_owned(),
                        blocked_on: "captcha".to_owned(),
                    },
                ),
                (
                    Marketplace::Tpt,
                    WorkEvent::Abandoned {
                        item: "d".to_owned(),
                        reason: "stalled".to_owned(),
                    },
                ),
                (
                    Marketplace::Tes,
                    WorkEvent::Held {
                        next_poll_ms: 5_000,
                    },
                ),
                (Marketplace::Tes, settled("e", ItemOutcome::Succeeded)),
                (
                    Marketplace::Etsy,
                    WorkEvent::Blocked {
                        reason: BlockReason::NoSession,
                    },
                ),
                (
                    Marketplace::Etsy,
                    WorkEvent::Failed {
                        detail: "502".to_owned(),
                    },
                ),
            ],
        }
    }

    #[test]
    fn a_summary_counts_what_settled_per_marketplace_and_nothing_else() {
        let summary = CycleSummary::of(&a_busy_report()).expect("three items settled");
        assert_eq!(
            summary.settled(),
            &[
                (
                    Marketplace::Tpt,
                    OutcomeCounts {
                        succeeded: 1,
                        failed: 1,
                        ..OutcomeCounts::default()
                    }
                ),
                (
                    Marketplace::Tes,
                    OutcomeCounts {
                        succeeded: 1,
                        ..OutcomeCounts::default()
                    }
                ),
            ],
            "a parked, an abandoned, a held, a blocked and a failed event settled nothing, \
             and a marketplace with no settlement has no entry"
        );
    }

    #[test]
    fn a_cycle_that_settled_nothing_has_no_summary() {
        let quiet = TickReport {
            events: vec![
                (Marketplace::Tpt, WorkEvent::Idle),
                (
                    Marketplace::Tes,
                    WorkEvent::Parked {
                        item: "c".to_owned(),
                        blocked_on: "captcha".to_owned(),
                    },
                ),
            ],
        };
        assert_eq!(
            CycleSummary::of(&quiet),
            None,
            "nothing due and an item parked have nothing to say"
        );
        assert_eq!(
            CycleSummary::of(&TickReport::default()),
            None,
            "an empty tick has nothing to say"
        );
    }

    #[test]
    fn the_notice_states_one_figure_when_everything_succeeded_and_both_otherwise() {
        let all_well = TickReport {
            events: vec![
                (Marketplace::Tpt, settled("a", ItemOutcome::Succeeded)),
                (Marketplace::Tpt, settled("b", ItemOutcome::Succeeded)),
            ],
        };
        let notice = CycleSummary::of(&all_well).expect("two settled").notice();
        assert_eq!(
            notice,
            Notice {
                title: "Your TPT sync finished — 2 resources updated".to_owned(),
                body: "2 succeeded".to_owned(),
            },
            "everything succeeded, so one figure"
        );

        let one = TickReport {
            events: vec![(Marketplace::Tes, settled("a", ItemOutcome::Degraded))],
        };
        let notice = CycleSummary::of(&one).expect("one settled").notice();
        assert_eq!(
            notice,
            Notice {
                title: "Your TES sync finished — 0 of 1 resource updated".to_owned(),
                body: "1 degraded".to_owned(),
            },
            "a degraded listing is live but not updated as intended, so both figures, \
             and the singular"
        );
    }

    #[test]
    fn the_notice_names_every_count_in_the_consoles_own_word_and_every_marketplace() {
        let notice = CycleSummary::of(&a_busy_report())
            .expect("three settled")
            .notice();
        assert_eq!(
            notice.title, "Your TPT and TES sync finished — 2 of 3 resources updated",
            "one notice for the cycle names both marketplaces it worked"
        );
        assert_eq!(
            notice.body, "TPT: 1 succeeded, 1 failed. TES: 1 succeeded",
            "the counts under the console's words, per marketplace once there is more than one"
        );

        let every_outcome = TickReport {
            events: vec![
                (Marketplace::Tpt, settled("a", ItemOutcome::Blocked)),
                (Marketplace::Tpt, settled("b", ItemOutcome::Skipped)),
                (Marketplace::Tpt, settled("c", ItemOutcome::Ambiguous)),
                (Marketplace::Tpt, settled("d", ItemOutcome::Failed)),
                (Marketplace::Tpt, settled("e", ItemOutcome::Degraded)),
                (Marketplace::Tpt, settled("f", ItemOutcome::Succeeded)),
                (Marketplace::Tpt, settled("g", ItemOutcome::Succeeded)),
            ],
        };
        let notice = CycleSummary::of(&every_outcome)
            .expect("seven settled")
            .notice();
        assert_eq!(
            notice.body, "2 succeeded, 1 degraded, 1 failed, 1 ambiguous, 1 skipped, 1 blocked",
            "in the console's order rather than the order they settled in"
        );
    }

    /// A platform whose permission answers are scripted, counting what it was
    /// asked and shown.
    struct Scripted {
        state: tokio::sync::Mutex<PermissionState>,
        answer: PermissionState,
        asks: AtomicUsize,
        shows: AtomicUsize,
    }

    impl Scripted {
        fn saying(state: PermissionState, answer: PermissionState) -> Self {
            Self {
                state: tokio::sync::Mutex::new(state),
                answer,
                asks: AtomicUsize::new(0),
                shows: AtomicUsize::new(0),
            }
        }
    }

    impl Surface for Scripted {
        fn permission(&self) -> Result<PermissionState, NotifyError> {
            Ok(*self.state.try_lock().expect("no test contends for it"))
        }

        fn request_permission(&self) -> Result<PermissionState, NotifyError> {
            self.asks.fetch_add(1, Ordering::SeqCst);
            *self.state.try_lock().expect("no test contends for it") = self.answer;
            Ok(self.answer)
        }

        fn show(&self, _notice: &Notice) -> Result<(), NotifyError> {
            self.shows.fetch_add(1, Ordering::SeqCst);
            Ok(())
        }
    }

    fn something_settled() -> CycleSummary {
        CycleSummary::of(&a_busy_report()).expect("three settled")
    }

    #[tokio::test]
    async fn the_permission_is_asked_once_even_when_the_platform_keeps_saying_prompt() {
        let notifier = DeviceNotifier::new(Scripted::saying(
            PermissionState::Prompt,
            PermissionState::Prompt,
        ));
        for _ in 0..3 {
            notifier
                .notify(&something_settled())
                .await
                .expect("a dismissed prompt is not a fault");
        }
        assert_eq!(
            notifier.surface.asks.load(Ordering::SeqCst),
            1,
            "a seller who dismissed the prompt without answering is asked once, not hourly"
        );
        assert_eq!(
            notifier.surface.shows.load(Ordering::SeqCst),
            0,
            "and nothing is shown without a grant"
        );
    }

    #[tokio::test]
    async fn a_grant_at_the_prompt_shows_that_notice_and_the_next_without_asking_again() {
        let notifier = DeviceNotifier::new(Scripted::saying(
            PermissionState::PromptWithRationale,
            PermissionState::Granted,
        ));
        notifier
            .notify(&something_settled())
            .await
            .expect("granted");
        notifier
            .notify(&something_settled())
            .await
            .expect("still granted");
        assert_eq!(
            notifier.surface.asks.load(Ordering::SeqCst),
            1,
            "the prompt is raised by the first cycle with something to say and by no other"
        );
        assert_eq!(
            notifier.surface.shows.load(Ordering::SeqCst),
            2,
            "the notice that earned the prompt is shown, and so is every later one"
        );
    }

    #[tokio::test]
    async fn a_refusal_shows_nothing_asks_nothing_and_is_not_a_fault() {
        let notifier = DeviceNotifier::new(Scripted::saying(
            PermissionState::Denied,
            PermissionState::Denied,
        ));
        notifier
            .notify(&something_settled())
            .await
            .expect("a refusal leaves the cycle as it was");
        assert_eq!(
            notifier.surface.asks.load(Ordering::SeqCst),
            0,
            "a seller who refused is not asked again"
        );
        assert_eq!(
            notifier.surface.shows.load(Ordering::SeqCst),
            0,
            "and sees nothing"
        );
    }

    #[tokio::test]
    async fn a_desktop_that_needs_no_permission_shows_without_asking() {
        let notifier = DeviceNotifier::new(Scripted::saying(
            PermissionState::Granted,
            PermissionState::Granted,
        ));
        notifier.notify(&something_settled()).await.expect("shown");
        assert_eq!(
            notifier.surface.asks.load(Ordering::SeqCst),
            0,
            "the plugin answers Granted on every desktop, so no prompt is ever raised there"
        );
        assert_eq!(
            notifier.surface.shows.load(Ordering::SeqCst),
            1,
            "and the notice is shown"
        );
    }
}
