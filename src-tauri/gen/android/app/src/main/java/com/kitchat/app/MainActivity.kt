package com.kitchat.app

import android.content.Intent
import android.os.Bundle
import android.webkit.WebView
import androidx.activity.OnBackPressedCallback
import androidx.activity.enableEdgeToEdge

class MainActivity : TauriActivity() {
  // Back is the app's, not the WebView history's (see onWebViewCreate).
  override val handleBackNavigation: Boolean = false

  override fun onCreate(savedInstanceState: Bundle?) {
    enableEdgeToEdge()
    super.onCreate(savedInstanceState)
    // Opened from another app's Share button.
    if (savedInstanceState == null) KitPlugin.received(intent)
  }

  // Shared to while already open (the Activity is singleTask).
  override fun onNewIntent(intent: Intent) {
    super.onNewIntent(intent)
    setIntent(intent)
    if (KitPlugin.received(intent)) KitPlugin.instance?.announceShared()
  }

  // Android's Back steps back one level in the app (a picture, a sheet, a chat; src/lib/back.ts), and only leaves
  // when there's nothing left to close.
  override fun onWebViewCreate(webView: WebView) {
    onBackPressedDispatcher.addCallback(this, object : OnBackPressedCallback(true) {
      override fun handleOnBackPressed() {
        webView.evaluateJavascript("(window.__kitBack ? window.__kitBack() : false)") { handled ->
          if (handled != "true") {
            isEnabled = false
            onBackPressedDispatcher.onBackPressed()
            isEnabled = true
          }
        }
      }
    })
  }
}
