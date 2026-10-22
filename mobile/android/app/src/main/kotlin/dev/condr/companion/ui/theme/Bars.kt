package dev.condr.companion.ui.theme

import androidx.compose.animation.core.withInfiniteAnimationFrameMillis
import androidx.compose.foundation.Canvas
import androidx.compose.foundation.background
import androidx.compose.foundation.layout.Box
import androidx.compose.foundation.layout.size
import androidx.compose.foundation.shape.RoundedCornerShape
import androidx.compose.material3.MaterialTheme
import androidx.compose.material3.Text
import androidx.compose.runtime.Composable
import androidx.compose.runtime.LaunchedEffect
import androidx.compose.runtime.getValue
import androidx.compose.runtime.mutableFloatStateOf
import androidx.compose.runtime.remember
import androidx.compose.runtime.setValue
import androidx.compose.ui.Alignment
import androidx.compose.ui.Modifier
import androidx.compose.ui.geometry.CornerRadius
import androidx.compose.ui.geometry.Offset
import androidx.compose.ui.geometry.Size
import androidx.compose.ui.graphics.drawscope.Stroke
import androidx.compose.ui.text.AnnotatedString
import androidx.compose.ui.text.SpanStyle
import androidx.compose.ui.text.buildAnnotatedString
import androidx.compose.ui.text.font.FontWeight
import androidx.compose.ui.text.withStyle
import androidx.compose.ui.unit.Dp
import androidx.compose.ui.unit.dp
import dev.condr.companion.model.title
import kotlin.math.sin
import uniffi.condr_mobile.AgentStatus

/** The bar width and gap that fit `count` bars in `width`, no wider than `widest`. */
// ponytail: past 2 dp bars they overflow; draw one bar per state when a Device runs that many.
fun fitBars(count: Int, width: Float, widest: Float, spacing: Float): Pair<Float, Float> {
  if (count <= 0) return widest to spacing
  val natural = (width - spacing * (count - 1)) / count
  if (natural >= 4f) return minOf(widest, natural) to spacing
  return maxOf(2f, (width - 2f * (count - 1)) / count) to 2f
}

/** The band each state's bar stands in, `wave` in -1..1 breathing a working one. */
fun barHeight(status: AgentStatus, tall: Float, wave: Float): Float =
  when (status) {
    AgentStatus.BLOCKED -> tall
    AgentStatus.ERROR -> tall * 0.9f
    AgentStatus.DONE -> tall * 0.75f
    AgentStatus.WORKING -> tall * (0.35f + 0.10f * wave)
    AgentStatus.IDLE,
    AgentStatus.UNKNOWN -> tall * 0.12f
  }

/**
 * Agents as the logo's bars, a height band per state so shape tells them apart as well as colour: a
 * blocked one full height, a finished one three quarters, a working one breathing between a quarter
 * and a half, a resting one low and grey. Every bar keeps the same clock and `first` places this
 * run in a longer row, so working bars rise in one wave travelling left to right across every
 * computer's column.
 */
@Composable
fun Bars(
  statuses: List<AgentStatus>,
  tall: Dp,
  widest: Dp,
  spacing: Dp,
  modifier: Modifier = Modifier,
  animated: Boolean = true,
  first: Int = 0,
  /** While someone scrubs the score: the bar under their finger here (or -1 for none of these). */
  highlight: Int? = null,
  /** How far the header has folded, 1 open to 0.3 folded: heights shrink, corners stay round. */
  fold: Float = 1f,
) {
  val breathing = animated && statuses.contains(AgentStatus.WORKING)
  var time by remember { mutableFloatStateOf(0f) }
  LaunchedEffect(breathing) {
    if (!breathing) return@LaunchedEffect
    while (true) withInfiniteAnimationFrameMillis { time = it / 1000f }
  }
  val tints = statuses.map { it.tint }
  val rest = MaterialTheme.colorScheme.outlineVariant
  val mark = MaterialTheme.colorScheme.onSurface
  val width = widest * statuses.size + spacing * (statuses.size - 1).coerceAtLeast(0)
  Canvas(modifier.size(width = width, height = tall)) {
    val w = widest.toPx()
    val gap = spacing.toPx()
    val h = tall.toPx()
    val radius = minOf(4.dp.toPx(), w * 0.26f)
    statuses.forEachIndexed { index, status ->
      val wave = if (animated) sin((time - (first + index) * 0.32f) * 2.4f) else 0f
      val resting = status == AgentStatus.IDLE || status == AgentStatus.UNKNOWN
      val height =
        if (resting) minOf(h * 0.2f, maxOf(3.dp.toPx(), h * 0.12f, w * 2))
        else maxOf(w * 0.8f, barHeight(status, h, wave) * fold)
      val dim = highlight != null && highlight != index
      val color =
        (if (resting) rest else tints[index]).let { if (dim) it.copy(alpha = 0.28f) else it }
      val x = index * (w + gap)
      drawRoundRect(color, Offset(x, h - height), Size(w, height), CornerRadius(radius))
      if (highlight == index) {
        val under = maxOf(8.dp.toPx(), w + 4.dp.toPx())
        drawRoundRect(
          mark,
          Offset(x + w / 2 - under / 2, h + 7.dp.toPx()),
          Size(under, 3.dp.toPx()),
          CornerRadius(1.5.dp.toPx()),
        )
      }
    }
  }
}

/** The score's placeholder for a computer it can't reach: three bars fallen flat. */
@Composable
fun Fallen(width: Dp, modifier: Modifier = Modifier) {
  val color = MaterialTheme.colorScheme.outlineVariant
  Canvas(modifier.size(width = width, height = maxOf(3.dp, width * 0.05f))) {
    val gap = maxOf(2.dp.toPx(), size.width * 0.06f)
    val lengths = listOf(1f, 0.72f, 0.44f)
    val unit = (size.width - gap * 2) / lengths.sum()
    var x = 0f
    for (length in lengths) {
      drawRoundRect(
        color,
        Offset(x, 0f),
        Size(unit * length, size.height),
        CornerRadius(1.5.dp.toPx()),
      )
      x += unit * length + gap
    }
  }
}

/** An Agent's state as a dot: filled in its colour, a ring when idle, grey when unknown. */
@Composable
fun StatusGlyph(status: AgentStatus, size: Dp = 9.dp, modifier: Modifier = Modifier) {
  val tint = status.tint
  Canvas(modifier.size(size)) {
    when (status) {
      AgentStatus.IDLE ->
        drawCircle(tint, style = Stroke(maxOf(1.2.dp.toPx(), this.size.minDimension / 6)))
      else -> drawCircle(tint)
    }
  }
}

/** A Workspace's initial on a small rounded square. */
@Composable
fun WorkspaceAvatar(name: String, side: Dp = 22.dp) {
  Box(
    Modifier.size(side)
      .background(MaterialTheme.colorScheme.secondaryContainer, RoundedCornerShape(side * 0.26f)),
    contentAlignment = Alignment.Center,
  ) {
    Text(
      name.take(1).uppercase(),
      style = MaterialTheme.typography.labelMedium,
      fontWeight = FontWeight.SemiBold,
      color = MaterialTheme.colorScheme.onSecondaryContainer,
    )
  }
}

/** "1 needs you · 9 working · 1 done", the need in red; idle only when nothing else is. */
@Composable
fun agentSummary(statuses: List<AgentStatus>): AnnotatedString {
  val count = { status: AgentStatus -> statuses.count { it == status } }
  val blocked = count(AgentStatus.BLOCKED)
  val rest =
    listOf(
        count(AgentStatus.ERROR) to "failed",
        count(AgentStatus.WORKING) to "working",
        count(AgentStatus.DONE) to "done",
      )
      .filter { it.first > 0 }
      .joinToString(" · ") { "${it.first} ${it.second}" }
  val need = AgentStatus.BLOCKED.tint
  return buildAnnotatedString {
    if (blocked > 0) {
      withStyle(SpanStyle(color = need, fontWeight = FontWeight.SemiBold)) {
        append("$blocked ${if (blocked == 1) "needs" else "need"} you")
      }
      if (rest.isNotEmpty()) append(" · $rest")
    } else if (rest.isNotEmpty()) {
      append(rest)
    } else {
      append(if (statuses.isEmpty()) "No agents" else "${statuses.size} idle")
    }
  }
}

/** An Agent's state in words; a wait or a failure also in red, with what it waits for or why. */
@Composable
fun agentStatusLine(
  label: String,
  status: AgentStatus,
  detail: String?,
  kind: Boolean = false,
): AnnotatedString = buildAnnotatedString {
  if (kind) append("$label · ")
  if (status == AgentStatus.BLOCKED || status == AgentStatus.ERROR) {
    withStyle(SpanStyle(color = status.tint, fontWeight = FontWeight.SemiBold)) {
      append(status.title)
    }
    if (detail != null) append(" · $detail")
  } else {
    append(status.title)
  }
}
