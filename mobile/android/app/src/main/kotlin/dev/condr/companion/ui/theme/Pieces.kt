package dev.condr.companion.ui.theme

import androidx.compose.foundation.Canvas
import androidx.compose.foundation.background
import androidx.compose.foundation.layout.Arrangement
import androidx.compose.foundation.layout.Box
import androidx.compose.foundation.layout.Column
import androidx.compose.foundation.layout.Row
import androidx.compose.foundation.layout.fillMaxWidth
import androidx.compose.foundation.layout.size
import androidx.compose.foundation.shape.CircleShape
import androidx.compose.foundation.shape.RoundedCornerShape
import androidx.compose.material3.MaterialTheme
import androidx.compose.material3.Text
import androidx.compose.runtime.Composable
import androidx.compose.ui.Alignment
import androidx.compose.ui.Modifier
import androidx.compose.ui.geometry.CornerRadius
import androidx.compose.ui.geometry.Offset
import androidx.compose.ui.geometry.Size
import androidx.compose.ui.graphics.Color
import androidx.compose.ui.text.AnnotatedString
import androidx.compose.ui.text.font.FontWeight
import androidx.compose.ui.unit.Dp
import androidx.compose.ui.unit.dp

/**
 * The three bars of `assets/brand/condr.svg`, in its 128-unit square. An `assembly` below 1 gathers
 * them to the spine, the top and bottom bars crossed over, so the mark can build itself on first
 * launch.
 */
@Composable
fun Logo(side: Dp, color: Color, modifier: Modifier = Modifier, assembly: Float = 1f) {
  Canvas(modifier.size(side)) {
    val s = size.minDimension / 128f
    val apart = (1 - assembly) * 20.dp.toPx()
    fun bar(x: Float, y: Float, w: Float, h: Float) =
      drawRoundRect(color, Offset(x, y), Size(w, h), CornerRadius(2 * s))
    // The spine grows from its middle; the arms grow out of it as they slide to their ends.
    val spine = 56 * s * (0.2f + 0.8f * assembly)
    bar(35.5f * s, 64 * s - spine / 2, 12 * s, spine)
    val reach = 42 * s * assembly
    bar(47.36f * s + 0.64f * s * assembly, 24 * s + apart, reach, 12 * s)
    bar(47.36f * s + 0.64f * s * assembly, 92 * s - apart, reach, 12 * s)
  }
}

/** A step's heading and, under it, what it asks. */
@Composable
fun StepTitle(title: String, detail: String? = null) {
  Column(verticalArrangement = Arrangement.spacedBy(8.dp)) {
    Text(title, style = MaterialTheme.typography.headlineMedium, fontWeight = FontWeight.Bold)
    if (detail != null) {
      Text(
        detail,
        style = MaterialTheme.typography.bodyLarge,
        color = MaterialTheme.colorScheme.onSurfaceVariant,
      )
    }
  }
}

/** "1 Open Condr on your computer." */
@Composable
fun NumberedStep(number: Int, text: AnnotatedString) {
  Row(
    horizontalArrangement = Arrangement.spacedBy(12.dp),
    verticalAlignment = Alignment.Top,
    modifier = Modifier.fillMaxWidth(),
  ) {
    Box(
      Modifier.size(24.dp)
        .background(MaterialTheme.colorScheme.surfaceContainerHighest, CircleShape),
      contentAlignment = Alignment.Center,
    ) {
      Text("$number", style = MaterialTheme.typography.labelMedium, fontWeight = FontWeight.Bold)
    }
    Text(text, style = MaterialTheme.typography.bodyLarge, modifier = Modifier.weight(1f))
  }
}

@Composable
fun NumberedStep(number: Int, text: String) = NumberedStep(number, AnnotatedString(text))

/** The app's icon: the mark in paper on an ink tile, wherever the app stands for itself. */
@Composable
fun AppMark(side: Dp, modifier: Modifier = Modifier) {
  Box(
    modifier.size(side).background(Color(0xFF1D1D1F), RoundedCornerShape(side * 0.225f)),
    contentAlignment = Alignment.Center,
  ) {
    Logo(side, Color(0xFFF5F5F7))
  }
}
