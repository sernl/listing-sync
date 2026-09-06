//! Which machine this installation is.
//!
//! A random identifier written once into application data, labelled with the
//! hostname on a computer and with the phone's own model on Android, where the
//! hostname is a loopback name no seller would recognise. Deliberately not a
//! hardware or machine UUID: decision D14 in
//! `docs/notes/design/vendoo-for-teachers-rethink.md` records that `machine-uid`
//! covers neither Android nor iOS, and D2 puts both on the roadmap, so a
//! self-generated id is the only one that survives the whole surface set.
//!
//! The identifier is not a secret and is not a capability. It names a device
//! in the server's registry and in the entitlement token, and the server
//! decides what that device may do.

use std::fs;
use std::path::{Path, PathBuf};

use serde::{Deserialize, Serialize};

/// The file the identity is written to, inside the application data directory.
pub const IDENTITY_FILE: &str = "device.json";

/// A device identifier: a version-4 UUID in its simple, unhyphenated spelling.
#[derive(Debug, Clone, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub struct DeviceId(String);

impl DeviceId {
    #[must_use]
    pub fn generate() -> Self {
        Self(uuid::Uuid::new_v4().simple().to_string())
    }

    /// Accepts a value produced elsewhere, such as one read back from disk or
    /// written into a test fixture.
    #[must_use]
    pub fn from_raw(raw: &str) -> Self {
        Self(raw.to_owned())
    }

    #[must_use]
    pub fn as_str(&self) -> &str {
        &self.0
    }
}

impl core::fmt::Display for DeviceId {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        f.write_str(&self.0)
    }
}

/// The identity as the server's device registry sees it: a stable id and a
/// label a human recognises in a "Your devices" list.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct DeviceIdentity {
    pub id: DeviceId,
    pub label: String,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum DeviceError {
    Io(String),
    Codec(String),
}

impl core::fmt::Display for DeviceError {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        match self {
            Self::Io(why) => write!(f, "the device identity file is unreadable: {why}"),
            Self::Codec(why) => write!(f, "the device identity file did not parse: {why}"),
        }
    }
}

impl core::error::Error for DeviceError {}

#[must_use]
fn identity_path(data_dir: &Path) -> PathBuf {
    data_dir.join(IDENTITY_FILE)
}

/// Reads the device identity, generating and writing one on first run.
///
/// The label is refreshed from the current source on every read while the id is
/// not, because a renamed machine is the same device and a re-registered one
/// would strand the entitlement token bound to the old id. That is also what
/// lets the source itself improve: a phone already registered under a hostname
/// takes its model on the next launch with no migration and without becoming a
/// second machine.
#[expect(
    clippy::disallowed_methods,
    reason = "the ban targets upload payloads, which are bounded but not small; this file is a \
              UUID and a device label that this process wrote itself"
)]
pub fn load_or_create(data_dir: &Path, label: &str) -> Result<DeviceIdentity, DeviceError> {
    let path = identity_path(data_dir);
    let existing = match fs::read_to_string(&path) {
        Ok(text) => Some(text),
        Err(why) if why.kind() == std::io::ErrorKind::NotFound => None,
        Err(why) => return Err(DeviceError::Io(why.to_string())),
    };

    if let Some(text) = existing {
        let stored: DeviceIdentity =
            serde_json::from_str(&text).map_err(|why| DeviceError::Codec(why.to_string()))?;
        let refreshed = DeviceIdentity {
            id: stored.id,
            label: label.to_owned(),
        };
        if refreshed.label != stored.label {
            write_identity(&path, &refreshed)?;
        }
        return Ok(refreshed);
    }

    let fresh = DeviceIdentity {
        id: DeviceId::generate(),
        label: label.to_owned(),
    };
    fs::create_dir_all(data_dir).map_err(|why| DeviceError::Io(why.to_string()))?;
    write_identity(&path, &fresh)?;
    Ok(fresh)
}

/// What an Android build calls itself when the phone reports nothing to call
/// it. Stated rather than left blank, because a nameless row in "Your
/// machines" is one the seller cannot tell from any other.
pub const ANDROID_FALLBACK_LABEL: &str = "Android phone";

/// The name a phone goes by, built from what Android reports about itself.
///
/// `tauri_plugin_os::hostname` is `gethostname` (tauri-plugin-os 2.3.2,
/// `src/lib.rs:96-99`), which on an Android application process answers a
/// loopback name: a registered phone would appear in the seller's list as
/// `localhost`. `Build.MANUFACTURER` and `Build.MODEL` are what is printed on
/// the box, need no Android permission and no extra plugin, and are what the
/// Kotlin bridge reads instead.
///
/// Most vendors already put their own name in the model — "Xiaomi Redmi Note
/// 8" against a manufacturer of "Xiaomi" — so the prefix is dropped when it is
/// already there and the label does not stutter. Several vendors report the
/// manufacturer lowercase ("samsung"), which is Android's identifier for the
/// maker rather than how the maker writes its own name, so a leading lowercase
/// letter is capitalised and nothing else is touched: the label presents the
/// name a seller recognises without inventing a spelling for "OnePlus" or
/// "HUAWEI", and the model stays exactly as the phone reports it.
#[must_use]
pub fn android_label(manufacturer: &str, model: &str) -> String {
    let manufacturer = as_the_maker_writes_it(manufacturer.trim());
    let model = model.trim();
    if model.is_empty() {
        return if manufacturer.is_empty() {
            ANDROID_FALLBACK_LABEL.to_owned()
        } else {
            manufacturer
        };
    }
    if manufacturer.is_empty() || names_its_own_maker(model, &manufacturer) {
        return model.to_owned();
    }
    format!("{manufacturer} {model}")
}

/// The manufacturer with a leading lowercase ASCII letter raised, which is the
/// whole of the change: the rest of the string, and any name that does not
/// open with such a letter, is left as reported.
fn as_the_maker_writes_it(manufacturer: &str) -> String {
    let mut characters = manufacturer.chars();
    let Some(first) = characters.next() else {
        return manufacturer.to_owned();
    };
    if first.is_ascii_lowercase() {
        format!("{}{}", first.to_ascii_uppercase(), characters.as_str())
    } else {
        manufacturer.to_owned()
    }
}

/// Whether the model already opens with the manufacturer's name, compared
/// without case because the two fields are not spelled consistently even on
/// one device ("samsung" against "Samsung Galaxy").
fn names_its_own_maker(model: &str, manufacturer: &str) -> bool {
    model
        .get(..manufacturer.len())
        .is_some_and(|opening| opening.eq_ignore_ascii_case(manufacturer))
}

fn write_identity(path: &Path, identity: &DeviceIdentity) -> Result<(), DeviceError> {
    let encoded = serde_json::to_string_pretty(identity)
        .map_err(|why| DeviceError::Codec(why.to_string()))?;
    fs::write(path, encoded).map_err(|why| DeviceError::Io(why.to_string()))
}

#[cfg(test)]
mod tests {
    use super::{load_or_create, DeviceId, IDENTITY_FILE};
    use std::path::PathBuf;

    fn scratch() -> PathBuf {
        let dir = std::env::temp_dir().join(format!("tam-desktop-device-{}", DeviceId::generate()));
        std::fs::create_dir_all(&dir).expect("the scratch directory is creatable");
        dir
    }

    #[test]
    fn the_identifier_is_generated_once_and_then_reused() {
        let dir = scratch();
        let first = load_or_create(&dir, "founder-pc").expect("a first run generates an identity");
        let second = load_or_create(&dir, "founder-pc").expect("a second run reads it back");
        assert_eq!(
            first, second,
            "a device that re-registered on every launch would strand every token bound to it"
        );
        assert!(
            dir.join(IDENTITY_FILE).exists(),
            "the identity is written to application data, not held only in memory"
        );
        std::fs::remove_dir_all(&dir).ok();
    }

    #[test]
    fn a_renamed_machine_keeps_its_identifier_and_gains_the_new_label() {
        let dir = scratch();
        let before = load_or_create(&dir, "founder-pc").expect("a first run generates an identity");
        let after = load_or_create(&dir, "studio-pc").expect("a rename still reads back");
        assert_eq!(
            before.id, after.id,
            "renaming a machine does not make it a different device"
        );
        assert_eq!(after.label, "studio-pc", "the label follows the hostname");
        let reread = load_or_create(&dir, "studio-pc").expect("the new label persisted");
        assert_eq!(reread.label, "studio-pc");
        std::fs::remove_dir_all(&dir).ok();
    }

    #[test]
    fn a_phone_is_named_by_what_it_says_it_is_rather_than_by_its_hostname() {
        use super::{android_label, ANDROID_FALLBACK_LABEL};

        assert_eq!(
            android_label("Google", "Pixel 8"),
            "Google Pixel 8",
            "a model that does not name its maker gets it prefixed"
        );
        assert_eq!(
            android_label("samsung", "SM-G991B"),
            "Samsung SM-G991B",
            "the lowercase manufacturer Android reports is an identifier, and the seller's \
             list shows the maker's name"
        );
        assert_eq!(
            android_label("OnePlus", "CPH2451"),
            "OnePlus CPH2451",
            "a name already capitalised keeps every letter it came with"
        );
        assert_eq!(
            android_label("HUAWEI", "ELS-NX9"),
            "HUAWEI ELS-NX9",
            "and a maker that shouts its own name is not tidied into one that does not"
        );
        assert_eq!(
            android_label("Xiaomi", "Xiaomi Redmi Note 8"),
            "Xiaomi Redmi Note 8",
            "a model that already opens with its maker must not stutter"
        );
        assert_eq!(
            android_label("samsung", "Samsung Galaxy A14"),
            "Samsung Galaxy A14",
            "and the two fields are not spelled alike even on one device"
        );
        assert_eq!(
            android_label("  Google  ", "  Pixel 8  "),
            "Google Pixel 8",
            "whitespace off either field would otherwise reach the registry"
        );
        assert_eq!(android_label("Nothing", ""), "Nothing");
        assert_eq!(
            android_label("samsung", ""),
            "Samsung",
            "a phone that reports only its maker is labelled with that maker's name"
        );
        assert_eq!(android_label("", "Pixel 8"), "Pixel 8");
        assert_eq!(
            android_label("", ""),
            ANDROID_FALLBACK_LABEL,
            "a phone that says nothing about itself still gets a name a seller can read"
        );
    }

    #[test]
    fn a_phone_that_reports_a_new_name_keeps_its_identifier() {
        let dir = scratch();
        let before = super::load_or_create(&dir, &super::android_label("Google", "Pixel 8"))
            .expect("a first run generates an identity");
        let after = super::load_or_create(&dir, &super::android_label("Google", "Pixel 9"))
            .expect("a replaced label still reads back");
        assert_eq!(
            before.id, after.id,
            "a phone that became a second machine on a label change would strand \
             the entitlement token bound to the first"
        );
        assert_eq!(
            after.label, "Google Pixel 9",
            "the label follows whatever source it was given, hostname or not"
        );
        std::fs::remove_dir_all(&dir).ok();
    }

    #[test]
    fn two_devices_do_not_collide() {
        let (one, two) = (scratch(), scratch());
        let first = load_or_create(&one, "a").expect("one identity");
        let second = load_or_create(&two, "b").expect("another identity");
        assert_ne!(
            first.id, second.id,
            "the identifier is random per installation"
        );
        std::fs::remove_dir_all(&one).ok();
        std::fs::remove_dir_all(&two).ok();
    }
}
