package dev.condr.companion

import android.content.Intent
import android.os.Build
import android.os.Bundle
import androidx.activity.ComponentActivity
import androidx.activity.compose.setContent
import androidx.activity.enableEdgeToEdge
import androidx.compose.runtime.getValue
import androidx.compose.runtime.mutableStateOf
import androidx.compose.runtime.setValue
import androidx.core.splashscreen.SplashScreen.Companion.installSplashScreen
import dev.condr.companion.demo.DemoScene
import dev.condr.companion.ui.App
import dev.condr.companion.ui.StartupFailure
import dev.condr.companion.ui.theme.CondrTheme

class MainActivity : ComponentActivity() {
  private var stores by mutableStateOf<Stores?>(null)

  /** Why the app's objects could not be built: the Device key would not keep. */
  private var failure by mutableStateOf<String?>(null)

  override fun onCreate(savedInstanceState: Bundle?) {
    installSplashScreen()
    super.onCreate(savedInstanceState)
    enableEdgeToEdge()
    runCatching { stores = stores(intent) }.onFailure { failure = it.message ?: it.toString() }
    // A recreated Activity, or one reopened from Recents, has its notification behind it.
    val fresh =
      savedInstanceState == null && intent.flags and Intent.FLAG_ACTIVITY_LAUNCHED_FROM_HISTORY == 0
    if (fresh) opened(intent)
    if (Build.VERSION.SDK_INT >= Build.VERSION_CODES.TIRAMISU) setRecentsScreenshotEnabled(false)
    setContent {
      CondrTheme {
        failure?.let { StartupFailure(it) }
        stores?.let { App(it) }
      }
    }
  }

  override fun onNewIntent(intent: Intent) {
    super.onNewIntent(intent)
    setIntent(intent)
    runCatching { stores = stores(intent) }.onFailure { failure = it.message ?: it.toString() }
    opened(intent)
  }

  /** The Pane a tapped notification names (ADR 0047), over its Device's Workspace list. */
  private fun opened(intent: Intent?) {
    val stores = stores ?: return
    val pushed = dev.condr.companion.push.Notifications.pushed(intent) ?: return
    stores.session.ref(pushed)?.let { stores.router.openNotified(it) }
  }

  override fun onResume() {
    super.onResume()
    stores?.session?.setActive(true)
  }

  override fun onPause() {
    stores?.session?.setActive(false)
    super.onPause()
  }

  /**
   * `adb shell am start -e scene agents`: a demo scene for review and the UI tests, in a debug
   * build only, so no other app can swap the real connection for sample data.
   */
  private fun stores(intent: Intent?): Stores {
    if (!BuildConfig.DEBUG) return (application as CompanionApplication).stores(null)
    val scene = DemoScene.named(intent?.getStringExtra("scene"))
    return (application as CompanionApplication).stores(scene).also {
      it.scrub = intent?.getStringExtra("scrub")
      it.focus = intent?.getStringExtra("focus") == "true"
    }
  }
}
