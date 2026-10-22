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
import dev.condr.companion.ui.theme.CondrTheme

class MainActivity : ComponentActivity() {
  private var stores by mutableStateOf<Stores?>(null)

  override fun onCreate(savedInstanceState: Bundle?) {
    installSplashScreen()
    super.onCreate(savedInstanceState)
    enableEdgeToEdge()
    stores = stores(intent)
    opened(intent)
    if (Build.VERSION.SDK_INT >= Build.VERSION_CODES.TIRAMISU) setRecentsScreenshotEnabled(false)
    setContent { CondrTheme { stores?.let { App(it) } } }
  }

  override fun onNewIntent(intent: Intent) {
    super.onNewIntent(intent)
    setIntent(intent)
    stores = stores(intent)
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

  /** `adb shell am start -e scene agents`: a demo scene for review and the UI tests. */
  private fun stores(intent: Intent?): Stores {
    val scene = DemoScene.named(intent?.getStringExtra("scene"))
    return (application as CompanionApplication).stores(scene).also {
      it.scrub = intent?.getStringExtra("scrub")
      it.focus = intent?.getStringExtra("focus") == "true"
    }
  }
}
