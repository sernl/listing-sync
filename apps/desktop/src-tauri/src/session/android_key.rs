//! The Android half of [`DeviceKeySource`]: the sealing secret, held by the
//! Android Keystore.
//!
//! The secret is thirty-two random bytes generated once on the device and
//! never written in the clear. An AES-GCM key inside the Keystore wraps it,
//! and only the wrapped blob reaches the filesystem; the Keystore key is not
//! exportable, so the file without the device is ciphertext and nothing more.
//! The Kotlin that does it is
//! `gen/android/app/src/main/java/io/teachouse/desktop/SessionKeyPlugin.kt`.
//!
//! It is reached through Tauri's own mobile bridge rather than through a
//! separate plugin crate: `PluginApi::register_android_plugin` hands back a
//! [`PluginHandle`] and `run_mobile_plugin` calls a `@Command` on it (tauri
//! 2.11.5, `src/plugin/mobile.rs:208` and `:324`), so this needs no Gradle
//! module of its own and no direct `jni` dependency.

use serde::Deserialize;
use tam_secrets::Kek;
use tauri::plugin::PluginHandle;
use tauri::Runtime;

use super::encrypted::DeviceKeySource;
use super::StoreError;

/// The Android package the Kotlin class lives in, and the class itself. Both
/// are matched by name at runtime, so a rename on either side is a failure at
/// the first call rather than at compile time.
pub const PLUGIN_IDENTIFIER: &str = "io.teachouse.desktop";
pub const PLUGIN_CLASS: &str = "SessionKeyPlugin";

/// The Kotlin side answers with the secret as an array of byte-valued
/// numbers, which is what `org.json` can carry and what `serde_json` reads
/// back into a `Vec<u8>` without a base64 or hex crate in between.
#[derive(Deserialize)]
struct KeyResponse {
    key: Vec<u8>,
}

pub struct KeystoreKey<R: Runtime> {
    handle: PluginHandle<R>,
}

/// Prints nothing but the type's name. The handle is a bridge to a secret,
/// and a derive here would be one `{:?}` away from a leak.
impl<R: Runtime> core::fmt::Debug for KeystoreKey<R> {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        f.write_str("KeystoreKey")
    }
}

impl<R: Runtime> KeystoreKey<R> {
    #[must_use]
    pub const fn new(handle: PluginHandle<R>) -> Self {
        Self { handle }
    }
}

impl<R: Runtime> DeviceKeySource for KeystoreKey<R> {
    fn obtain(&self) -> Result<Kek, StoreError> {
        let response: KeyResponse = self
            .handle
            .run_mobile_plugin("obtain", ())
            .map_err(|why| StoreError::Backend(why.to_string()))?;
        // `Kek::from_bytes` is the length check: a secret of the wrong size
        // is refused here rather than producing a key that opens nothing.
        Kek::from_bytes(&response.key).map_err(|why| StoreError::Backend(why.to_string()))
    }

    fn forget(&self) -> Result<(), StoreError> {
        self.handle
            .run_mobile_plugin::<()>("forget", ())
            .map_err(|why| StoreError::Backend(why.to_string()))
    }
}
