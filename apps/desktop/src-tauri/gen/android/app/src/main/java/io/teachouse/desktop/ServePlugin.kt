package io.teachouse.desktop

import android.app.Activity
import app.tauri.annotation.Command
import app.tauri.annotation.TauriPlugin
import app.tauri.plugin.Invoke
import app.tauri.plugin.Plugin

/**
 * Starts and stops [ServeService] for the Rust serving loop.
 *
 * Rust calls `start` as a serving run begins, which is only ever from a pass
 * the seller could see: Android lets an app start a foreground service from
 * the foreground and refuses it from the background. It calls `stop` as the
 * run ends — ten minutes after the seller last used the app or a file was
 * last opened, or on a sign-out.
 *
 * Registered from Rust through `PluginApi::register_android_plugin`, as
 * [SessionKeyPlugin] is, so this needs no Gradle module of its own.
 */
@TauriPlugin
class ServePlugin(private val activity: Activity) : Plugin(activity) {

    @Command
    fun start(invoke: Invoke) {
        try {
            ServeService.start(activity.applicationContext)
            invoke.resolve()
        } catch (failure: Exception) {
            invoke.reject(failure.message ?: failure.javaClass.simpleName)
        }
    }

    @Command
    fun stop(invoke: Invoke) {
        try {
            ServeService.stop(activity.applicationContext)
            invoke.resolve()
        } catch (failure: Exception) {
            invoke.reject(failure.message ?: failure.javaClass.simpleName)
        }
    }
}
