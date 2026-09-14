package io.teachouse.desktop

import android.os.Bundle
import androidx.activity.enableEdgeToEdge
import androidx.core.view.ViewCompat
import androidx.core.view.WindowInsetsCompat
import java.util.concurrent.atomic.AtomicInteger

class MainActivity : TauriActivity() {
  // wry handles back by walking the WebView's history and finishing the
  // Activity only once it is exhausted (WryActivity.setWebView), and Tauri's
  // generated TauriActivity turns that off, which leaves every back press
  // finishing the Activity. That default strands a seller here: a marketplace
  // sign-in replaces the console inside this one WebView, so back would leave
  // the app rather than return them, and the connect would never learn they
  // abandoned it.
  override val handleBackNavigation: Boolean = true

  override fun onCreate(savedInstanceState: Bundle?) {
    enableEdgeToEdge()
    super.onCreate(savedInstanceState)
    // WebView does not consistently report system bars as CSS safe areas.
    // Reserve them natively on every edge, including while a marketplace
    // sign-in page occupies the WebView, and consume them to avoid padding twice.
    val content = findViewById<android.view.View>(android.R.id.content)
    ViewCompat.setOnApplyWindowInsetsListener(content) { view, insets ->
      val bars = insets.getInsets(
        WindowInsetsCompat.Type.systemBars() or WindowInsetsCompat.Type.displayCutout()
      )
      val ime = insets.getInsets(WindowInsetsCompat.Type.ime())
      view.setPadding(bars.left, bars.top, bars.right, maxOf(bars.bottom, ime.bottom))
      WindowInsetsCompat.CONSUMED
    }
  }

  /**
   * Whether a window of this application is on screen, counted rather than
   * asked of one Activity instance.
   *
   * The Rust coordinator gates its continuous discovery on this: while the
   * seller is looking at the app it polls for work every ten seconds, and
   * while they are not it asks the network for nothing at all.
   *
   * Counted here, in a companion object that lives as long as the process,
   * because the obvious alternative is wrong. Tauri's PluginManager keeps its
   * plugin instances and returns early from `onActivityCreate` after its first
   * initialisation (tauri 2.11.5), so a plugin built with an Activity holds
   * that Activity for ever. A configuration change this manifest does not
   * handle — fontScale, density — recreates the Activity while Tao and Wry
   * keep the Rust application alive, and the original instance reaches
   * DESTROYED. Asking that instance about its lifecycle would answer "not
   * here" for the rest of the session however visible the replacement is, and
   * continuous discovery would never resume.
   *
   * `onStart`/`onStop` rather than `onResume`/`onPause` deliberately: a
   * started-but-not-resumed activity is one in split screen or partly
   * covered, and a seller working in it is as present as one with it
   * full-screen. The count also spans a recreation correctly, because the new
   * instance starts before the old one stops.
   */
  override fun onStart() {
    super.onStart()
    onScreen.incrementAndGet()
  }

  override fun onStop() {
    super.onStop()
    onScreen.decrementAndGet()
  }

  companion object {
    private val onScreen = AtomicInteger(0)

    /**
     * Read from the session-key plugin's `foreground` command, on whatever
     * thread Tauri dispatches it on, which is why the counter is atomic.
     */
    @JvmStatic
    fun isOnScreen(): Boolean = onScreen.get() > 0
  }
}
