package dev.condr.companion.ui.theme

import android.os.Build
import androidx.compose.foundation.isSystemInDarkTheme
import androidx.compose.material3.ExperimentalMaterial3ExpressiveApi
import androidx.compose.material3.MaterialExpressiveTheme
import androidx.compose.material3.MaterialTheme
import androidx.compose.material3.MotionScheme
import androidx.compose.material3.darkColorScheme
import androidx.compose.material3.dynamicDarkColorScheme
import androidx.compose.material3.dynamicLightColorScheme
import androidx.compose.material3.lightColorScheme
import androidx.compose.runtime.Composable
import androidx.compose.ui.graphics.Color
import androidx.compose.ui.platform.LocalContext
import uniffi.condr_mobile.AgentStatus

// The brand's two colours (assets/brand); the neutral scheme below stands in for dynamic
// colour on Android 10 and 11 (ADR 0048).
private val Ink = Color(0xFF1D1D1F)
private val Paper = Color(0xFFF5F5F7)

private val NeutralLight =
  lightColorScheme(
    primary = Ink,
    onPrimary = Paper,
    primaryContainer = Color(0xFFE2E2E6),
    onPrimaryContainer = Ink,
    secondary = Color(0xFF5E5E62),
    onSecondary = Paper,
    secondaryContainer = Color(0xFFE5E5E9),
    onSecondaryContainer = Color(0xFF1B1B1F),
    background = Paper,
    onBackground = Ink,
    surface = Paper,
    onSurface = Ink,
    surfaceVariant = Color(0xFFE5E5E9),
    onSurfaceVariant = Color(0xFF46464A),
    outline = Color(0xFF77777B),
  )

private val NeutralDark =
  darkColorScheme(
    primary = Paper,
    onPrimary = Ink,
    primaryContainer = Color(0xFF444448),
    onPrimaryContainer = Paper,
    secondary = Color(0xFFC6C6CA),
    onSecondary = Color(0xFF2F2F33),
    secondaryContainer = Color(0xFF46464A),
    onSecondaryContainer = Color(0xFFE2E2E6),
    background = Color(0xFF131316),
    onBackground = Color(0xFFE5E2E6),
    surface = Color(0xFF131316),
    onSurface = Color(0xFFE5E2E6),
    surfaceVariant = Color(0xFF46464A),
    onSurfaceVariant = Color(0xFFC7C6CA),
    outline = Color(0xFF909094),
  )

/**
 * The app's theme: Material 3 Expressive, in the system's dynamic colour where Android offers it
 * (ADR 0048).
 */
// Expressive's theme entry point is the one API still behind the Expressive opt-in that
// every screen needs; it is used here and nowhere else (ADR 0048).
@OptIn(ExperimentalMaterial3ExpressiveApi::class)
@Composable
fun CondrTheme(content: @Composable () -> Unit) {
  val dark = isSystemInDarkTheme()
  val context = LocalContext.current
  val scheme =
    when {
      Build.VERSION.SDK_INT >= Build.VERSION_CODES.S ->
        if (dark) dynamicDarkColorScheme(context) else dynamicLightColorScheme(context)
      dark -> NeutralDark
      else -> NeutralLight
    }
  MaterialExpressiveTheme(
    colorScheme = scheme,
    motionScheme = MotionScheme.expressive(),
    content = content,
  )
}

/** The terminal's chrome is always dark, over the library's dark scheme (ADR 0048). */
@OptIn(ExperimentalMaterial3ExpressiveApi::class)
@Composable
fun TerminalTheme(content: @Composable () -> Unit) {
  val context = LocalContext.current
  val scheme =
    if (Build.VERSION.SDK_INT >= Build.VERSION_CODES.S) dynamicDarkColorScheme(context)
    else NeutralDark
  MaterialExpressiveTheme(
    colorScheme = scheme,
    motionScheme = MotionScheme.expressive(),
    content = content,
  )
}

/**
 * The desktop sidebar's tones for an Agent's state, as fixed semantic colours: they do not follow
 * the wallpaper (ADR 0048).
 */
val AgentStatus.tint: Color
  @Composable
  get() =
    when (this) {
      AgentStatus.WORKING -> Color(0xFFF59E0B)
      AgentStatus.BLOCKED,
      AgentStatus.ERROR -> MaterialTheme.colorScheme.error
      AgentStatus.DONE -> Color(0xFF22A06B)
      AgentStatus.IDLE,
      AgentStatus.UNKNOWN -> MaterialTheme.colorScheme.onSurfaceVariant
    }

@Composable fun tintOf(status: AgentStatus): Color = status.tint

/**
 * While a page that is dark whatever the theme shows, the status and navigation bars take light
 * icons and, where Android still draws a bar of its own (three-button navigation before 15), a dark
 * one; both go back as the page leaves.
 */
@Composable
fun DarkSystemBars() {
  val view = androidx.compose.ui.platform.LocalView.current
  androidx.compose.runtime.DisposableEffect(view) {
    val window = (view.context as? android.app.Activity)?.window
    val controller = window?.let { androidx.core.view.WindowCompat.getInsetsController(it, view) }
    val wasLightStatus = controller?.isAppearanceLightStatusBars
    val wasLightNavigation = controller?.isAppearanceLightNavigationBars
    @Suppress("DEPRECATION") val wasNavigationColor = window?.navigationBarColor
    controller?.isAppearanceLightStatusBars = false
    controller?.isAppearanceLightNavigationBars = false
    @Suppress("DEPRECATION")
    if (window != null && Build.VERSION.SDK_INT < Build.VERSION_CODES.VANILLA_ICE_CREAM)
      window.navigationBarColor = 0xFF1D1D1F.toInt()
    onDispose {
      if (wasLightStatus != null) controller.isAppearanceLightStatusBars = wasLightStatus
      if (wasLightNavigation != null)
        controller.isAppearanceLightNavigationBars = wasLightNavigation
      @Suppress("DEPRECATION")
      if (
        window != null &&
          wasNavigationColor != null &&
          Build.VERSION.SDK_INT < Build.VERSION_CODES.VANILLA_ICE_CREAM
      )
        window.navigationBarColor = wasNavigationColor
    }
  }
}
