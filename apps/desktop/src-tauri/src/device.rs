//! Which machine this installation is.
//!
//! A random identifier written once into application data, labelled with the
//! hostname. Deliberately not a hardware or machine UUID: decision D14 in
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
/// The label is refreshed from the current hostname on every read while the id
/// is not, because a renamed machine is the same device and a re-registered
/// one would strand the entitlement token bound to the old id.
#[expect(
    clippy::disallowed_methods,
    reason = "the ban targets upload payloads, which are bounded but not small; this file is a \
              UUID and a hostname that this process wrote itself"
)]
pub fn load_or_create(data_dir: &Path, hostname: &str) -> Result<DeviceIdentity, DeviceError> {
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
            label: hostname.to_owned(),
        };
        if refreshed.label != stored.label {
            write_identity(&path, &refreshed)?;
        }
        return Ok(refreshed);
    }

    let fresh = DeviceIdentity {
        id: DeviceId::generate(),
        label: hostname.to_owned(),
    };
    fs::create_dir_all(data_dir).map_err(|why| DeviceError::Io(why.to_string()))?;
    write_identity(&path, &fresh)?;
    Ok(fresh)
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
