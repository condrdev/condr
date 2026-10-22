package dev.condr.companion.ui.terminal

import androidx.compose.ui.text.TextStyle
import androidx.compose.ui.text.font.Font
import androidx.compose.ui.text.font.FontFamily
import androidx.compose.ui.text.font.FontStyle
import androidx.compose.ui.text.font.FontVariation
import androidx.compose.ui.text.font.FontWeight
import androidx.compose.ui.unit.TextUnit
import dev.condr.companion.R

/**
 * JetBrains Mono, bundled as two variable files, its ligatures off so every glyph keeps to its cell
 * (ADR 0048). The renderer draws with the framework `Paint`; this is the same face for Compose text
 * that previews it.
 */
object TerminalFont {
  val family =
    FontFamily(
      Font(
        resId = R.font.jetbrains_mono,
        weight = FontWeight.Normal,
        style = FontStyle.Normal,
        variationSettings = FontVariation.Settings(FontVariation.weight(400)),
      ),
      Font(
        resId = R.font.jetbrains_mono,
        weight = FontWeight.Bold,
        style = FontStyle.Normal,
        variationSettings = FontVariation.Settings(FontVariation.weight(700)),
      ),
      Font(
        resId = R.font.jetbrains_mono_italic,
        weight = FontWeight.Normal,
        style = FontStyle.Italic,
        variationSettings = FontVariation.Settings(FontVariation.weight(400)),
      ),
      Font(
        resId = R.font.jetbrains_mono_italic,
        weight = FontWeight.Bold,
        style = FontStyle.Italic,
        variationSettings = FontVariation.Settings(FontVariation.weight(700)),
      ),
    )

  const val FEATURES = "calt off, liga off"

  fun style(size: TextUnit): TextStyle =
    TextStyle(fontFamily = family, fontSize = size, fontFeatureSettings = FEATURES)
}

/** The library's default scheme, GitHub Dark Default's: what the terminal and its previews use. */
object TerminalPalette {
  const val BACKGROUND = 0xFF0D1117
  const val FOREGROUND = 0xFFC9D1D9
  const val GREEN = 0xFF3FB950
  const val GRAY = 0xFF6E7681
  const val BRIGHT_WHITE = 0xFFFFFFFF
}
