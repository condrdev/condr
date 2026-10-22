package dev.condr.companion.ui.pairing

import androidx.compose.animation.AnimatedVisibility
import androidx.compose.animation.core.animateFloatAsState
import androidx.compose.animation.core.spring
import androidx.compose.animation.core.withInfiniteAnimationFrameMillis
import androidx.compose.animation.fadeIn
import androidx.compose.animation.fadeOut
import androidx.compose.animation.scaleIn
import androidx.compose.animation.scaleOut
import androidx.compose.foundation.Canvas
import androidx.compose.foundation.layout.Arrangement
import androidx.compose.foundation.layout.Box
import androidx.compose.foundation.layout.BoxWithConstraints
import androidx.compose.foundation.layout.Row
import androidx.compose.foundation.layout.fillMaxSize
import androidx.compose.foundation.layout.offset
import androidx.compose.foundation.layout.padding
import androidx.compose.foundation.layout.size
import androidx.compose.material3.Icon
import androidx.compose.material3.MaterialTheme
import androidx.compose.material3.Surface
import androidx.compose.material3.Text
import androidx.compose.runtime.Composable
import androidx.compose.runtime.LaunchedEffect
import androidx.compose.runtime.getValue
import androidx.compose.runtime.mutableFloatStateOf
import androidx.compose.runtime.mutableStateOf
import androidx.compose.runtime.remember
import androidx.compose.runtime.setValue
import androidx.compose.ui.Alignment
import androidx.compose.ui.Modifier
import androidx.compose.ui.geometry.CornerRadius
import androidx.compose.ui.geometry.Offset
import androidx.compose.ui.geometry.Size
import androidx.compose.ui.graphics.Color
import androidx.compose.ui.graphics.drawscope.drawIntoCanvas
import androidx.compose.ui.graphics.nativeCanvas
import androidx.compose.ui.platform.LocalDensity
import androidx.compose.ui.res.painterResource
import androidx.compose.ui.text.font.FontWeight
import androidx.compose.ui.unit.IntOffset
import androidx.compose.ui.unit.dp
import dev.condr.companion.ui.agents.AgentMarks
import dev.condr.companion.ui.theme.barHeight
import dev.condr.companion.ui.theme.tint
import kotlin.math.roundToInt
import kotlin.math.sin
import kotlinx.coroutines.delay
import uniffi.condr_mobile.AgentStatus

private class Story(
  val column: Int,
  val bar: Int,
  val status: AgentStatus,
  val kind: String,
  val words: String,
)

private val stories =
  listOf(
    Story(0, 2, AgentStatus.BLOCKED, "claude", "Claude needs you"),
    Story(1, 4, AgentStatus.DONE, "codex", "Codex is done"),
    Story(2, 1, AgentStatus.BLOCKED, "claude", "Claude asks a question"),
    Story(1, 0, AgentStatus.DONE, "opencode", "OpenCode is done"),
  )
private val computers = listOf("macmini", "buildbox", "thinkbook")
/** Each bar's own state when no story uses it. */
private val base =
  listOf(
    listOf(
      AgentStatus.WORKING,
      AgentStatus.IDLE,
      AgentStatus.WORKING,
      AgentStatus.IDLE,
      AgentStatus.WORKING,
      AgentStatus.IDLE,
    ),
    listOf(
      AgentStatus.WORKING,
      AgentStatus.IDLE,
      AgentStatus.WORKING,
      AgentStatus.WORKING,
      AgentStatus.IDLE,
      AgentStatus.WORKING,
      AgentStatus.IDLE,
    ),
    listOf(
      AgentStatus.IDLE,
      AgentStatus.WORKING,
      AgentStatus.IDLE,
      AgentStatus.WORKING,
      AgentStatus.IDLE,
    ),
  )

/**
 * Three computers' Agents as the first screen's score draws them, a column each, idling and
 * working, while one after another rises to need you or settles as done, its mark and words
 * floating over it: the home screen's language, taught before it is seen.
 */
@Composable
fun WelcomeStage(playing: Boolean, modifier: Modifier = Modifier) {
  val paper = Color(0xFFF5F5F7)
  var cue by remember { mutableStateOf<Int?>(null) }
  var time by remember { mutableFloatStateOf(0f) }
  LaunchedEffect(playing) {
    if (!playing) return@LaunchedEffect
    while (true) withInfiniteAnimationFrameMillis { time = it / 1000f }
  }
  LaunchedEffect(playing) {
    if (!playing) return@LaunchedEffect
    var next = 0
    while (true) {
      delay(700)
      cue = next
      delay(2800)
      cue = null
      next = (next + 1) % stories.size
    }
  }
  val story = cue?.let { stories[it] }
  /** The story's bar rises and falls with a spring, the rest breathe. */
  val rise by
    animateFloatAsState(
      if (story != null) 1f else 0f,
      spring(dampingRatio = 0.75f, stiffness = 180f),
      label = "rise",
    )
  val tints = remember { mutableMapOf<AgentStatus, Color>() }
  for (status in AgentStatus.entries) tints[status] = status.tint
  val density = LocalDensity.current
  BoxWithConstraints(modifier.fillMaxSize()) {
    val margin = with(density) { 26.dp.toPx() }
    val gap = with(density) { 30.dp.toPx() }
    val spacing = with(density) { 7.dp.toPx() }
    val count = base.sumOf { it.size }
    val totalWidth = with(density) { maxWidth.toPx() }
    val totalHeight = with(density) { maxHeight.toPx() }
    val width =
      (totalWidth - margin * 2 - gap * (base.size - 1) - spacing * (count - base.size)) / count
    val baseline = totalHeight - with(density) { 30.dp.toPx() }
    val tall = baseline - with(density) { 58.dp.toPx() }
    fun origin(column: Int): Float {
      val before = base.take(column).sumOf { it.size }
      return margin + before * width + (before - column) * spacing + column * gap
    }
    val labelPaint =
      remember(density) {
        android.graphics.Paint(android.graphics.Paint.ANTI_ALIAS_FLAG).apply {
          color =
            paper.copy(alpha = 0.42f).let {
              android.graphics.Color.argb((it.alpha * 255).toInt(), 245, 245, 247)
            }
          textSize = with(density) { 12.sp.toPx() }
          typeface =
            android.graphics.Typeface.create(
              android.graphics.Typeface.DEFAULT,
              android.graphics.Typeface.BOLD,
            )
        }
      }
    Canvas(Modifier.fillMaxSize()) {
      val radius = minOf(4.dp.toPx(), width * 0.28f)
      var index = 0
      base.forEachIndexed { column, bars ->
        bars.forEachIndexed { bar, resting ->
          val storied = story != null && story.column == column && story.bar == bar
          val wave = sin(time * 2.4f - index * 0.32f)
          val restHeight = barHeight(resting, tall, wave)
          val height =
            if (storied) restHeight + (barHeight(story!!.status, tall, 0f) - restHeight) * rise
            else restHeight
          val status = if (storied && rise > 0.5f) story!!.status else resting
          val color =
            if (status == AgentStatus.IDLE) paper.copy(alpha = 0.14f) else tints.getValue(status)
          val x = origin(column) + bar * (width + spacing)
          drawRoundRect(
            color,
            Offset(x, baseline - height),
            Size(width, height),
            CornerRadius(radius),
          )
          index++
        }
        drawIntoCanvas {
          it.nativeCanvas.drawText(
            computers[column],
            origin(column),
            baseline + 17.dp.toPx() + 4.dp.toPx(),
            labelPaint,
          )
        }
      }
    }
    // The story's caption, level with its bar's top and kept on screen.
    AnimatedVisibility(
      visible = story != null,
      enter = fadeIn() + scaleIn(initialScale = 0.6f),
      exit = fadeOut() + scaleOut(targetScale = 0.6f),
    ) {
      val shown = remember { mutableStateOf(story) }
      if (story != null) shown.value = story
      val s = shown.value ?: return@AnimatedVisibility
      val h = barHeight(s.status, tall, 0f)
      val x =
        (origin(s.column) + s.bar * (width + spacing) + width / 2).coerceIn(
          with(density) { 104.dp.toPx() },
          totalWidth - with(density) { 104.dp.toPx() },
        )
      val y = baseline - h - with(density) { 28.dp.toPx() }
      Box(Modifier.fillMaxSize()) {
        Surface(
          shape = MaterialTheme.shapes.extraLarge,
          color = Color(0xFF2A2A2E),
          contentColor = paper,
          modifier =
            Modifier.offset { IntOffset(0, (y - with(density) { 18.dp.toPx() }).roundToInt()) }
              .padding(start = 0.dp)
              .align(Alignment.TopStart)
              .offset { IntOffset((x - with(density) { 104.dp.toPx() }).roundToInt(), 0) }
              .size(width = 208.dp, height = 36.dp),
        ) {
          Row(
            verticalAlignment = Alignment.CenterVertically,
            horizontalArrangement = Arrangement.spacedBy(8.dp),
            modifier = Modifier.padding(horizontal = 13.dp),
          ) {
            AgentMarks.resource(s.kind)?.let {
              Icon(painterResource(it), contentDescription = null, modifier = Modifier.size(15.dp))
            }
            Text(
              s.words,
              style = MaterialTheme.typography.labelLarge,
              fontWeight = FontWeight.SemiBold,
              maxLines = 1,
            )
          }
        }
      }
    }
  }
}

private val Int.sp: androidx.compose.ui.unit.TextUnit
  get() =
    androidx.compose.ui.unit.TextUnit(this.toFloat(), androidx.compose.ui.unit.TextUnitType.Sp)
