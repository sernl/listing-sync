//! Read-only probe answering decision D12: can the webview stack the Tauri v2
//! desktop client will ship reach a marketplace login page without hitting a
//! bot challenge?
//!
//! It navigates, waits, and reads. It submits nothing, fills nothing, and
//! carries no credential.

use std::collections::BTreeMap;
use std::path::{Path, PathBuf};
use std::time::{Duration, SystemTime, UNIX_EPOCH};

use tao::event::{Event, WindowEvent};
use tao::event_loop::{ControlFlow, EventLoopBuilder};
use tao::window::WindowBuilder;
use wry::WebViewBuilder;

const PROBE_JS: &str = include_str!("probe.js");
const US: char = '\u{1f}';
const IPC_GRACE: Duration = Duration::from_secs(20);

const USAGE: &str = "\
login-probe <url> [--wait-secs N] [--out report.json]

Opens <url> in the OS webview, waits N seconds (default 15) for redirects and
challenges to settle, then reads the page and writes a JSON report plus the
full page HTML beside it. Never submits a form and never sends a credential.
Exit status is 0 whatever the verdict; the report is the result.
";

#[derive(Debug)]
struct Config {
    url: String,
    wait_secs: u64,
    report_path: PathBuf,
    html_path: PathBuf,
}

#[derive(Debug, Default)]
struct Page {
    final_url: String,
    title: String,
    user_agent: String,
    markers: Vec<String>,
    password_form_count: u32,
    password_input_count: u32,
    body_text_head: String,
}

enum UserEvent {
    Probe,
    Message(String),
    Deadline,
}

fn main() {
    if let Err(e) = run() {
        eprintln!("login-probe: {e}");
        std::process::exit(2);
    }
}

fn run() -> Result<(), String> {
    let cfg = match parse_args(std::env::args().skip(1))? {
        Some(cfg) => cfg,
        None => {
            print!("{USAGE}");
            return Ok(());
        }
    };

    let event_loop = EventLoopBuilder::<UserEvent>::with_user_event().build();
    let window = WindowBuilder::new()
        .with_title(format!("login-probe — {}", cfg.url))
        .with_inner_size(tao::dpi::LogicalSize::new(1280.0, 900.0))
        .build(&event_loop)
        .map_err(|e| format!("window: {e}"))?;

    let ipc_proxy = event_loop.create_proxy();
    let builder = WebViewBuilder::new()
        .with_url(cfg.url.clone())
        .with_ipc_handler(move |req| {
            let _ = ipc_proxy.send_event(UserEvent::Message(req.into_body()));
        });

    #[cfg(any(target_os = "windows", target_os = "macos"))]
    let webview = builder
        .build(&window)
        .map_err(|e| format!("webview: {e}"))?;
    #[cfg(not(any(target_os = "windows", target_os = "macos")))]
    let webview = {
        use tao::platform::unix::WindowExtUnix;
        use wry::WebViewBuilderExtUnix;
        let vbox = window
            .default_vbox()
            .ok_or_else(|| "window has no GTK vbox".to_string())?;
        builder
            .build_gtk(vbox)
            .map_err(|e| format!("webview: {e}"))?
    };

    let timer_proxy = event_loop.create_proxy();
    let wait = Duration::from_secs(cfg.wait_secs);
    std::thread::spawn(move || {
        std::thread::sleep(wait);
        let _ = timer_proxy.send_event(UserEvent::Probe);
        std::thread::sleep(IPC_GRACE);
        let _ = timer_proxy.send_event(UserEvent::Deadline);
    });

    eprintln!(
        "login-probe: {} — waiting {}s for the page to settle",
        cfg.url, cfg.wait_secs
    );

    let mut page: Option<Page> = None;
    let mut chunks: BTreeMap<usize, String> = BTreeMap::new();
    let mut expected: Option<usize> = None;

    event_loop.run(move |event, _, control_flow| {
        *control_flow = ControlFlow::Wait;
        match event {
            Event::UserEvent(UserEvent::Probe) => {
                if let Err(e) = webview.evaluate_script(PROBE_JS) {
                    eprintln!("login-probe: evaluate_script failed: {e}");
                }
            }
            Event::UserEvent(UserEvent::Message(msg)) => {
                match ingest(&msg, &mut page, &mut chunks, &mut expected) {
                    Ok(true) => {
                        settle(&cfg, "reported", page.as_ref(), &assemble(&chunks));
                        *control_flow = ControlFlow::Exit;
                    }
                    Ok(false) => {}
                    Err(e) => eprintln!("login-probe: malformed ipc message: {e}"),
                }
            }
            Event::UserEvent(UserEvent::Deadline) => {
                settle(&cfg, "timeout", page.as_ref(), &assemble(&chunks));
                *control_flow = ControlFlow::Exit;
            }
            Event::WindowEvent {
                event: WindowEvent::CloseRequested,
                ..
            } => {
                settle(&cfg, "window_closed", page.as_ref(), &assemble(&chunks));
                *control_flow = ControlFlow::Exit;
            }
            _ => {}
        }
    });
}

fn parse_args<I: Iterator<Item = String>>(mut args: I) -> Result<Option<Config>, String> {
    let mut url: Option<String> = None;
    let mut wait_secs: u64 = 15;
    let mut out: Option<PathBuf> = None;

    while let Some(arg) = args.next() {
        match arg.as_str() {
            "-h" | "--help" => return Ok(None),
            "--wait-secs" => {
                let v = args.next().ok_or("--wait-secs needs a value")?;
                wait_secs = v
                    .parse()
                    .map_err(|_| format!("--wait-secs: {v} is not a number"))?;
            }
            "--out" => out = Some(PathBuf::from(args.next().ok_or("--out needs a value")?)),
            other if other.starts_with('-') => return Err(format!("unknown flag: {other}")),
            other if url.is_none() => url = Some(other.to_string()),
            other => return Err(format!("unexpected argument: {other}")),
        }
    }

    let url = url.ok_or_else(|| format!("a url is required\n\n{USAGE}"))?;
    let report_path = out.unwrap_or_else(|| PathBuf::from("report.json"));
    let html_path = report_path.with_extension("html");
    Ok(Some(Config {
        url,
        wait_secs,
        report_path,
        html_path,
    }))
}

/// Returns `Ok(true)` once the end marker has arrived and every chunk it
/// announced is present.
fn ingest(
    msg: &str,
    page: &mut Option<Page>,
    chunks: &mut BTreeMap<usize, String>,
    expected: &mut Option<usize>,
) -> Result<bool, String> {
    let (tag, rest) = msg.split_once(US).ok_or("no record separator")?;
    match tag {
        "M" => {
            let f: Vec<&str> = rest.splitn(7, US).collect();
            if f.len() != 7 {
                return Err(format!("metadata has {} fields, expected 7", f.len()));
            }
            *page = Some(Page {
                final_url: f[0].to_string(),
                title: f[1].to_string(),
                user_agent: f[2].to_string(),
                markers: f[3]
                    .split(" | ")
                    .filter(|s| !s.is_empty())
                    .map(str::to_string)
                    .collect(),
                password_form_count: f[4].parse().unwrap_or(0),
                password_input_count: f[5].parse().unwrap_or(0),
                body_text_head: f[6].to_string(),
            });
            Ok(false)
        }
        "H" => {
            let (index, body) = rest.split_once(US).ok_or("chunk has no index")?;
            let index: usize = index.parse().map_err(|_| "chunk index is not a number")?;
            chunks.insert(index, body.to_string());
            Ok(expected.is_some_and(|total| chunks.len() >= total))
        }
        "E" => {
            let total: usize = rest.parse().map_err(|_| "end marker is not a number")?;
            *expected = Some(total);
            Ok(chunks.len() >= total)
        }
        other => Err(format!("unknown tag: {other}")),
    }
}

fn assemble(chunks: &BTreeMap<usize, String>) -> String {
    chunks.values().fold(String::new(), |mut acc, c| {
        acc.push_str(c);
        acc
    })
}

fn verdict(page: Option<&Page>) -> &'static str {
    match page {
        Some(p) if !p.markers.is_empty() => "CHALLENGE",
        Some(p) if p.password_form_count > 0 || p.password_input_count > 0 => "CLEAR",
        _ => "UNKNOWN",
    }
}

fn settle(cfg: &Config, outcome: &str, page: Option<&Page>, html: &str) {
    let v = verdict(page);
    if let Err(e) = write_report(cfg, outcome, page, html) {
        eprintln!("login-probe: could not write the report: {e}");
    }
    println!(
        "{v} {} ({outcome}) -> {}",
        page.map_or(cfg.url.as_str(), |p| if p.final_url.is_empty() {
            cfg.url.as_str()
        } else {
            p.final_url.as_str()
        }),
        cfg.report_path.display()
    );
}

fn write_report(
    cfg: &Config,
    outcome: &str,
    page: Option<&Page>,
    html: &str,
) -> std::io::Result<()> {
    if !html.is_empty() {
        write_beside(&cfg.html_path, html.as_bytes())?;
    }
    let blank = Page::default();
    let p = page.unwrap_or(&blank);
    let markers = p
        .markers
        .iter()
        .map(|m| json_str(m))
        .collect::<Vec<_>>()
        .join(", ");
    let body = format!(
        concat!(
            "{{\n",
            "  \"schema\": \"login-probe/1\",\n",
            "  \"timestamp_utc\": {},\n",
            "  \"os\": {},\n  \"arch\": {},\n  \"webview_stack\": {},\n",
            "  \"requested_url\": {},\n  \"wait_secs\": {},\n",
            "  \"verdict\": {},\n  \"outcome\": {},\n",
            "  \"final_url\": {},\n  \"title\": {},\n  \"user_agent\": {},\n",
            "  \"challenge_markers\": [{}],\n",
            "  \"password_form_count\": {},\n  \"password_input_count\": {},\n",
            "  \"body_text_head\": {},\n",
            "  \"page_html_bytes\": {},\n  \"page_html_file\": {}\n",
            "}}\n"
        ),
        json_str(&iso8601_utc(now_unix())),
        json_str(std::env::consts::OS),
        json_str(std::env::consts::ARCH),
        json_str(WEBVIEW_STACK),
        json_str(&cfg.url),
        cfg.wait_secs,
        json_str(verdict(page)),
        json_str(outcome),
        json_str(&p.final_url),
        json_str(&p.title),
        json_str(&p.user_agent),
        markers,
        p.password_form_count,
        p.password_input_count,
        json_str(&p.body_text_head),
        html.len(),
        json_str(&cfg.html_path.display().to_string()),
    );
    write_beside(&cfg.report_path, body.as_bytes())
}

fn write_beside(path: &Path, bytes: &[u8]) -> std::io::Result<()> {
    if let Some(dir) = path.parent().filter(|d| !d.as_os_str().is_empty()) {
        std::fs::create_dir_all(dir)?;
    }
    std::fs::write(path, bytes)
}

#[cfg(target_os = "windows")]
const WEBVIEW_STACK: &str = "WebView2 (wry 0.55.1, tao 0.35.3)";
#[cfg(target_os = "macos")]
const WEBVIEW_STACK: &str = "WKWebView (wry 0.55.1, tao 0.35.3)";
#[cfg(not(any(target_os = "windows", target_os = "macos")))]
const WEBVIEW_STACK: &str = "WebKitGTK 4.1 (wry 0.55.1, tao 0.35.3)";

fn json_str(s: &str) -> String {
    let mut out = String::with_capacity(s.len() + 2);
    out.push('"');
    for c in s.chars() {
        match c {
            '"' => out.push_str("\\\""),
            '\\' => out.push_str("\\\\"),
            '\n' => out.push_str("\\n"),
            '\r' => out.push_str("\\r"),
            '\t' => out.push_str("\\t"),
            c if (c as u32) < 0x20 => out.push_str(&format!("\\u{:04x}", c as u32)),
            c => out.push(c),
        }
    }
    out.push('"');
    out
}

#[expect(
    clippy::disallowed_methods,
    reason = "this is the driver that reads the clock: the probe stamps one report at the moment it writes it"
)]
fn now_unix() -> i64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map_or(0, |d| d.as_secs() as i64)
}

fn iso8601_utc(secs: i64) -> String {
    let rem = secs.rem_euclid(86_400);
    let (y, m, d) = civil_from_days(secs.div_euclid(86_400));
    format!(
        "{y:04}-{m:02}-{d:02}T{:02}:{:02}:{:02}Z",
        rem / 3600,
        (rem % 3600) / 60,
        rem % 60
    )
}

/// Howard Hinnant, "chrono-Compatible Low-Level Date Algorithms",
/// <https://howardhinnant.github.io/date_algorithms.html#civil_from_days>.
/// Days are counted from 1970-01-01; the shift moves the era boundary to
/// 0000-03-01 so that the leap day lands at the end of the cycle.
fn civil_from_days(z: i64) -> (i64, i64, i64) {
    let z = z + 719_468;
    let era = z.div_euclid(146_097);
    let doe = z.rem_euclid(146_097);
    let yoe = (doe - doe / 1460 + doe / 36_524 - doe / 146_096) / 365;
    let doy = doe - (365 * yoe + yoe / 4 - yoe / 100);
    let mp = (5 * doy + 2) / 153;
    let d = doy - (153 * mp + 2) / 5 + 1;
    let m = if mp < 10 { mp + 3 } else { mp - 9 };
    let y = yoe + era * 400;
    (if m <= 2 { y + 1 } else { y }, m, d)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn iso8601_matches_known_instants() {
        assert_eq!(iso8601_utc(0), "1970-01-01T00:00:00Z");
        assert_eq!(iso8601_utc(1_788_393_600), "2026-09-03T00:00:00Z");
        assert_eq!(iso8601_utc(951_868_800), "2000-03-01T00:00:00Z");
        assert_eq!(iso8601_utc(1_709_164_800), "2024-02-29T00:00:00Z");
    }

    #[test]
    fn json_escapes_quotes_controls_and_backslashes() {
        assert_eq!(json_str("a\"b\\c\nd\u{1}"), "\"a\\\"b\\\\c\\nd\\u0001\"");
    }

    #[test]
    fn metadata_then_chunks_completes_only_after_the_end_marker() {
        let (mut page, mut chunks, mut expected) = (None, BTreeMap::new(), None);
        let m = format!("M{US}https://x/{US}Title{US}UA{US}{US}1{US}1{US}body");
        assert_eq!(ingest(&m, &mut page, &mut chunks, &mut expected), Ok(false));
        assert_eq!(page.as_ref().map(|p| p.password_form_count), Some(1));
        assert!(page.as_ref().is_some_and(|p| p.markers.is_empty()));
        assert_eq!(
            ingest(
                &format!("H{US}1{US}<b>"),
                &mut page,
                &mut chunks,
                &mut expected
            ),
            Ok(false)
        );
        assert_eq!(
            ingest(
                &format!("H{US}0{US}<a>"),
                &mut page,
                &mut chunks,
                &mut expected
            ),
            Ok(false)
        );
        assert_eq!(
            ingest(&format!("E{US}2"), &mut page, &mut chunks, &mut expected),
            Ok(true)
        );
        assert_eq!(assemble(&chunks), "<a><b>");
    }

    #[test]
    fn a_chunk_keeps_separators_that_occur_inside_the_page() {
        let (mut page, mut chunks, mut expected) = (None, BTreeMap::new(), None);
        let _ = ingest(
            &format!("H{US}0{US}a{US}b"),
            &mut page,
            &mut chunks,
            &mut expected,
        );
        assert_eq!(assemble(&chunks), format!("a{US}b"));
    }

    #[test]
    fn verdict_ranks_a_challenge_above_a_visible_login_form() {
        let challenged = Page {
            markers: vec!["cloudflare-interstitial (html: cf-chl)".into()],
            password_form_count: 1,
            ..Page::default()
        };
        assert_eq!(verdict(Some(&challenged)), "CHALLENGE");
        assert_eq!(
            verdict(Some(&Page {
                password_form_count: 1,
                ..Page::default()
            })),
            "CLEAR"
        );
        assert_eq!(verdict(Some(&Page::default())), "UNKNOWN");
        assert_eq!(verdict(None), "UNKNOWN");
    }

    #[test]
    fn parse_args_defaults_and_overrides() {
        let base = parse_args(["https://example.com".to_string()].into_iter())
            .unwrap()
            .unwrap();
        assert_eq!(
            (base.wait_secs, base.report_path.as_path()),
            (15, Path::new("report.json"))
        );
        assert_eq!(base.html_path.as_path(), Path::new("report.html"));

        let tuned = parse_args(
            [
                "https://x/".into(),
                "--wait-secs".into(),
                "30".into(),
                "--out".into(),
                "a/b.json".into(),
            ]
            .into_iter(),
        )
        .unwrap()
        .unwrap();
        assert_eq!(tuned.wait_secs, 30);
        assert_eq!(tuned.html_path.as_path(), Path::new("a/b.html"));

        assert!(parse_args(["--nope".to_string()].into_iter()).is_err());
        assert!(parse_args(std::iter::empty()).is_err());
    }
}
