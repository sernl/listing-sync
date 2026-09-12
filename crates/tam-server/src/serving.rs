//! What answers a request the API router did not match: the landing page, the
//! downloads directory, the console, or nothing.
//!
//! The four tiers are ordered — API routes, then the landing build, then
//! `/downloads`, then the console — and [`route`] is where that order is
//! written down. It is pure: a request path and a way to ask whether the
//! landing build holds a file, in; one of four answers, out. Everything that
//! reads a directory or writes a header is below it.
//!
//! Four namespaces are reserved ahead of the landing probe rather than left to
//! it. `app` and `_app` are the console's home and its client bundle,
//! `resources` is the catalogue board, and `downloads` is the tier below; a
//! landing build that emitted a page at any of them would otherwise take it,
//! and the console's home, a seller's catalogue or the whole download surface
//! would be unreachable in every browser with nothing logged.
//!
//! The landing build is read into memory once at start-up, the way the console
//! shell already is, so a request never joins a client-supplied string to a
//! filesystem path. Traversal is refused twice over: [`route`] rejects a `..`
//! or `.` segment after percent-decoding, and the only names that can be served
//! at all are the ones this module put in its own map while walking the
//! directory. The downloads tier reaches the filesystem per request, because
//! its contents change under the server between refreshes, and it holds the
//! same construction a different way: see [`crate::downloads`].

use std::collections::{BTreeMap, BTreeSet};
use std::path::Path;

use axum::body::Bytes;
use axum::response::IntoResponse as _;

/// The most of a landing directory that is read into memory.
///
/// The built site is under 200 KB, so this is two orders of magnitude of
/// headroom and still refuses to swallow a mis-pointed path before serving
/// anything from it. Named for the same reason the entitlement-key cap is:
/// without it the refusal would describe the path rather than its size.
const LANDING_BYTES_MAX: u64 = 64 * 1024 * 1024;

/// The console's home, the console's client bundle and the catalogue board,
/// reserved as first segments so no landing build can take any of them.
///
/// `console-serving.md` used to state this as a rule the landing build had to
/// keep; a rule nothing enforces is a sentence. The cost of the rule breaking
/// is the console's home page answering with a marketing page under the
/// marketing policy, or every SvelteKit chunk 404ing into the shell.
///
/// `resources` is here because the word is one a marketing site reaches for --
/// a teaching-resources site with a `resources.astro` is an ordinary thing to
/// build -- and the catalogue board answers there. Without the reservation the
/// landing probe runs first, so the day that page exists a signed-in seller
/// asking for their catalogue gets it silently.
const CONSOLE_NAMESPACES: [&str; 3] = ["app", "_app", "resources"];

/// The first segment the downloads tier answers under.
const DOWNLOADS_NAMESPACE: &str = "downloads";

/// Which tier owns a request path.
///
/// The API is an answer here even though the router matches its own routes
/// before this decision is reached, and that is the point: it makes the
/// precedence a statement this module can be tested against rather than a
/// property of how the router happened to be composed. A landing build that
/// emitted `v1/status` must not answer where the API declined to.
#[derive(Debug, PartialEq, Eq)]
pub(crate) enum Answer {
    Api,
    /// The path names this file, relative to the landing directory.
    Landing(String),
    /// The path names this file in the downloads directory. Always one
    /// segment: that directory is flat by construction, so a name carrying a
    /// separator is not one of its files and never becomes a path.
    Downloads(String),
    Console,
}

/// Which tier answers `path`, given a way to ask whether the landing build
/// holds a file at some relative name and whether this deployment serves
/// downloads at all.
///
/// The path is percent-decoded first, because that is what the client encoded
/// and what the console's own `ServeDir` does on the other arm; a page named
/// `für-lehrer` is requested as `/f%C3%BCr-lehrer/` and has to resolve to the
/// same file. Decoding happens before the `..` guard, never after, so an
/// encoded traversal is refused by the same line a plain one is.
///
/// A directory page resolves through its `index.html`, which is what makes
/// `/privacy`, `/pricing` and `/terms` work without the Astro build emitting
/// extensionless files. Everything no tier claims is the console's, so the
/// console keeps answering exactly what it answered before a landing directory
/// existed.
pub(crate) fn route(
    path: &str,
    landing_has: impl Fn(&str) -> bool,
    serves_downloads: bool,
) -> Answer {
    let Some(raw) = path.strip_prefix('/') else {
        return Answer::Console;
    };
    // A malformed escape or a non-utf-8 decode names no file any tier holds, so
    // it falls to the console rather than being guessed at.
    let Some(decoded) = percent_decoded(raw) else {
        return Answer::Console;
    };
    let relative = decoded.trim_end_matches('/');
    let mut segments = relative.split('/');
    let first = segments.next().unwrap_or_default();
    if first.parse::<tam_api::APIVersion>().is_ok() {
        return Answer::Api;
    }
    // The map holds only names this module walked out of the directory, so a
    // traversal cannot match one anyway. Refused here as well so the guarantee
    // belongs to the decision rather than to how its caller built the map, and
    // so that it survives a later tier that does reach the filesystem.
    if relative
        .split('/')
        .any(|segment| segment == ".." || segment == ".")
    {
        return Answer::Console;
    }
    if CONSOLE_NAMESPACES.contains(&first) {
        return Answer::Console;
    }
    // Reserved whether or not this deployment serves downloads, so the
    // guarantee is the same one `app` and `_app` get. Conditionally reserving
    // it would let a landing build take `/downloads/` on a deployment with the
    // tier off, and enabling the tier later would then shadow a live page with
    // no diagnostic anywhere.
    if first == DOWNLOADS_NAMESPACE {
        if !serves_downloads {
            return Answer::Console;
        }
        return match (segments.next(), segments.next()) {
            // A leading dot is refused rather than looked up. The refresh
            // stages into `.staging/` inside this directory so its rename is
            // atomic, and an older deployment's interrupted run can have left
            // a `.incoming-…` beside the published files; serving either would
            // hand a client a partial installer under a name the manifest makes
            // guessable.
            (Some(file), None) if !file.is_empty() && !file.starts_with('.') => {
                Answer::Downloads(file.to_owned())
            }
            // `/downloads` and `/downloads/` name the directory itself, and a
            // deeper path names nothing: that directory is flat and is never
            // listed.
            _ => Answer::Console,
        };
    }
    let candidate = if relative.is_empty() {
        "index.html"
    } else {
        relative
    };
    if landing_has(candidate) {
        return Answer::Landing(candidate.to_owned());
    }
    let indexed = format!("{candidate}/index.html");
    if landing_has(&indexed) {
        return Answer::Landing(indexed);
    }
    Answer::Console
}

/// `path` with its `%XX` escapes resolved, or `None` where an escape is
/// malformed or the result is not utf-8.
///
/// Hand-rolled rather than pulled from a crate, because the whole of it is
/// this: the workspace's dependency set is a founder decision and this is
/// twenty lines. `+` is left alone — it means a space in a query string, never
/// in a path.
fn percent_decoded(path: &str) -> Option<String> {
    if !path.contains('%') {
        return Some(path.to_owned());
    }
    let raw = path.as_bytes();
    let mut out = Vec::with_capacity(raw.len());
    let mut at = 0usize;
    while let Some(&byte) = raw.get(at) {
        if byte == b'%' {
            let high = raw.get(at + 1).copied().and_then(hex_value)?;
            let low = raw.get(at + 2).copied().and_then(hex_value)?;
            out.push(high * 16 + low);
            at += 3;
        } else {
            out.push(byte);
            at += 1;
        }
    }
    String::from_utf8(out).ok()
}

fn hex_value(digit: u8) -> Option<u8> {
    match digit {
        b'0'..=b'9' => Some(digit - b'0'),
        b'a'..=b'f' => Some(digit - b'a' + 10),
        b'A'..=b'F' => Some(digit - b'A' + 10),
        _ => None,
    }
}

/// GET and HEAD are the only methods a static tier answers.
///
/// The console's `ServeDir` refuses everything else with 405 and does not call
/// its fallback, so answering a `POST /` with the landing page would have been
/// a change in behaviour rather than a new surface. The body is built for HEAD
/// too and hyper drops it on the way out, which is what keeps `content-length`
/// honest.
pub(crate) fn method_serves_static(method: &axum::http::Method) -> bool {
    method == axum::http::Method::GET || method == axum::http::Method::HEAD
}

/// The refusal a static tier gives every other method.
pub(crate) fn method_not_allowed() -> axum::response::Response {
    (
        axum::http::StatusCode::METHOD_NOT_ALLOWED,
        [(axum::http::header::ALLOW, "GET, HEAD")],
    )
        .into_response()
}

/// Whether a validator in an `if-none-match` header matches `etag`.
///
/// The header is a comma-separated list; `*` matches anything the server holds.
///
/// The comparison is over the opaque tags with the `W/` prefix stripped from
/// *both* sides, which is the weak comparison RFC 9110 §8.8.3.2 specifies for
/// this header — and stripping only the client's side is a bug that hides:
/// the landing tier's validators are strong, so it looks right there, while the
/// downloads tier's are weak by construction and never match, and every
/// conditional request for an installer re-sends tens of megabytes.
pub(crate) fn none_match(header: Option<&axum::http::HeaderValue>, etag: &str) -> bool {
    let Some(value) = header.and_then(|value| value.to_str().ok()) else {
        return false;
    };
    let held = etag.trim_start_matches("W/");
    value.split(',').any(|candidate| {
        let candidate = candidate.trim();
        candidate == "*" || candidate.trim_start_matches("W/") == held
    })
}

/// The 304 a matching validator earns: the validator itself and the freshness
/// rule, and no body.
pub(crate) fn not_modified(etag: &str, cache_control: &str) -> axum::response::Response {
    (
        axum::http::StatusCode::NOT_MODIFIED,
        [
            (axum::http::header::ETAG, etag),
            (axum::http::header::CACHE_CONTROL, cache_control),
        ],
    )
        .into_response()
}

/// The strong validator for a body: the first half of its SHA-256, quoted.
///
/// Half the digest is 64 bits of collision resistance against an accident,
/// which is what a cache validator needs; it is not a security boundary and
/// nothing downstream treats it as one.
pub(crate) fn etag_of(bytes: &[u8]) -> String {
    use sha2::Digest as _;
    const HEX: [char; 16] = [
        '0', '1', '2', '3', '4', '5', '6', '7', '8', '9', 'a', 'b', 'c', 'd', 'e', 'f',
    ];
    let digest = sha2::Sha256::digest(bytes);
    let mut etag = String::with_capacity(19);
    etag.push('"');
    for byte in digest.iter().take(8) {
        etag.push(HEX[usize::from(byte >> 4)]);
        etag.push(HEX[usize::from(byte & 0x0f)]);
    }
    etag.push('"');
    etag
}

/// One file of the landing build, with everything its response needs decided
/// once at start-up rather than per request.
struct Asset {
    bytes: Bytes,
    content_type: &'static str,
    cache_control: &'static str,
    etag: String,
}

/// The landing build in memory: every file it holds, and the one policy they
/// are all served under.
pub(crate) struct Landing {
    files: BTreeMap<String, Asset>,
    policy: String,
}

impl Landing {
    /// Read the directory once, and compute the policy from what was read.
    pub(crate) fn load(dir: &Path) -> Result<Self, Box<dyn std::error::Error>> {
        let bytes = walk(dir)?;
        if !bytes.contains_key("index.html") {
            return Err(format!(
                "{} holds no index.html, so it is not a built landing site",
                dir.display()
            )
            .into());
        }
        let policy = landing_policy(&inline_script_hashes_across(&bytes)?);
        let files = bytes
            .into_iter()
            .map(|(name, bytes)| {
                let asset = Asset {
                    etag: etag_of(&bytes),
                    content_type: content_type(&name),
                    cache_control: cache_control(&name),
                    bytes,
                };
                (name, asset)
            })
            .collect();
        Ok(Self { files, policy })
    }

    pub(crate) fn has(&self, file: &str) -> bool {
        self.files.contains_key(file)
    }

    /// The bytes, their type, their freshness rule, their validator and the
    /// landing policy — or a 304 where the client already holds them.
    ///
    /// Total rather than infallible-by-contract: [`route`] only ever names a
    /// file this map holds, and a miss here answering 404 is cheaper than a
    /// signature that made the caller prove it.
    pub(crate) fn respond(
        &self,
        file: &str,
        if_none_match: Option<&axum::http::HeaderValue>,
    ) -> axum::response::Response {
        let Some(asset) = self.files.get(file) else {
            return axum::http::StatusCode::NOT_FOUND.into_response();
        };
        if none_match(if_none_match, &asset.etag) {
            return not_modified(&asset.etag, asset.cache_control);
        }
        let mut response = (
            [
                (axum::http::header::CONTENT_TYPE, asset.content_type),
                (axum::http::header::CACHE_CONTROL, asset.cache_control),
                (axum::http::header::ETAG, asset.etag.as_str()),
                (
                    axum::http::header::CONTENT_SECURITY_POLICY,
                    self.policy.as_str(),
                ),
            ],
            // A refcount bump rather than a copy of the file: these are the
            // public root's fonts, requested by every first-time visitor.
            asset.bytes.clone(),
        )
            .into_response();
        // A page carries a nonce and an asset does not: the edge injects into
        // HTML alone, and a header built per response is one allocation this
        // origin has no reason to spend on a font.
        if asset.content_type.starts_with("text/html") {
            if let Ok(value) =
                axum::http::HeaderValue::from_str(&with_nonce(&self.policy, &fresh_nonce()))
            {
                response
                    .headers_mut()
                    .insert(axum::http::header::CONTENT_SECURITY_POLICY, value);
            }
        }
        response
    }
}

/// How long a landing file may be reused without asking.
///
/// Astro content-addresses everything it emits under `_astro/`, so those names
/// change whenever their bytes do and a year is safe. The pages must not be
/// held at all — a cutover has to be visible on the next request — and
/// everything else, the fonts and the icon among them, keeps a stable name with
/// changeable bytes, so it revalidates against the entity tag after an hour.
fn cache_control(file: &str) -> &'static str {
    if file.starts_with("_astro/") {
        "public, max-age=31536000, immutable"
    } else if content_type(file).starts_with("text/html") {
        "no-cache"
    } else {
        "public, max-age=3600"
    }
}

/// Every regular file under `root`, keyed by its path relative to it with `/`
/// separators, which is the shape a request path is compared against.
///
/// A symlink is neither followed nor served: following one is the one way a
/// directory's contents can name bytes outside it, and nothing a static-site
/// build emits needs it. A name that is not utf-8 is skipped, because no
/// request path can name it. Both skips are reported, because a builder that
/// used `symlinkJoin` would otherwise start cleanly, pass the `index.html`
/// check, and serve a page whose stylesheet and fonts 404 into the console
/// shell with nothing said anywhere.
fn walk(root: &Path) -> Result<BTreeMap<String, Bytes>, Box<dyn std::error::Error>> {
    use std::io::Read as _;

    let mut files = BTreeMap::new();
    let mut total: u64 = 0;
    let mut pending = vec![(root.to_path_buf(), String::new())];
    while let Some((directory, prefix)) = pending.pop() {
        for entry in std::fs::read_dir(&directory)? {
            let entry = entry?;
            let name = entry.file_name();
            let Some(name) = name.to_str() else {
                eprintln!(
                    "tam-server: {} has a name that is not utf-8 and is not served",
                    entry.path().display()
                );
                continue;
            };
            let relative = if prefix.is_empty() {
                name.to_owned()
            } else {
                format!("{prefix}/{name}")
            };
            let kind = entry.file_type()?;
            if kind.is_dir() {
                pending.push((entry.path(), relative));
            } else if kind.is_file() {
                total = total.saturating_add(entry.metadata()?.len());
                if total > LANDING_BYTES_MAX {
                    return Err(format!(
                        "{} holds more than {LANDING_BYTES_MAX} bytes, so it is not a built \
                         landing site; check what {} names",
                        root.display(),
                        crate::LANDING_FLAG
                    )
                    .into());
                }
                let mut bytes = Vec::new();
                std::fs::File::open(entry.path())?.read_to_end(&mut bytes)?;
                files.insert(relative, Bytes::from(bytes));
            } else {
                eprintln!(
                    "tam-server: {} is neither a file nor a directory and is not served; a \
                     symlink is not followed out of the landing directory",
                    entry.path().display()
                );
            }
        }
    }
    Ok(files)
}

/// Every inline-script hash the landing build needs, across every page in it.
///
/// The console computes its hashes over one shell; a static site is many pages,
/// and any of them can carry an inline block. Deduplicated and ordered, because
/// several pages built from one layout carry the same block and a policy that
/// repeated it would be longer without admitting anything more.
fn inline_script_hashes_across(
    files: &BTreeMap<String, Bytes>,
) -> Result<Vec<String>, Box<dyn std::error::Error>> {
    let mut hashes = BTreeSet::new();
    for (name, bytes) in files {
        if !Path::new(name)
            .extension()
            .is_some_and(|extension| extension.eq_ignore_ascii_case("html"))
        {
            continue;
        }
        let text = core::str::from_utf8(bytes).map_err(|_| {
            format!("{name} is not utf-8, so the hashes its policy needs cannot be computed")
        })?;
        hashes.extend(inline_script_hashes(text));
    }
    Ok(hashes.into_iter().collect())
}

/// A script nonce for the one inline script this origin does not write.
///
/// The edge in front of this origin injects one: Cloudflare's JavaScript
/// Detections, an inline block that runs on every HTML page a browser loads
/// and issues the `cf_clearance` cookie its bot scoring reads. The block is
/// different on every response, so no hash can admit it, and a policy that
/// admits inline scripts by hash alone refuses it — which is what every page
/// of this origin did, silently, in every browser's console. A browser that
/// never runs it never earns the cookie, and a client the scoring already
/// distrusts, which is what an Android WebView or WebView2 is to it, is then
/// challenged on its next fetch and shown a page the console reads as a
/// failure. The vendor documents the way out: a policy carrying a nonce, which
/// its edge parses out of the header and stamps on the block it injects.
///
/// Fresh per response, sixteen random bytes: a nonce reused across responses
/// is a hash with extra steps, and one a page could predict admits any script
/// an attacker can get into the page. The hashes stay beside it, because the
/// shell's own boot block is ours and is admitted by what it is rather than by
/// what the header says this once.
pub(crate) fn fresh_nonce() -> String {
    use base64::Engine as _;
    base64::engine::general_purpose::STANDARD.encode(uuid::Uuid::new_v4().as_bytes())
}

/// `policy` with `'nonce-{nonce}'` on its script directive.
///
/// Inserted after the directive's name rather than appended to the policy,
/// because a source expression belongs to one directive and `script-src` is
/// the one the injected block is judged by.
pub(crate) fn with_nonce(policy: &str, nonce: &str) -> String {
    match policy.split_once("script-src") {
        Some((before, after)) => {
            let mut out = String::with_capacity(policy.len() + nonce.len() + 20);
            out.push_str(before);
            out.push_str("script-src 'nonce-");
            out.push_str(nonce);
            out.push('\'');
            out.push_str(after);
            out
        }
        None => policy.to_owned(),
    }
}

/// The landing page's content-security policy, which is not the console's.
///
/// It is the narrower of the two because the page is: no WebAssembly, no
/// Turnstile, no Paddle, no frame of its own, and `frame-ancestors 'none'` on
/// top, so the marketing surface cannot be framed by anything. `script-src`
/// carries no `'unsafe-inline'` and admits an inline block only by its hash,
/// computed from the pages actually being served.
///
/// It admits no third-party host at all, and `style-src` carries no
/// `'unsafe-inline'` either. The first version of this carried the console's
/// two Google font hosts and its inline-style grant; the built site loads
/// neither host and emits neither a `<style>` element nor a `style` attribute,
/// because `apps/landing/src/styles/site.css` self-hosts both faces from
/// `/fonts/*.woff2` and Astro bundles component styles into `_astro/`. A
/// dead grant on the public origin is a grant nobody is checking, so a page
/// that ever needs one has to come back here and say why.
fn landing_policy(hashes: &[String]) -> String {
    let mut script = String::from("script-src 'self'");
    for hash in hashes {
        script.push_str(" '");
        script.push_str(hash);
        script.push('\'');
    }
    format!(
        "default-src 'self'; {script}; \
         style-src 'self'; \
         font-src 'self'; \
         img-src 'self' data:; \
         connect-src 'self'; \
         frame-ancestors 'none'"
    )
}

/// The `sha256-…` token for every inline `<script>` block in a page.
///
/// The hash is over the element's exact text content, which is what the CSP
/// specification says a hash source matches, so a byte of whitespace changed by
/// a later build changes the token — and that is the point: the policy is
/// computed from the page actually being served rather than pinned to a page
/// somebody saw once.
///
/// A script element carrying a `src` is not inline and contributes no hash.
pub(crate) fn inline_script_hashes(page: &str) -> Vec<String> {
    use base64::Engine as _;
    use sha2::Digest as _;

    // Over bytes rather than string slices: tag syntax is ASCII, the hash is
    // over bytes anyway, and slicing a `str` by a byte index is a panic waiting
    // for the first non-ASCII character in a page title.
    let bytes = page.as_bytes();
    let mut hashes = Vec::new();
    let mut at = 0usize;
    while let Some(open) = find_from(bytes, at, b"<script") {
        let Some(gt) = find_from(bytes, open, b">") else {
            break;
        };
        let attributes = &bytes[open..gt];
        let body_start = gt + 1;
        let Some(close) = find_from(bytes, body_start, b"</script>") else {
            break;
        };
        let body = &bytes[body_start..close];
        if find_from(attributes, 0, b" src=").is_none() && !body.iter().all(u8::is_ascii_whitespace)
        {
            hashes.push(format!(
                "sha256-{}",
                base64::engine::general_purpose::STANDARD.encode(sha2::Sha256::digest(body))
            ));
        }
        at = close + b"</script>".len();
    }
    hashes
}

/// The first occurrence of `needle` at or after `from`.
fn find_from(haystack: &[u8], from: usize, needle: &[u8]) -> Option<usize> {
    haystack
        .get(from..)?
        .windows(needle.len())
        .position(|window| window == needle)
        .map(|offset| from + offset)
}

/// The type a landing file is served as, named here because this module owns
/// the response rather than handing the file to a directory service that would
/// guess. An extension nothing in a static build emits is served as bytes.
pub(crate) fn content_type(file: &str) -> &'static str {
    let extension = Path::new(file)
        .extension()
        .and_then(std::ffi::OsStr::to_str)
        .unwrap_or_default()
        .to_ascii_lowercase();
    match extension.as_str() {
        "html" => "text/html; charset=utf-8",
        "css" => "text/css; charset=utf-8",
        "js" | "mjs" => "text/javascript; charset=utf-8",
        "json" | "map" => "application/json",
        "webmanifest" => "application/manifest+json",
        "xml" => "application/xml",
        "txt" => "text/plain; charset=utf-8",
        "md" => "text/markdown; charset=utf-8",
        "svg" => "image/svg+xml",
        "png" => "image/png",
        "jpg" | "jpeg" => "image/jpeg",
        "gif" => "image/gif",
        "webp" => "image/webp",
        "avif" => "image/avif",
        "ico" => "image/x-icon",
        "woff2" => "font/woff2",
        "woff" => "font/woff",
        "ttf" => "font/ttf",
        "pdf" => "application/pdf",
        "mp4" => "video/mp4",
        // The two the downloads tier serves. Named rather than left to the
        // octet-stream default so a browser's own download prompt says what
        // the file is.
        "exe" => "application/vnd.microsoft.portable-executable",
        "apk" => "application/vnd.android.package-archive",
        _ => "application/octet-stream",
    }
}

#[cfg(test)]
mod tests {
    use super::{route, Answer};

    /// A fixture in the shape of a landing build: enough names to reach every
    /// arm of `route` without a directory to read.
    ///
    /// It stands in for `landing_has` and claims nothing about what
    /// `apps/landing` emits. The real artefact's shape is asserted by
    /// `checks.served-artefacts` in `flake.nix`, against the store path the
    /// build produces -- including the font names, which used to appear
    /// nowhere but here.
    const BUILT: [&str; 7] = [
        "index.html",
        "pricing/index.html",
        "privacy/index.html",
        "terms/index.html",
        "favicon.svg",
        "_astro/Base.B4LvswBy.css",
        "fonts/fraunces-latin.woff2",
    ];

    fn built(file: &str) -> bool {
        BUILT.contains(&file)
    }

    fn landing(path: &str) -> Answer {
        route(path, built, false)
    }

    /// A landing build that claims every name there is.
    ///
    /// Every assertion that a path is *not* the landing's runs through this
    /// rather than through [`built`], because a hand-written fixture makes such
    /// an assertion pass by omitting the name — which is exactly how the
    /// reserved-namespace rule went unenforced while a design note said it held.
    fn greedy(path: &str) -> Answer {
        route(path, |_| true, true)
    }

    /// The origin's root is the landing page. This is the whole decision the
    /// founder took: the public page takes `/`, and the console gives it up.
    #[test]
    fn the_root_is_the_landing_page() {
        assert_eq!(
            landing("/"),
            Answer::Landing("index.html".to_owned()),
            "the public page takes the root"
        );
    }

    /// A page directory resolves through its own index, with or without the
    /// trailing slash a browser may or may not send.
    #[test]
    fn a_page_directory_resolves_through_its_index() {
        for path in ["/privacy", "/privacy/", "/terms", "/terms/"] {
            let expected = format!("{}/index.html", path.trim_matches('/'));
            assert_eq!(
                landing(path),
                Answer::Landing(expected),
                "{path} is a page of the landing build"
            );
        }
    }

    /// Hashed assets and the fonts resolve by their exact names, which is what
    /// makes the page render rather than merely load.
    #[test]
    fn an_asset_resolves_by_its_exact_name() {
        for path in [
            "/_astro/Base.B4LvswBy.css",
            "/fonts/fraunces-latin.woff2",
            "/favicon.svg",
        ] {
            assert_eq!(
                landing(path),
                Answer::Landing(path.trim_start_matches('/').to_owned()),
                "{path} is an asset of the landing build"
            );
        }
    }

    /// The console's home is `/app`, and every deep route stays where it was.
    ///
    /// This is the other half of the decision, and the half that breaks
    /// silently: a landing directory that claimed one of these would serve a
    /// marketing page to a signed-in seller instead of their console.
    #[test]
    fn the_console_keeps_its_own_paths() {
        for path in [
            "/app",
            "/app/",
            "/app/settings",
            "/resources",
            "/labels",
            "/sync",
            "/_app/immutable/entry/start.js",
        ] {
            assert_eq!(
                landing(path),
                Answer::Console,
                "{path} belongs to the console"
            );
        }
    }

    /// The console's home, its client bundle and the catalogue board are
    /// reserved, against a landing build that claims every name there is.
    ///
    /// This is the assertion the previous version of this file did not have:
    /// `the_console_keeps_its_own_paths` runs through a hand-written fixture,
    /// so it would have passed unchanged the day somebody added
    /// `apps/landing/src/pages/app.astro` and made the console's home answer
    /// with a marketing page. `resources.astro` is the likelier one to be
    /// written, because a teaching-resources site has an obvious use for the
    /// word and the catalogue board answers under it.
    #[test]
    fn the_console_namespaces_are_reserved_against_any_landing_build() {
        for path in [
            "/app",
            "/app/",
            "/app/settings",
            "/_app",
            "/_app/immutable/entry/start.js",
            "/resources",
            "/resources/",
            "/resources/new",
            "/resources/9f2c8a11-0000-4000-8000-000000000000",
        ] {
            assert_eq!(
                greedy(path),
                Answer::Console,
                "{path} is the console's whatever the landing build holds"
            );
        }
    }

    /// A traversal never reaches any tier, against a landing build that claims
    /// every name there is.
    ///
    /// Through [`greedy`] rather than the fixture, because under the fixture
    /// every one of these misses on the name alone and the test passes with the
    /// guard deleted. Here only the guard can produce `Console`, so deleting it
    /// fails this test — and so does moving the percent-decoding to after it,
    /// which is what the encoded cases are for.
    #[test]
    fn a_traversal_never_reaches_any_tier() {
        for path in [
            "/../etc/passwd",
            "/..",
            "/a/../../etc/passwd",
            "/./index.html",
            "/privacy/../../index.html",
            "/%2e%2e/index.html",
            "/downloads/../../etc/passwd",
            "/downloads/%2e%2e",
        ] {
            assert_eq!(
                greedy(path),
                Answer::Console,
                "{path} names no file of any tier"
            );
        }
    }

    /// A percent-encoded page name resolves to the page.
    ///
    /// The console's own `ServeDir` decodes, so a landing arm that did not
    /// would answer a non-ASCII page name with the console shell — 200, the
    /// wrong document, and nothing logged.
    #[test]
    fn a_percent_encoded_name_resolves_to_its_file() {
        let claims = |file: &str| file == "für-lehrer/index.html";
        assert_eq!(
            route("/f%C3%BCr-lehrer/", claims, false),
            Answer::Landing("für-lehrer/index.html".to_owned()),
            "a browser encodes a non-ASCII page name and it has to resolve"
        );
        assert_eq!(
            route("/f%C3%BCr-lehrer", claims, false),
            Answer::Landing("für-lehrer/index.html".to_owned()),
            "with or without the trailing slash"
        );
    }

    /// A malformed escape names nothing rather than being guessed at.
    #[test]
    fn a_malformed_escape_falls_to_the_console() {
        for path in ["/%", "/%zz", "/%c3", "/a%2"] {
            assert_eq!(greedy(path), Answer::Console, "{path} decodes to nothing");
        }
    }

    /// The downloads tier answers one flat name, and only where it is
    /// configured.
    #[test]
    fn the_downloads_tier_answers_one_flat_name() {
        assert_eq!(
            greedy("/downloads/downloads.json"),
            Answer::Downloads("downloads.json".to_owned()),
            "the manifest the console reads"
        );
        assert_eq!(
            greedy("/downloads/Teachouse_0.2.0_x64-setup.exe"),
            Answer::Downloads("Teachouse_0.2.0_x64-setup.exe".to_owned()),
            "an installer beside it"
        );
        for path in ["/downloads", "/downloads/", "/downloads/a/b"] {
            assert_eq!(
                greedy(path),
                Answer::Console,
                "{path} names the directory or a depth it does not have, and is never listed"
            );
        }
        assert_eq!(
            route("/downloads/downloads.json", |_| false, false),
            Answer::Console,
            "a deployment serving no downloads has no such tier"
        );
    }

    /// The staging directory and any interrupted run's leftovers are refused by
    /// name, before anything looks in the directory.
    ///
    /// The refresh stages inside the served directory so its rename is atomic.
    /// Nothing beginning with a dot is an answer, which covers `.staging`
    /// itself, a file inside it, and a `.incoming-…` left by a run that was
    /// killed between writing and renaming.
    #[test]
    fn nothing_beginning_with_a_dot_is_a_download() {
        for path in [
            "/downloads/.staging",
            "/downloads/.staging/Teachouse_0.2.0_arm64.apk",
            "/downloads/.incoming-Teachouse_0.2.0_arm64.apk",
            "/downloads/.",
        ] {
            assert_eq!(
                greedy(path),
                Answer::Console,
                "{path} is staging, not a published download"
            );
        }
    }

    /// The downloads namespace is reserved even where the tier is off.
    ///
    /// Otherwise a landing build could take `/downloads/` on a deployment
    /// serving no downloads, and enabling the tier later would shadow a live
    /// page with nothing said anywhere.
    #[test]
    fn the_downloads_namespace_is_reserved_even_when_the_tier_is_off() {
        for path in ["/downloads", "/downloads/index.html", "/downloads/anything"] {
            assert_eq!(
                route(path, |_| true, false),
                Answer::Console,
                "{path} is never the landing build's, tier or no tier"
            );
        }
    }

    /// The API namespace outranks a landing file of the same name.
    ///
    /// The router matches its own routes first, so this only bites where it
    /// declines — an unknown path under a version it serves — and there a
    /// landing build that had emitted `v1/status` would otherwise answer for
    /// the API.
    #[test]
    fn the_api_namespace_outranks_a_landing_file() {
        for path in ["/v1/status", "/v2/whoami", "/v1/nothing-here", "/v1"] {
            assert_eq!(
                greedy(path),
                Answer::Api,
                "{path} is the API's, whatever the landing build holds"
            );
        }
    }

    /// A version this build does not serve is not the API's namespace, so it
    /// falls through like any other unknown path.
    #[test]
    fn an_unserved_version_is_not_the_api_namespace() {
        assert_eq!(
            landing("/v9/status"),
            Answer::Console,
            "v9 is not a version this build serves"
        );
    }

    /// A page's inline bootstrap gets a hash, and it is the hash the browser
    /// will compute.
    ///
    /// The exact token is asserted rather than merely its shape, because the
    /// whole value of this routine is that the browser and we agree on the
    /// digest to the byte. `sha256-` over the element's text content is what
    /// the specification says a hash source matches, and this fixture is the
    /// smallest thing that has one.
    #[test]
    fn an_inline_script_gets_the_token_the_browser_will_compute() {
        let page = "<html><body><script>\nkit.start();\n</script></body></html>";
        let hashes = super::inline_script_hashes(page);
        assert_eq!(hashes.len(), 1, "one inline block, one hash");
        // sha256 of "\nkit.start();\n" in base64, computed independently rather
        // than copied out of this implementation's own output — otherwise the
        // test would assert only that the routine agrees with itself, which it
        // would also do if the digest and the encoding were wrong together.
        assert_eq!(
            hashes[0],
            "sha256-VjAiOu2+5hLy4jzOw1UoUpq6dLNOdmsOvp6FQGkcMSI="
        );
    }

    /// A script with a `src` is not inline and contributes no hash.
    ///
    /// Hashing it would produce a token matching nothing, and a policy full of
    /// tokens that match nothing is one nobody can read.
    #[test]
    fn a_sourced_script_contributes_no_hash() {
        let page = "<script src=\"/_app/start.js\"></script><script>\nkit.start();\n</script>";
        // The identity rather than the count: a mutation that hashed the
        // sourced block and dropped the inline one would keep the count at one.
        // The token is the independently-computed one from the test above.
        assert_eq!(
            super::inline_script_hashes(page),
            vec!["sha256-VjAiOu2+5hLy4jzOw1UoUpq6dLNOdmsOvp6FQGkcMSI=".to_owned()],
            "only the inline block, and it is the inline block"
        );
    }

    /// Hashes are collected across every page, deduplicated, and pages without
    /// one contribute nothing.
    #[test]
    fn hashes_are_collected_across_every_page() {
        let files = [
            ("index.html", "<script>\nboot();\n</script>"),
            ("terms/index.html", "<script>\nboot();\n</script>"),
            ("privacy/index.html", "<script>\nother();\n</script>"),
            ("_astro/base.css", "<script>not html</script>"),
        ]
        .into_iter()
        .map(|(name, text)| (name.to_owned(), super::Bytes::from_static(text.as_bytes())))
        .collect();
        let hashes = super::inline_script_hashes_across(&files).unwrap();
        assert_eq!(
            hashes.len(),
            2,
            "two distinct blocks across three pages: {hashes:?}"
        );
    }

    /// Every page's hash is in the policy those pages are served under, and the
    /// policy admits no inline script by any other means.
    #[test]
    fn the_policy_admits_each_page_by_its_hash_and_no_other_way() {
        let page = "<html><body><script>\nkit.start();\n</script></body></html>";
        let hashes = super::inline_script_hashes(page);
        let policy = super::landing_policy(&hashes);
        for hash in &hashes {
            assert!(
                policy.contains(hash.as_str()),
                "a page's own hash belongs in its policy: {policy}"
            );
        }
        assert!(
            !policy.contains("'unsafe-inline' 'sha256")
                && !policy.contains("script-src 'self' 'unsafe-inline'"),
            "an inline block is admitted by hash, never by 'unsafe-inline': {policy}"
        );
    }

    /// A page's policy carries a nonce on its script directive, an asset's
    /// does not, and two pages never carry the same one.
    ///
    /// The nonce is what lets the edge's injected detection script run; a
    /// reused one would admit any script that learned it.
    #[test]
    fn a_page_carries_a_fresh_nonce_and_an_asset_carries_none() {
        let policy = super::landing_policy(&["'sha256-abc'".to_owned()]);
        let one = super::with_nonce(&policy, &super::fresh_nonce());
        let two = super::with_nonce(&policy, &super::fresh_nonce());
        let script = |p: &str| {
            p.split(';')
                .find(|part| part.trim_start().starts_with("script-src"))
                .expect("script-src is in the policy")
                .to_owned()
        };
        assert!(
            script(&one).contains("'nonce-") && script(&one).contains("'sha256-abc'"),
            "the nonce sits on script-src beside the hashes: {one}"
        );
        assert_ne!(one, two, "a nonce is fresh per response");
        assert!(
            axum::http::HeaderValue::from_str(&one).is_ok(),
            "the nonced policy is a sendable header: {one}"
        );
        assert_eq!(
            super::with_nonce("default-src 'self'", "x"),
            "default-src 'self'",
            "a policy with no script directive is left alone"
        );
    }

    /// The landing policy is the narrow one, and it refuses to be framed.
    #[test]
    fn the_landing_policy_is_narrower_than_the_consoles() {
        let policy = super::landing_policy(&[]);
        assert!(
            policy.contains("frame-ancestors 'none'"),
            "the marketing page is framed by nothing: {policy}"
        );
        // Enumerated by hand rather than by a list of things the console
        // happens to name, because the previous version of this list did not
        // name the two font hosts and they sat in the policy unused for that
        // reason alone.
        assert!(
            !policy.contains("https://"),
            "the landing policy admits no third-party host on any directive: {policy}"
        );
        assert!(
            !policy.contains("'unsafe-inline'"),
            "neither a script nor a style is admitted for being inline: {policy}"
        );
        for absent in ["'wasm-unsafe-eval'", "blob:", "'unsafe-eval'"] {
            assert!(
                !policy.contains(absent),
                "{absent} is the console's, not the landing page's: {policy}"
            );
        }
        assert!(
            axum::http::HeaderValue::from_str(&policy).is_ok(),
            "the computed policy has to survive being put in a header: {policy}"
        );
    }

    /// The freshness rule matches how each kind of name changes.
    #[test]
    fn each_kind_of_landing_file_carries_its_own_freshness_rule() {
        for (file, expected) in [
            ("index.html", "no-cache"),
            ("privacy/index.html", "no-cache"),
            (
                "_astro/Base.B4LvswBy.css",
                "public, max-age=31536000, immutable",
            ),
            ("fonts/fraunces-latin.woff2", "public, max-age=3600"),
            ("favicon.svg", "public, max-age=3600"),
        ] {
            assert_eq!(
                super::cache_control(file),
                expected,
                "{file} is reused under {expected}"
            );
        }
    }

    /// A validator matches the entity tag it was minted from, and nothing else.
    #[test]
    fn a_validator_matches_its_own_entity_tag() {
        let etag = super::etag_of(b"the bytes of a page");
        let header = |raw: &str| axum::http::HeaderValue::from_str(raw).expect("a header value");
        assert!(
            super::none_match(Some(&header(&etag)), &etag),
            "the client holds this exact entity"
        );
        assert!(
            super::none_match(Some(&header(&format!("W/{etag}"))), &etag),
            "a weak validator over the same entity still matches"
        );
        assert!(
            super::none_match(Some(&header(&format!("\"other\", {etag}"))), &etag),
            "one of a list matching is a match"
        );
        assert!(
            super::none_match(Some(&header("*")), &etag),
            "a star matches whatever the server holds"
        );
        assert!(
            !super::none_match(Some(&header("\"other\"")), &etag),
            "a different entity is not a match"
        );
        assert!(
            !super::none_match(None, &etag),
            "no validator is not a match"
        );
        assert_ne!(
            super::etag_of(b"one body"),
            super::etag_of(b"another body"),
            "two bodies do not share a validator"
        );

        // The weak side, which the downloads tier is made of. Comparing a weak
        // validator against itself has to succeed: stripping the prefix from
        // only the client's copy leaves every conditional request for an
        // installer re-sending the whole file, and the landing tier's strong
        // validators hide it.
        let weak = "W/\"3fa2-6a9b6f72\"";
        assert!(
            super::none_match(Some(&header(weak)), weak),
            "a weak validator matches itself"
        );
        assert!(
            super::none_match(Some(&header("\"3fa2-6a9b6f72\"")), weak),
            "and matches its own opaque tag sent strong"
        );
        assert!(
            !super::none_match(Some(&header("W/\"3fa2-00000000\"")), weak),
            "a different weak validator is not a match"
        );
    }

    /// Only GET and HEAD reach a static tier, and the refusal says which.
    #[test]
    fn only_get_and_head_reach_a_static_tier() {
        use axum::http::Method;
        for method in [Method::GET, Method::HEAD] {
            assert!(
                super::method_serves_static(&method),
                "{method} reads a static file"
            );
        }
        for method in [Method::POST, Method::PUT, Method::DELETE, Method::OPTIONS] {
            assert!(
                !super::method_serves_static(&method),
                "{method} does not, and the console's ServeDir refused it before this tier existed"
            );
        }
        let refusal = super::method_not_allowed();
        assert_eq!(
            refusal.status(),
            axum::http::StatusCode::METHOD_NOT_ALLOWED,
            "the refusal is a 405"
        );
        assert_eq!(
            refusal
                .headers()
                .get(axum::http::header::ALLOW)
                .map(|value| value.to_str().unwrap_or_default()),
            Some("GET, HEAD"),
            "and it says what would have been allowed"
        );
    }

    /// The types a page needs to render are named, and an unknown one is bytes
    /// rather than a guess.
    #[test]
    fn each_served_type_is_named() {
        for (file, expected) in [
            ("index.html", "text/html; charset=utf-8"),
            ("_astro/Base.B4LvswBy.css", "text/css; charset=utf-8"),
            ("fonts/fraunces-latin.woff2", "font/woff2"),
            ("favicon.svg", "image/svg+xml"),
            ("fonts/OFL-Fraunces.txt", "text/plain; charset=utf-8"),
            ("robots", "application/octet-stream"),
        ] {
            assert_eq!(
                super::content_type(file),
                expected,
                "{file} is served as {expected}"
            );
        }
    }
}
