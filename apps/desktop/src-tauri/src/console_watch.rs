//! Whether the console actually appeared after the window was sent to it.
//!
//! The bundled start page reaches `/healthz` first and only then hands the
//! window to the console, so what it can promise is that the server answered
//! a moment ago. It cannot promise that the console then draws: once the
//! window has left for the console's origin the bundled page is gone, and a
//! navigation that stalls, lands on the WebView's own error page, or loads a
//! document that never renders leaves the seller looking at a white window
//! with nothing to press. Android has no address bar and no reload, so that
//! white window lasts until the app is killed from the recent-apps list.
//!
//! So the window is looked at from here, from the moment it was sent until
//! [`GIVE_UP_AFTER`]: a console that has drawn anything ends the watch, and
//! anything else by then — or the WebView's own error page at any point —
//! returns the window to the bundled page, which says why and offers Retry.
//! The decision is [`verdict`], a pure function, so every branch is tested
//! without a WebView; [`watch`] only asks the questions and acts on the answer.

use std::time::Duration;

use serde::Deserialize;
use tauri::Url;

/// How long the console has to draw before the window is taken back.
///
/// Measured from the moment the window was sent, which is after `/healthz`
/// answered, so this is the console's own load and not the connection check.
pub(crate) const GIVE_UP_AFTER: Duration = Duration::from_secs(8);

/// How often the window is looked at until then.
pub(crate) const LOOK_EVERY: Duration = Duration::from_secs(1);

/// How long a look may take to be answered. A WebView that cannot evaluate
/// a one-line script in this long is not showing the seller anything either.
const ANSWER_WITHIN: Duration = Duration::from_secs(2);

/// The query the bundled page reads to show its failure state at once rather
/// than trying again by itself, which would loop.
pub(crate) const REASON_QUERY: (&str, &str) = ("reason", "unreachable");

/// The one expression evaluated in the window: where it is and whether the
/// page there has drawn any text. Text rather than elements, because the
/// console's shell is present in the markup before its first load resolves,
/// and a stalled load is exactly the case that shows it empty; every screen
/// the console can actually reach — sign-in, the catalogue, its own error
/// pages — carries words.
pub(crate) const LOOK_SCRIPT: &str = "JSON.stringify({href: String(location.href), \
     drawn: !!(document.body && document.body.innerText && document.body.innerText.trim())})";

/// What one look found.
#[derive(Debug, Clone, PartialEq, Eq, Deserialize)]
pub(crate) struct Look {
    /// The document's own address.
    pub href: String,
    /// Whether it has drawn any text.
    pub drawn: bool,
}

/// What to do after a look.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum Verdict {
    /// The console is up; stop watching.
    Shown,
    /// Not yet; look again.
    Wait,
    /// Take the window back to the bundled page.
    Unreachable,
}

/// The decision, from one look (or none, when the WebView did not answer)
/// and the time since the window was sent.
///
/// - A console-origin document with text is up, whenever it is seen.
/// - A document on neither the console's origin nor the bundled page's is
///   the WebView's own error page (`chrome-error://`, `about:blank` after a
///   failed load, a `data:` page), which cannot recover by itself, so it is
///   taken back at once rather than at the deadline.
/// - Everything else — still on the bundled page while the navigation is
///   pending, on the console's origin without text yet, or no answer — waits
///   until [`GIVE_UP_AFTER`] and is taken back then.
pub(crate) fn verdict(
    look: Option<&Look>,
    console: &Url,
    start: &Url,
    elapsed: Duration,
) -> Verdict {
    if let Some(look) = look {
        match Url::parse(&look.href) {
            Ok(at) if same_origin(&at, console) => {
                if look.drawn {
                    return Verdict::Shown;
                }
            }
            Ok(at) if same_origin(&at, start) => {}
            _ => return Verdict::Unreachable,
        }
    }
    if elapsed >= GIVE_UP_AFTER {
        Verdict::Unreachable
    } else {
        Verdict::Wait
    }
}

fn same_origin(a: &Url, b: &Url) -> bool {
    a.origin() == b.origin()
}

/// The bundled page's address with the failure reason on it, from the
/// address the window had before it was sent to the console. Any query the
/// page already carried (a previous failure) is replaced, not appended to.
pub(crate) fn taken_back(start: &Url) -> Url {
    let mut url = start.clone();
    url.set_fragment(None);
    url.query_pairs_mut()
        .clear()
        .append_pair(REASON_QUERY.0, REASON_QUERY.1);
    url
}

/// One look at the window, or `None` when it did not answer in time or
/// answered with something that is not a look.
async fn look<R: tauri::Runtime>(window: &tauri::WebviewWindow<R>) -> Option<Look> {
    // A bounded channel rather than a oneshot: the callback is `Fn`, and
    // `try_send` takes `&self`, so nothing has to be moved out of it.
    let (tx, mut rx) = tokio::sync::mpsc::channel::<String>(1);
    window
        .eval_with_callback(LOOK_SCRIPT, move |answer| {
            tx.try_send(answer).ok();
        })
        .ok()?;
    let answer = tokio::time::timeout(ANSWER_WITHIN, rx.recv())
        .await
        .ok()??;
    read_look(&answer)
}

/// The look from what the WebView handed the callback. Android's
/// `evaluateJavascript` serialises the expression's value as JSON, and the
/// expression is itself a JSON string, so the answer arrives quoted twice;
/// an engine that hands the string back unserialised is read as well.
pub(crate) fn read_look(answer: &str) -> Option<Look> {
    match serde_json::from_str::<String>(answer) {
        Ok(inner) => serde_json::from_str(&inner).ok(),
        Err(_) => serde_json::from_str(answer).ok(),
    }
}

/// Watch the window from the moment it was sent to `console` until the
/// console draws or the verdict takes it back to `start`.
pub(crate) async fn watch<R: tauri::Runtime>(
    window: tauri::WebviewWindow<R>,
    console: Url,
    start: Url,
    how: crate::StartUpNav,
) {
    let sent = tokio::time::Instant::now();
    loop {
        tokio::time::sleep(LOOK_EVERY).await;
        let seen = look(&window).await;
        match verdict(seen.as_ref(), &console, &start, sent.elapsed()) {
            Verdict::Shown => return,
            Verdict::Wait => {}
            Verdict::Unreachable => {
                eprintln!(
                    "the console did not appear within {}s (last seen: {}); showing the \
                     connection page",
                    GIVE_UP_AFTER.as_secs(),
                    seen.map_or_else(|| "no answer".to_owned(), |look| look.href)
                );
                if let Err(why) = crate::show_console(&window, taken_back(&start), how) {
                    eprintln!("the connection page could not be shown: {why}");
                }
                return;
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::{read_look, taken_back, verdict, Look, Verdict, GIVE_UP_AFTER};
    use std::time::Duration;
    use tauri::Url;

    fn console() -> Url {
        "https://teachouse.io/resources".parse().unwrap()
    }

    fn start() -> Url {
        "http://tauri.localhost/unreachable.html".parse().unwrap()
    }

    fn at(href: &str, drawn: bool) -> Look {
        Look {
            href: href.to_owned(),
            drawn,
        }
    }

    const EARLY: Duration = Duration::from_secs(2);

    /// A console that has drawn is up at any moment, even past the deadline:
    /// the deadline is for giving up on one that has not, not for a slow one
    /// that made it.
    #[test]
    fn a_drawn_console_is_up_whenever_it_is_seen() {
        for elapsed in [EARLY, GIVE_UP_AFTER, GIVE_UP_AFTER * 3] {
            assert_eq!(
                verdict(
                    Some(&at("https://teachouse.io/login?next=%2F", true)),
                    &console(),
                    &start(),
                    elapsed
                ),
                Verdict::Shown,
                "{elapsed:?}"
            );
        }
    }

    /// The white window: on the console's origin with nothing drawn waits,
    /// and is taken back at the deadline rather than left white.
    #[test]
    fn a_blank_console_is_taken_back_at_the_deadline() {
        let blank = at("https://teachouse.io/resources", false);
        assert_eq!(
            verdict(Some(&blank), &console(), &start(), EARLY),
            Verdict::Wait
        );
        assert_eq!(
            verdict(Some(&blank), &console(), &start(), GIVE_UP_AFTER),
            Verdict::Unreachable
        );
    }

    /// Still on the bundled page is a navigation in flight, and a stalled one
    /// is taken back at the deadline; so is a WebView that does not answer.
    #[test]
    fn a_pending_navigation_or_no_answer_waits_until_the_deadline() {
        let pending = at("http://tauri.localhost/unreachable.html", true);
        for look in [Some(&pending), None] {
            assert_eq!(verdict(look, &console(), &start(), EARLY), Verdict::Wait);
            assert_eq!(
                verdict(look, &console(), &start(), GIVE_UP_AFTER),
                Verdict::Unreachable
            );
        }
    }

    /// The WebView's own error page cannot recover by itself, so it is taken
    /// back at once, drawn or not.
    #[test]
    fn the_webviews_error_page_is_taken_back_at_once() {
        for href in [
            "chrome-error://chromewebdata/",
            "about:blank",
            "data:text/html,oops",
            "https://evil.example/resources",
            "not a url",
        ] {
            assert_eq!(
                verdict(Some(&at(href, true)), &console(), &start(), EARLY),
                Verdict::Unreachable,
                "{href}"
            );
        }
    }

    /// Another port on the same host is another origin: a development
    /// console on `127.0.0.1:8080` is not the bundled page on `:80`.
    #[test]
    fn origin_means_scheme_host_and_port() {
        let dev: Url = "http://127.0.0.1:8080/resources".parse().unwrap();
        let other_port = at("http://127.0.0.1:9090/resources", true);
        assert_eq!(
            verdict(Some(&other_port), &dev, &start(), EARLY),
            Verdict::Unreachable
        );
    }

    /// The reason replaces whatever query the bundled page had, so a second
    /// failure does not stack a second reason onto the first.
    #[test]
    fn the_page_is_taken_back_with_one_reason() {
        let first = taken_back(&start());
        assert_eq!(
            first.as_str(),
            "http://tauri.localhost/unreachable.html?reason=unreachable"
        );
        assert_eq!(taken_back(&first), first);
        let windows: Url = "https://tauri.localhost/unreachable.html#x"
            .parse()
            .unwrap();
        assert_eq!(
            taken_back(&windows).as_str(),
            "https://tauri.localhost/unreachable.html?reason=unreachable"
        );
    }

    /// The answer is read whether the WebView quoted it once more or not,
    /// and anything else is no look rather than a guess.
    #[test]
    fn the_answer_is_read_quoted_or_not() {
        let raw = r#"{"href":"https://teachouse.io/resources","drawn":false}"#;
        let quoted = serde_json::to_string(raw).unwrap();
        let expected = at("https://teachouse.io/resources", false);
        assert_eq!(read_look(raw), Some(expected.clone()));
        assert_eq!(read_look(&quoted), Some(expected));
        for junk in ["null", "", "\"loading\"", "{\"href\":1}"] {
            assert_eq!(read_look(junk), None, "{junk}");
        }
    }
}
