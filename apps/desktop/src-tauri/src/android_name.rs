//! What an Android build calls this phone in the seller's machine list.
//!
//! The desktop label is the hostname, and on Android that is the wrong
//! source: `tauri_plugin_os::hostname` is `gethostname` (tauri-plugin-os
//! 2.3.2, `src/lib.rs:96-99`), which in an Android application process answers
//! a loopback name, so a registered phone would appear in "Your machines" as
//! `localhost`. What the seller recognises is what is printed on the box, and
//! `Build.MANUFACTURER` and `Build.MODEL` are it.
//!
//! Read through the same Kotlin class the sealing key is read through, and for
//! the same reason: `PluginApi::register_android_plugin` hands back a
//! [`PluginHandle`] whose `run_mobile_plugin` calls a `@Command` on it (tauri
//! 2.11.5, `src/plugin/mobile.rs:324`), so this needs no Gradle module, no
//! second plugin crate and no direct `jni` dependency.
//!
//! Nothing here is a secret and nothing here is a capability: the name is a
//! string the phone shows in its own settings screen, and the server decides
//! what a device may do from its id and its organisation, never from its name.

use serde::Deserialize;
use tauri::plugin::PluginHandle;
use tauri::Runtime;

use crate::device::android_label;

/// The `@Command` the Kotlin side answers. Matched by name at run time, so a
/// rename on either side fails at the first call rather than at compile time —
/// the same hazard recorded for `obtain` and `forget`.
pub const COMMAND: &str = "deviceName";

/// Two Java fields, each of which Kotlin may find null and sends as an empty
/// string. The formatter takes an empty field as "the phone said nothing"
/// rather than rendering it.
#[derive(Deserialize)]
struct Reported {
    manufacturer: String,
    model: String,
}

/// Where a build reads its own name from.
///
/// A trait so the handle's runtime parameter is erased at the managed-state
/// boundary, which is how the sealing key already reaches the same start-up
/// closure.
pub trait DeviceNameSource: Send + Sync {
    /// The phone's name, or `None` when the bridge did not answer.
    ///
    /// `None` rather than a fabricated name: the caller has a stated fallback
    /// and a phone whose name we invented would be worse than one that admits
    /// it is an Android phone and nothing more.
    fn label(&self) -> Option<String>;
}

pub struct PhoneName<R: Runtime> {
    handle: PluginHandle<R>,
}

impl<R: Runtime> core::fmt::Debug for PhoneName<R> {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        f.write_str("PhoneName")
    }
}

impl<R: Runtime> PhoneName<R> {
    #[must_use]
    pub const fn new(handle: PluginHandle<R>) -> Self {
        Self { handle }
    }
}

impl<R: Runtime> DeviceNameSource for PhoneName<R> {
    fn label(&self) -> Option<String> {
        match self.handle.run_mobile_plugin::<Reported>(COMMAND, ()) {
            Ok(reported) => Some(android_label(&reported.manufacturer, &reported.model)),
            Err(why) => {
                // Not fatal and not silent. A phone that cannot say its own
                // name still registers, still syncs and still obeys a
                // sign-out; only the row's wording is poorer, and the log line
                // is what tells a developer the Kotlin command was renamed.
                eprintln!("the phone's own name could not be read: {why}");
                None
            }
        }
    }
}
