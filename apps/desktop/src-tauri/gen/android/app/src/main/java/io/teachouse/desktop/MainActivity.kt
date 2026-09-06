package io.teachouse.desktop

import android.os.Bundle
import androidx.activity.enableEdgeToEdge

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
  }
}
