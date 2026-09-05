//! The desktop installers, served read-only from a directory this process
//! never writes.
//!
//! The server fetches nothing. Its unit denies IP egress outright
//! (`nix/module.nix`, `loopbackOnly`) and that is the point of the split: a
//! separate oneshot with its own account and its own egress allowance refreshes
//! the directory on a timer, and this tier only reads what it finds there.
//!
//! Which means the contents change under a running process, so nothing here is
//! held in memory the way the landing build is. Each request reads the file
//! again, and streams it: an Android package is tens of megabytes and there is
//! no reason for one to sit in the heap of an HTTP process.
//!
//! Traversal is impossible by the landing tier's construction, applied per
//! request instead of once. [`crate::serving::route`] only ever yields a single
//! path segment and never one beginning with a dot, and this module does not
//! join that segment to the directory: it enumerates the directory and serves
//! the entry whose name is equal, so the path that is opened is one this
//! process produced. A name carrying a separator, a `..`, or anything else is
//! not equal to any entry and answers 404. Nothing lists the directory to a
//! client.
//!
//! Only a regular file is served. A symlink is an entry of the directory like
//! any other, and following one would let the account that writes here — the
//! only one in the deployment with internet egress — name a file it cannot read
//! itself and have this process read it out. The refusal is on the entry's own
//! type, before anything is opened, because an open follows the link and every
//! check after it describes the target.

use std::path::{Path, PathBuf};

use axum::response::IntoResponse as _;

use crate::serving::{content_type, none_match, not_modified};

/// The manifest the console reads to render its download cards. Named because
/// two things must agree on it: the refresh unit writes it, and the freshness
/// rule below is keyed on it.
pub(crate) const MANIFEST: &str = "downloads.json";

/// The most of a download that is read for a single response.
///
/// The installers are tens of megabytes; this is an order of magnitude of
/// headroom and still refuses to stream something that is not one of them.
/// Over it the answer is 404 rather than a distinct status: this tier tells a
/// client what it can have, and nothing about what it cannot. A zero-byte file
/// is served as an empty 200 for the same reason — the refresh publishes no
/// such file, and inventing a status for one would be describing the
/// directory's contents to a stranger.
const DOWNLOAD_BYTES_MAX: u64 = 512 * 1024 * 1024;

/// The directory the refresh unit writes and this tier reads.
pub(crate) struct Downloads {
    dir: PathBuf,
}

impl Downloads {
    /// Take the directory, refusing at start-up if it is not one.
    ///
    /// Fail-fast for the reason the landing directory fails fast: a deployment
    /// that asked to serve downloads and cannot is a configuration error, and
    /// discovering it at the first seller's click rather than at start-up costs
    /// the one thing a start-up check is for.
    pub(crate) fn open(dir: &Path) -> Result<Self, Box<dyn std::error::Error>> {
        if !dir.is_dir() {
            return Err(format!(
                "{} is not a directory, so there are no downloads to serve; check what {} names",
                dir.display(),
                crate::DOWNLOADS_FLAG
            )
            .into());
        }
        Ok(Self {
            dir: dir.to_path_buf(),
        })
    }

    /// The named regular file, streamed, or 404.
    ///
    /// The path opened comes from this process's own enumeration of the
    /// directory, never from the client's string joined to anything.
    pub(crate) async fn respond(
        &self,
        file: &str,
        if_none_match: Option<&axum::http::HeaderValue>,
    ) -> axum::response::Response {
        let Some(path) = self.entry(file).await else {
            return axum::http::StatusCode::NOT_FOUND.into_response();
        };
        // Opened before it is measured, and measured through the open handle:
        // a refresh landing between a `stat` and an `open` would otherwise put
        // one file's length on another file's bytes, which is a framing error
        // rather than a wrong page.
        let Ok(handle) = tokio::fs::File::open(&path).await else {
            return axum::http::StatusCode::NOT_FOUND.into_response();
        };
        let Ok(metadata) = handle.metadata().await else {
            return axum::http::StatusCode::NOT_FOUND.into_response();
        };
        if !metadata.is_file() || metadata.len() > DOWNLOAD_BYTES_MAX {
            return axum::http::StatusCode::NOT_FOUND.into_response();
        }
        let etag = weak_etag(&metadata);
        let cache = cache_control(file);
        if none_match(if_none_match, &etag) {
            return not_modified(&etag, cache);
        }
        let length = metadata.len().to_string();
        let body = axum::body::Body::from_stream(tokio_util::io::ReaderStream::new(handle));
        (
            [
                (axum::http::header::CONTENT_TYPE, content_type(file)),
                (axum::http::header::CACHE_CONTROL, cache),
                (axum::http::header::ETAG, etag.as_str()),
                (axum::http::header::CONTENT_LENGTH, length.as_str()),
            ],
            body,
        )
            .into_response()
    }

    /// The path of the regular-file entry whose name equals `file`, enumerated
    /// rather than joined. `None` where the directory holds no such entry, or
    /// holds one that is not a regular file.
    ///
    /// The type is the entry's own — `DirEntry::file_type` does not follow a
    /// link — and it is checked here rather than after opening, because
    /// `File::open` follows a symlink and the `fstat` that follows then
    /// describes the target rather than the entry. A link is therefore
    /// invisible to any check made on the open handle.
    ///
    /// This matters more here than in the landing tier, and it is the same
    /// rule. This directory is written by `teachouse-downloads`, the one
    /// account in the deployment with internet egress; the two-unit split
    /// exists so that a compromise there can publish bytes and nothing else. A
    /// followed link would let it name any file `teachouse-api` can read — the
    /// blob key-encryption key, the entitlement signing key, any sealed object
    /// — and have this process serve it.
    async fn entry(&self, file: &str) -> Option<PathBuf> {
        let mut entries = tokio::fs::read_dir(&self.dir).await.ok()?;
        while let Ok(Some(entry)) = entries.next_entry().await {
            if entry.file_name().to_str() != Some(file) {
                continue;
            }
            return entry
                .file_type()
                .await
                .ok()?
                .is_file()
                .then(|| entry.path());
        }
        None
    }
}

/// How long a download may be reused without asking.
///
/// The manifest is the thing that changes at a release and the thing a console
/// reads to decide what to show, so it is never held; the installers beside it
/// carry the version in their names, so an hour is a compromise between a
/// pointless revalidation and a stale card after a refresh.
fn cache_control(file: &str) -> &'static str {
    if file == MANIFEST {
        "no-cache"
    } else {
        "public, max-age=3600"
    }
}

/// A validator over the file's size and modification time rather than its
/// bytes.
///
/// Weak, and marked weak, because it is not a digest: hashing a sixty-megabyte
/// installer on every conditional request would cost more than sending it. The
/// refresh unit replaces the directory's contents atomically, so a changed file
/// changes at least one of the two.
fn weak_etag(metadata: &std::fs::Metadata) -> String {
    let modified = metadata
        .modified()
        .ok()
        .and_then(|at| at.duration_since(std::time::UNIX_EPOCH).ok())
        .map_or(0, |since| since.as_secs());
    format!("W/\"{:x}-{modified:x}\"", metadata.len())
}

#[cfg(test)]
mod tests {
    use super::Downloads;

    /// A directory that is not one is refused at start-up rather than answering
    /// 404 to every seller.
    #[test]
    fn a_missing_directory_is_refused_at_start_up() {
        let missing = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("no-such-downloads");
        let refusal = Downloads::open(&missing)
            .err()
            .expect("a path that is not a directory is refused")
            .to_string();
        assert!(
            refusal.contains("--downloads-dir"),
            "the refusal names the flag that has to be fixed: {refusal}"
        );
    }

    /// Only the manifest is uncacheable; the installers beside it are not.
    #[test]
    fn the_manifest_is_never_held_and_the_installers_are() {
        assert_eq!(
            super::cache_control(super::MANIFEST),
            "no-cache",
            "a console reading a held manifest shows the previous release"
        );
        assert_eq!(
            super::cache_control("Teachouse_0.2.0_x64-setup.exe"),
            "public, max-age=3600",
            "an installer carries its version in its name"
        );
    }

    /// The two installer types are named rather than served as bytes.
    #[test]
    fn each_download_type_is_named() {
        for (file, expected) in [
            ("downloads.json", "application/json"),
            (
                "Teachouse_0.2.0_x64-setup.exe",
                "application/vnd.microsoft.portable-executable",
            ),
            (
                "Teachouse_0.2.0_arm64.apk",
                "application/vnd.android.package-archive",
            ),
            ("SHA256SUMS.txt", "text/plain; charset=utf-8"),
        ] {
            assert_eq!(
                crate::serving::content_type(file),
                expected,
                "{file} is served as {expected}"
            );
        }
    }

    /// A name the directory does not hold is a 404, and the lookup never became
    /// a path.
    ///
    /// The cases below are what [`crate::serving::route`] can never produce —
    /// it yields one segment and refuses a leading dot — asserted here anyway,
    /// because this is the layer that would open the file if it did.
    #[tokio::test]
    async fn a_name_the_directory_does_not_hold_is_never_opened() {
        let here = std::path::Path::new(env!("CARGO_MANIFEST_DIR"));
        let downloads = Downloads::open(here).expect("the crate directory is a directory");
        for name in ["..", "../Cargo.toml", "src/main.rs", "Cargo.toml.bak", ""] {
            assert!(
                downloads.entry(name).await.is_none(),
                "{name} is not an entry of the directory and must not resolve"
            );
        }
        assert!(
            downloads.entry("Cargo.toml").await.is_some(),
            "an entry the directory does hold resolves by enumeration"
        );
    }

    /// Neither a symlink nor a directory is served, whatever it points at.
    ///
    /// The link below points at a file this process can certainly read, which
    /// is the whole shape of the attack: the account that writes this directory
    /// has internet egress and cannot read tam-server's keys, and following a
    /// link would have this process read them on its behalf. The staging
    /// directory is the second case — it lives inside the served directory so
    /// that a rename out of it is atomic, and it must never be an answer.
    #[cfg(unix)]
    #[tokio::test]
    async fn neither_a_symlink_nor_a_directory_is_served() {
        use std::io::Write as _;

        let root = std::env::temp_dir().join(format!(
            "tam-server-downloads-symlink-{}",
            std::process::id()
        ));
        drop(std::fs::remove_dir_all(&root));
        std::fs::create_dir_all(root.join(".staging")).expect("a temporary directory");
        let secret = root.join("pretend-key");
        std::fs::File::create(&secret)
            .expect("a temporary file")
            .write_all(b"the blob key-encryption key")
            .expect("writing the fixture");
        std::fs::File::create(root.join("real.txt"))
            .expect("a temporary file")
            .write_all(b"a published file")
            .expect("writing the fixture");
        std::os::unix::fs::symlink(&secret, root.join("notes.txt")).expect("a symlink");

        let downloads = Downloads::open(&root).expect("a directory");
        assert!(
            downloads.entry("notes.txt").await.is_none(),
            "a symlink is an entry of the directory and is still not served"
        );
        assert!(
            downloads.entry(".staging").await.is_none(),
            "the staging directory is not a file and is never an answer"
        );
        assert!(
            downloads.entry("real.txt").await.is_some(),
            "a regular file beside them still resolves"
        );

        let answer = downloads.respond("notes.txt", None).await;
        assert_eq!(
            answer.status(),
            axum::http::StatusCode::NOT_FOUND,
            "and the response says nothing about what the link pointed at"
        );
        drop(std::fs::remove_dir_all(&root));
    }
}
