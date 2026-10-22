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

/** The three bars of `assets/brand/condr.svg`, in its 128-unit square. */
@Composable
fun Logo(side: Dp, color: Color, modifier: Modifier = Modifier) {
  Canvas(modifier.size(side)) {
    val s = size.minDimension / 128f
    for ((x, y, w, h) in
      listOf(
        listOf(48f, 24f, 42f, 12f),
        listOf(35.5f, 36f, 12f, 56f),
        listOf(48f, 92f, 42f, 12f),
      )) {
      drawRoundRect(color, Offset(x * s, y * s), Size(w * s, h * s), CornerRadius(2 * s))
    }
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
