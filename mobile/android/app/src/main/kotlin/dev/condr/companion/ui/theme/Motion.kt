package dev.condr.companion.ui.theme

import android.provider.Settings
import androidx.compose.runtime.Composable
import androidx.compose.runtime.remember
import androidx.compose.ui.platform.LocalContext

/**
 * Whether the person turned animations off (Accessibility › Remove animations), Android's
 * counterpart of iOS's Reduce Motion: decorative motion stops, state changes still show.
 */
@Composable
fun reducedMotion(): Boolean {
  val resolver = LocalContext.current.contentResolver
  return remember(resolver) {
    Settings.Global.getFloat(resolver, Settings.Global.ANIMATOR_DURATION_SCALE, 1f) == 0f
  }
}
