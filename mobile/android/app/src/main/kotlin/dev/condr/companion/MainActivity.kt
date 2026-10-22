package dev.condr.companion

import android.content.Intent
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
    stores = (application as CompanionApplication).stores(scene(intent))
    setContent { CondrTheme { stores?.let { App(it) } } }
  }

  override fun onNewIntent(intent: Intent) {
    super.onNewIntent(intent)
    setIntent(intent)
    stores = (application as CompanionApplication).stores(scene(intent))
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
  private fun scene(intent: Intent?): DemoScene? = DemoScene.named(intent?.getStringExtra("scene"))
}
