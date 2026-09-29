//! What keeps a phone answering the files the seller opens in a browser once
//! the screen is off.
//!
//! [`crate::serve`] decides when a phone serves; this is how it asks Android
//! to let it. While a serving run lasts, the Kotlin `ServePlugin` holds a
//! foreground service of type `dataSync`, with the ongoing notification
//! "Teachouse is sharing your files" and a partial wake lock, so the process
//! keeps its network and its CPU with the screen off. When the run ends the
//! service stops and takes the wake lock and the notification with it.
//!
//! Registered through `PluginApi::register_android_plugin` exactly as the
//! session key's class is, for the same reasons: no Gradle module, no second
//! plugin crate, and no direct `jni` dependency. A second class rather than
//! two more commands on that one, because the service is a lifecycle of its
//! own and the keystore class should not have to know about it.
//!
//! The limits are Android's: some manufacturers' battery savers stop a
//! foreground service anyway, and Android 15 ends a `dataSync` service after
//! six hours in a day. Either is a phone that stops polling, which the console
//! reads as offline, and opening the app starts serving again.

use std::sync::Arc;

use tauri::plugin::PluginHandle;
use tauri::{Manager, Runtime};

use crate::android_name::ForegroundSource;
use crate::serve::Presence;

/// The Kotlin class, under the package the session key's class is in.
pub const PLUGIN_CLASS: &str = "ServePlugin";

/// The `@Command`s the Kotlin side answers. Matched by name at run time, so a
/// rename on either side fails at the first call rather than at compile time,
/// and the failure is logged where it happens.
pub const START_COMMAND: &str = "start";
pub const STOP_COMMAND: &str = "stop";

/// A phone's [`Presence`]: the activity count the coordinator already reads,
/// and the service that keeps a serving run alive.
pub struct PhonePresence<R: Runtime> {
    here: Arc<dyn ForegroundSource>,
    handle: PluginHandle<R>,
}

impl<R: Runtime> core::fmt::Debug for PhonePresence<R> {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        f.write_str("PhonePresence")
    }
}

impl<R: Runtime> Presence for PhonePresence<R> {
    fn foreground(&self) -> bool {
        self.here.foreground()
    }

    fn keep_serving(&self, on: bool) {
        let command = if on { START_COMMAND } else { STOP_COMMAND };
        if let Err(why) = self.handle.run_mobile_plugin::<serde_json::Value>(command, ()) {
            // Not fatal: without the service a phone still serves while the
            // app is on screen, and only answering with the screen off is
            // lost. The line is what tells a developer the command was
            // renamed or the service refused to start.
            eprintln!("the phone could not {command} sharing files in the background: {why}");
        }
    }
}

/// Registers `ServePlugin` and files a [`Presence`] for `setup` to read.
///
/// Must be registered after [`crate::run`]'s session-key bridge, whose
/// [`ForegroundSource`] this reads.
pub fn serve_bridge<R: Runtime>() -> tauri::plugin::TauriPlugin<R> {
    tauri::plugin::Builder::new("serve")
        .setup(|app, api| {
            let handle = api.register_android_plugin(
                crate::session::android_key::PLUGIN_IDENTIFIER,
                PLUGIN_CLASS,
            )?;
            let here = Arc::clone(app.state::<Arc<dyn ForegroundSource>>().inner());
            let presence: Arc<dyn Presence> = Arc::new(PhonePresence { here, handle });
            app.manage(presence);
            Ok(())
        })
        .build()
}
