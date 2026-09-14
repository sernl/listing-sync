package io.teachouse.desktop

import android.app.Activity
import android.os.Build
import android.security.keystore.KeyGenParameterSpec
import android.security.keystore.KeyProperties
import android.security.keystore.StrongBoxUnavailableException
import app.tauri.annotation.Command
import app.tauri.annotation.TauriPlugin
import app.tauri.plugin.Invoke
import app.tauri.plugin.JSArray
import app.tauri.plugin.JSObject
import app.tauri.plugin.Plugin
import java.io.File
import java.security.KeyStore
import java.security.SecureRandom
import javax.crypto.Cipher
import javax.crypto.KeyGenerator
import javax.crypto.SecretKey
import javax.crypto.spec.GCMParameterSpec

/**
 * Where the Android build's session-sealing secret lives.
 *
 * The secret itself is thirty-two random bytes generated once. It is never
 * stored in the clear: an AES-GCM key held by the Android Keystore wraps it,
 * and only the wrapped blob reaches the filesystem. The Keystore key is not
 * exportable, so an attacker with the file and without the device has
 * ciphertext and nothing else, and an uninstall destroys the key and with it
 * every session sealed under the secret.
 *
 * Registered from Rust through `PluginApi::register_android_plugin`, so this
 * needs no separate plugin crate and no Gradle module of its own.
 */
@TauriPlugin
class SessionKeyPlugin(private val activity: Activity) : Plugin(activity) {

    private companion object {
        const val KEYSTORE = "AndroidKeyStore"
        const val ALIAS = "io.teachouse.desktop.session-key"
        const val BLOB = "session-key.bin"
        const val SECRET_BYTES = 32
        /** AES-GCM's nominal IV length; anything else costs performance and buys nothing. */
        const val IV_BYTES = 12
        const val TAG_BITS = 128
    }

    private val blob: File
        get() = File(activity.filesDir, BLOB)

    /** The secret, generated on first call and unwrapped on every call after. */
    @Command
    fun obtain(invoke: Invoke) {
        try {
            val secret = if (blob.exists()) unwrap(blob.readBytes()) else create()
            val bytes = JSArray()
            for (byte in secret) {
                bytes.put(byte.toInt() and 0xFF)
            }
            secret.fill(0)
            invoke.resolve(JSObject().put("key", bytes))
        } catch (failure: Exception) {
            invoke.reject(failure.message ?: failure.javaClass.simpleName)
        }
    }

    /**
     * What this phone calls itself, for the row it takes in the seller's
     * machine list.
     *
     * `Build.MANUFACTURER` and `Build.MODEL` need no Android permission and no
     * further plugin, which is why they are read here rather than
     * `Settings.Global.DEVICE_NAME` or the Bluetooth adapter name. Both are
     * Java fields and therefore nullable from Kotlin's side; an empty string
     * is what the Rust formatter takes as "the phone said nothing".
     */
    @Command
    fun deviceName(invoke: Invoke) {
        invoke.resolve(
            JSObject()
                .put("manufacturer", Build.MANUFACTURER ?: "")
                .put("model", Build.MODEL ?: ""),
        )
    }

    /**
     * Whether the seller is looking at this application right now.
     *
     * Read from `MainActivity`'s process-lifetime started count rather than
     * from the Activity this plugin was constructed with. Tauri keeps its
     * plugin instances across an Activity recreation (its PluginManager
     * returns early from `onActivityCreate` after the first initialisation,
     * tauri 2.11.5), so the captured Activity outlives its own window: after a
     * configuration change this manifest does not handle — fontScale,
     * density — it sits at DESTROYED while a replacement is on screen, and
     * asking it would answer "not here" for the rest of the session.
     *
     * The count is incremented in `onStart` and decremented in `onStop`, so a
     * split-screen or partly covered window still counts: a seller working in
     * one is as present as a seller with it full-screen. Zero is a phone in a
     * pocket, and the client asks the network for nothing there.
     */
    @Command
    fun foreground(invoke: Invoke) {
        invoke.resolve(JSObject().put("foreground", MainActivity.isOnScreen()))
    }

    /**
     * Destroys the key and the wrapped secret.
     *
     * Deleting the Keystore entry is what makes this a wipe rather than a
     * deletion: any copy of the sealed session file that survived elsewhere
     * becomes permanently unreadable, because the key that could open it no
     * longer exists anywhere.
     */
    @Command
    fun forget(invoke: Invoke) {
        try {
            blob.delete()
            keystore().deleteEntry(ALIAS)
            invoke.resolve()
        } catch (failure: Exception) {
            invoke.reject(failure.message ?: failure.javaClass.simpleName)
        }
    }

    private fun keystore(): KeyStore = KeyStore.getInstance(KEYSTORE).apply { load(null) }

    private fun create(): ByteArray {
        val secret = ByteArray(SECRET_BYTES)
        SecureRandom().nextBytes(secret)

        val cipher = Cipher.getInstance("AES/GCM/NoPadding")
        cipher.init(Cipher.ENCRYPT_MODE, wrappingKey())
        val sealed = cipher.doFinal(secret)

        // The IV first, then the ciphertext with its tag appended, which is
        // what javax.crypto's GCM returns.
        blob.writeBytes(cipher.iv + sealed)
        return secret
    }

    private fun unwrap(stored: ByteArray): ByteArray {
        require(stored.size > IV_BYTES) { "the wrapped session key is truncated" }
        val key = (keystore().getEntry(ALIAS, null) as? KeyStore.SecretKeyEntry)?.secretKey
            ?: throw IllegalStateException("the session key's Keystore entry is gone")
        val cipher = Cipher.getInstance("AES/GCM/NoPadding")
        cipher.init(
            Cipher.DECRYPT_MODE,
            key,
            GCMParameterSpec(TAG_BITS, stored, 0, IV_BYTES),
        )
        return cipher.doFinal(stored, IV_BYTES, stored.size - IV_BYTES)
    }

    private fun wrappingKey(): SecretKey {
        (keystore().getEntry(ALIAS, null) as? KeyStore.SecretKeyEntry)?.let { return it.secretKey }

        // StrongBox is a separate security chip and most devices have none, so
        // it is requested and not required. The fallback key is still held by
        // the Keystore and still non-exportable; it is the hardware isolation
        // that differs, not the custody.
        if (Build.VERSION.SDK_INT < Build.VERSION_CODES.P) {
            return generate(strongBox = false)
        }
        return try {
            generate(strongBox = true)
        } catch (unavailable: StrongBoxUnavailableException) {
            // A failed generation can still have claimed the alias, and a
            // second generation under a claimed alias fails rather than
            // replacing it.
            keystore().deleteEntry(ALIAS)
            generate(strongBox = false)
        }
    }

    private fun generate(strongBox: Boolean): SecretKey {
        val spec = KeyGenParameterSpec.Builder(
            ALIAS,
            KeyProperties.PURPOSE_ENCRYPT or KeyProperties.PURPOSE_DECRYPT,
        ).apply {
            setBlockModes(KeyProperties.BLOCK_MODE_GCM)
            setEncryptionPaddings(KeyProperties.ENCRYPTION_PADDING_NONE)
            setKeySize(256)
            // Deliberately no setUserAuthenticationRequired. Decision D3
            // already makes every run on a phone something the seller starts
            // on an unlocked device, so a fingerprint or PIN prompt in front
            // of every sync would cost them every time and buy little. It is
            // one line to add if that judgement changes.
            if (Build.VERSION.SDK_INT >= Build.VERSION_CODES.P) {
                setUnlockedDeviceRequired(true)
                setIsStrongBoxBacked(strongBox)
            }
        }.build()

        return KeyGenerator.getInstance(KeyProperties.KEY_ALGORITHM_AES, KEYSTORE)
            .apply { init(spec) }
            .generateKey()
    }
}
