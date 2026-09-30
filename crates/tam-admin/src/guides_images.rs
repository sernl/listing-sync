//! The guide pictures as files: `guides images --dir docs/guides/images`
//! seals every picture into the blob store under the platform organisation,
//! and `guides check --dir docs/guides` proves, without a database, that the
//! handles the guides name and the pictures the directory holds are one set.
//!
//! A guide names a picture by its handle, the BLAKE3 hash of its bytes
//! (`/v1/guides/images/<handle>`). That is what makes placing the pictures
//! idempotent with nothing to remember: the same bytes are the same handle,
//! and [`BlobRepo::put_once`] writes neither the row nor the object for bytes
//! the platform organisation already holds. It is also what makes the check
//! exact: a retaken screenshot is a new handle, so a guide still naming the
//! old one is a guide pointing at a picture this corpus no longer carries.
//!
//! The pictures are written straight into the blob table and the object store
//! rather than posted to `POST /{version}/admin/guides/images`: this runs in
//! the migration step, before any server is up and with no operator session.
//! The write is the one that route makes -- [`ensure_platform_org`] and then
//! the seal-and-store put -- so a picture placed here and a picture uploaded
//! in the console are the same row.

use std::collections::BTreeSet;
use std::path::{Path, PathBuf};

use tam_pipeline::store::ObjectStore;
use tam_storage::{ensure_platform_org, BlobPut, BlobRepo};
use tam_types::{ContentHash, FileKind, Timestamp};

type Failure = Box<dyn std::error::Error>;

/// Where a guide body addresses an uploaded picture.
const IMAGE_ROUTE: &str = "/v1/guides/images/";

/// A handle is 32 bytes of hash spelled as lowercase hex.
const HANDLE_LEN: usize = 64;

/// The file endings read as pictures. Anything else in the directory -- a
/// README, an editor's swap file -- is not a picture and is passed over.
const PICTURE_EXTENSIONS: [&str; 5] = ["png", "jpg", "jpeg", "gif", "webp"];

/// A handle as every route spells it: the hash as lowercase hex.
#[must_use]
pub(crate) fn handle_of(hash: ContentHash) -> String {
    use core::fmt::Write as _;
    let mut hex = String::with_capacity(HANDLE_LEN);
    for byte in hash.0 {
        let _unused: core::fmt::Result = write!(hex, "{byte:02x}");
    }
    hex
}

/// Every picture file in `dir`, in name order so a report reads the same on
/// every run.
pub(crate) fn picture_paths(dir: &Path) -> Result<Vec<PathBuf>, Failure> {
    let mut paths = Vec::new();
    for entry in std::fs::read_dir(dir).map_err(|error| format!("{}: {error}", dir.display()))? {
        let path = entry?.path();
        let picture = path
            .extension()
            .and_then(|extension| extension.to_str())
            .is_some_and(|extension| {
                PICTURE_EXTENSIONS
                    .iter()
                    .any(|known| known.eq_ignore_ascii_case(extension))
            });
        if picture && path.is_file() {
            paths.push(path);
        }
    }
    paths.sort();
    Ok(paths)
}

#[expect(
    clippy::disallowed_methods,
    reason = "a guide picture is a repository screenshot, read one at a time by a one-shot command and dropped before the next; the ban is about unbounded uploads"
)]
fn read_picture(path: &Path) -> Result<Vec<u8>, Failure> {
    std::fs::read(path).map_err(|error| format!("{}: {error}", path.display()).into())
}

fn file_name(path: &Path) -> String {
    path.file_name()
        .map_or_else(String::new, |name| name.to_string_lossy().into_owned())
}

/// Every handle a guide body names, in the order it names them.
fn handles_named_in(text: &str) -> Vec<String> {
    text.match_indices(IMAGE_ROUTE)
        .filter_map(|(at, route)| {
            let rest = text.get(at + route.len()..)?;
            let handle = rest.get(..HANDLE_LEN)?;
            let tail_is_hex = rest
                .get(HANDLE_LEN..)
                .and_then(|tail| tail.chars().next())
                .is_none_or(|next| !next.is_ascii_hexdigit());
            (handle
                .chars()
                .all(|c| c.is_ascii_digit() || ('a'..='f').contains(&c))
                && tail_is_hex)
                .then(|| handle.to_owned())
        })
        .collect()
}

/// What `guides check` found.
#[derive(Debug, Default, PartialEq, Eq)]
pub(crate) struct Check {
    /// `(guide file, handle)`: a guide names a picture no file is.
    pub(crate) missing: Vec<(String, String)>,
    /// `(picture file, handle)`: a picture no guide names.
    pub(crate) stray: Vec<(String, String)>,
    pub(crate) guides: usize,
    pub(crate) pictures: usize,
}

impl Check {
    #[must_use]
    pub(crate) const fn passes(&self) -> bool {
        self.missing.is_empty() && self.stray.is_empty()
    }

    #[must_use]
    pub(crate) fn report(&self) -> String {
        let mut lines: Vec<String> = self
            .missing
            .iter()
            .map(|(guide, handle)| {
                format!("missing\t{guide} names {handle}, which no picture under images/ is")
            })
            .chain(self.stray.iter().map(|(picture, handle)| {
                format!("stray\t{picture} ({handle}) is named by no guide")
            }))
            .collect();
        lines.push(format!(
            "{} guide(s), {} picture(s): {} missing, {} stray",
            self.guides,
            self.pictures,
            self.missing.len(),
            self.stray.len()
        ));
        lines.join("\n")
    }
}

/// The guides in `guides_dir` against the pictures in `guides_dir/images`.
pub(crate) fn check(guides_dir: &Path) -> Result<Check, Failure> {
    let mut named = BTreeSet::new();
    let mut check = Check::default();
    let mut guides: Vec<PathBuf> = std::fs::read_dir(guides_dir)
        .map_err(|error| format!("{}: {error}", guides_dir.display()))?
        .filter_map(|entry| entry.ok().map(|entry| entry.path()))
        .filter(|path| path.extension().is_some_and(|extension| extension == "md"))
        .collect();
    guides.sort();
    let mut references = BTreeSet::new();
    for guide in &guides {
        let text = crate::guides_seed::read_to_string(guide)
            .map_err(|error| format!("{}: {error}", guide.display()))?;
        for handle in handles_named_in(&text) {
            named.insert(handle.clone());
            references.insert((file_name(guide), handle));
        }
    }
    check.guides = guides.len();

    let mut held = BTreeSet::new();
    for picture in picture_paths(&guides_dir.join("images"))? {
        let bytes = read_picture(&picture)?;
        let handle = handle_of(tam_pipeline::hash::content_hash(&bytes));
        if !named.contains(&handle) {
            check.stray.push((file_name(&picture), handle.clone()));
        }
        held.insert(handle);
        check.pictures += 1;
    }
    check.missing = references
        .into_iter()
        .filter(|(_, handle)| !held.contains(handle))
        .collect();
    Ok(check)
}

/// What placing one picture did.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct Placed {
    pub(crate) file: String,
    pub(crate) handle: String,
    pub(crate) stored: bool,
}

/// Every picture in `dir`, sealed and stored under the platform organisation.
///
/// One file at a time, read, stored and dropped, so the pass holds one
/// picture rather than the corpus. A file whose bytes are not a picture stops
/// the pass: the upload route refuses the same bytes, and a guide pointing at
/// a handle the reader then serves as an image would be a lie told by the
/// file ending.
pub(crate) async fn place<S: ObjectStore>(
    pool: &sqlx::PgPool,
    repo: &BlobRepo<S>,
    dir: &Path,
    at: Timestamp,
) -> Result<Vec<Placed>, Failure> {
    let paths = picture_paths(dir)?;
    if paths.is_empty() {
        return Err(format!("{} holds no pictures", dir.display()).into());
    }
    let org = ensure_platform_org(pool, at).await?;
    let mut placed = Vec::with_capacity(paths.len());
    for path in paths {
        let bytes = read_picture(&path)?;
        if tam_pipeline::probe::probe_kind(&bytes) != Some(FileKind::Image) {
            return Err(
                format!("{} is not a PNG, JPEG, GIF or WebP picture", path.display()).into(),
            );
        }
        let put = repo.put_once(org, &bytes, at).await?;
        placed.push(Placed {
            file: file_name(&path),
            handle: handle_of(put.hash()),
            stored: matches!(put, BlobPut::Stored(_)),
        });
    }
    Ok(placed)
}

/// How the command prints what it did.
#[must_use]
pub(crate) fn report(placed: &[Placed]) -> String {
    let stored = placed.iter().filter(|picture| picture.stored).count();
    let mut lines: Vec<String> = placed
        .iter()
        .map(|picture| {
            let outcome = if picture.stored {
                "stored"
            } else {
                "unchanged"
            };
            format!("{}\t{}\t{outcome}", picture.handle, picture.file)
        })
        .collect();
    lines.push(format!(
        "{} picture(s): {stored} stored, {} unchanged",
        placed.len(),
        placed.len() - stored
    ));
    lines.join("\n")
}

#[cfg(test)]
mod tests {
    use super::{check, handle_of, handles_named_in};

    const HANDLE: &str = "0123456789abcdef0123456789abcdef0123456789abcdef0123456789abcdef";

    #[test]
    fn a_handle_is_read_only_where_it_is_whole() {
        let text = format!(
            "![one](/v1/guides/images/{HANDLE})\n\
             ![short](/v1/guides/images/abc)\n\
             ![long](/v1/guides/images/{HANDLE}0)\n\
             ![upper](/v1/guides/images/{})\n",
            HANDLE.to_uppercase()
        );
        assert_eq!(
            handles_named_in(&text),
            vec![HANDLE.to_owned()],
            "a short, an over-long and an upper-case spelling are not handles the reader serves"
        );
    }

    #[test]
    fn the_check_names_both_directions_of_a_mismatch() {
        let dir = std::env::temp_dir().join(format!("tam-admin-check-{}", std::process::id()));
        let images = dir.join("images");
        std::fs::create_dir_all(&images).expect("the scratch directory is created");
        let kept = b"\x89PNG\r\n\x1a\nkept".to_vec();
        let named = handle_of(tam_pipeline::hash::content_hash(&kept));
        std::fs::write(images.join("kept.png"), &kept).expect("writes");
        std::fs::write(images.join("stray.png"), b"\x89PNG\r\n\x1a\nstray").expect("writes");
        std::fs::write(
            dir.join("guide.md"),
            format!("![a](/v1/guides/images/{named})\n![b](/v1/guides/images/{HANDLE})\n"),
        )
        .expect("writes");

        let found = check(&dir).expect("the check reads");
        std::fs::remove_dir_all(&dir).expect("the scratch directory is removed");
        assert!(!found.passes(), "a mismatch fails the check");
        assert_eq!(
            found.missing,
            vec![("guide.md".to_owned(), HANDLE.to_owned())],
            "the handle no picture is, and the guide naming it"
        );
        assert_eq!(
            found
                .stray
                .iter()
                .map(|(file, _)| file.as_str())
                .collect::<Vec<_>>(),
            vec!["stray.png"],
            "the picture no guide names"
        );
    }
}
