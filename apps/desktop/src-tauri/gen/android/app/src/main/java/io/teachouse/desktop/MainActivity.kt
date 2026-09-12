package io.teachouse.desktop

import android.os.Bundle
import androidx.activity.enableEdgeToEdge
import androidx.core.view.ViewCompat
import androidx.core.view.WindowInsetsCompat

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
    // Edge to edge is kept for the top, where the console spends the status
    // bar's inset itself through `env(safe-area-inset-top)`, and given back
    // at the bottom, where it cannot: Chromium's WebView reports
    // `safe-area-inset-bottom` as zero under a three-button navigation bar
    // (only a display cutout counts as unsafe to it), so the console's own
    // tab bar drew underneath Android's buttons and neither could be pressed.
    // The bottom system-bar inset is applied as padding on the content view
    // the WebView sits in, so the page ends where the buttons begin; the
    // keyboard's inset is included so a focused field above it is not hidden.
    val content = findViewById<android.view.View>(android.R.id.content)
    ViewCompat.setOnApplyWindowInsetsListener(content) { view, insets ->
      val bars = insets.getInsets(WindowInsetsCompat.Type.systemBars())
      val ime = insets.getInsets(WindowInsetsCompat.Type.ime())
      view.setPadding(0, 0, 0, maxOf(bars.bottom, ime.bottom))
      insets
    }
  }
}
