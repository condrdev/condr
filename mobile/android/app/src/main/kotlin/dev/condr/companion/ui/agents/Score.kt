package dev.condr.companion.ui.agents

import androidx.compose.animation.AnimatedVisibility
import androidx.compose.animation.core.animateDpAsState
import androidx.compose.animation.core.animateFloatAsState
import androidx.compose.animation.core.tween
import androidx.compose.animation.fadeIn
import androidx.compose.animation.fadeOut
import androidx.compose.animation.scaleIn
import androidx.compose.animation.scaleOut
import androidx.compose.foundation.clickable
import androidx.compose.foundation.gestures.detectDragGesturesAfterLongPress
import androidx.compose.foundation.horizontalScroll
import androidx.compose.foundation.layout.Arrangement
import androidx.compose.foundation.layout.Box
import androidx.compose.foundation.layout.BoxWithConstraints
import androidx.compose.foundation.layout.Column
import androidx.compose.foundation.layout.Row
import androidx.compose.foundation.layout.fillMaxWidth
import androidx.compose.foundation.layout.height
import androidx.compose.foundation.layout.padding
import androidx.compose.foundation.layout.size
import androidx.compose.foundation.layout.widthIn
import androidx.compose.foundation.rememberScrollState
import androidx.compose.material3.Icon
import androidx.compose.material3.MaterialTheme
import androidx.compose.material3.Surface
import androidx.compose.material3.Text
import androidx.compose.runtime.Composable
import androidx.compose.runtime.getValue
import androidx.compose.runtime.mutableStateMapOf
import androidx.compose.runtime.mutableStateOf
import androidx.compose.runtime.remember
import androidx.compose.runtime.setValue
import androidx.compose.ui.Alignment
import androidx.compose.ui.Modifier
import androidx.compose.ui.draw.alpha
import androidx.compose.ui.draw.drawWithContent
import androidx.compose.ui.geometry.Offset
import androidx.compose.ui.geometry.Rect
import androidx.compose.ui.graphics.BlendMode
import androidx.compose.ui.graphics.Brush
import androidx.compose.ui.graphics.Color
import androidx.compose.ui.graphics.CompositingStrategy
import androidx.compose.ui.graphics.graphicsLayer
import androidx.compose.ui.hapticfeedback.HapticFeedbackType
import androidx.compose.ui.input.pointer.pointerInput
import androidx.compose.ui.layout.boundsInParent
import androidx.compose.ui.layout.layout
import androidx.compose.ui.layout.onGloballyPositioned
import androidx.compose.ui.platform.LocalDensity
import androidx.compose.ui.platform.LocalHapticFeedback
import androidx.compose.ui.res.painterResource
import androidx.compose.ui.semantics.clearAndSetSemantics
import androidx.compose.ui.semantics.contentDescription
import androidx.compose.ui.semantics.heading
import androidx.compose.ui.semantics.semantics
import androidx.compose.ui.text.font.FontWeight
import androidx.compose.ui.unit.Constraints
import androidx.compose.ui.unit.Dp
import androidx.compose.ui.unit.dp
import dev.condr.companion.R
import dev.condr.companion.model.DeviceRoute
import dev.condr.companion.model.PaneRef
import dev.condr.companion.model.byUrgency
import dev.condr.companion.model.connected
import dev.condr.companion.model.name
import dev.condr.companion.model.title
import dev.condr.companion.ui.LocalStores
import dev.condr.companion.ui.theme.Bars
import dev.condr.companion.ui.theme.Fallen
import dev.condr.companion.ui.theme.StatusGlyph
import dev.condr.companion.ui.theme.fitBars
import kotlin.math.roundToInt
import uniffi.condr_mobile.AgentStatus
import uniffi.condr_mobile.DeviceStatus
import uniffi.condr_mobile.DeviceView

/**
 * The first screen's head: each computer as a column of the logo's bars, one per Agent, most urgent
 * first, so a colour reads as a run however many there are. A tap on a column opens its computer.
 * Press, hold and slide to scrub the bars: each Agent passed under the finger ticks and names
 * itself, and letting go opens its terminal.
 */
@Composable
fun Score(fold: Float, modifier: Modifier = Modifier) {
  val stores = LocalStores.current
  val session = stores.session
  val router = stores.router
  val haptics = LocalHapticFeedback.current
  val density = LocalDensity.current
  val devices = session.devices
  val tall = if (devices.size <= 3) 88.dp else 56.dp
  val columns = remember { mutableStateMapOf<String, Rect>() }
  var touch by remember { mutableStateOf<Offset?>(null) }
  var scrub by remember { mutableStateOf<PaneRef?>(null) }
  var staged by remember { mutableStateOf(stagedScrub(stores.scrub, session)) }
  val active = touch != null || staged != null
  val shown = scrub ?: staged
  val scroll = rememberScrollState()
  val inset = 20.dp

  fun fitting(count: Int) =
    with(density) { fitBars(count, 160.dp.toPx(), 18.dp.toPx(), 6.dp.toPx()) }

  /** The Agent whose bar is nearest the finger, in whichever column it is over. */
  fun pane(at: Offset): PaneRef? {
    for (device in devices) {
      if (!device.connected) continue
      val frame = columns[device.address] ?: continue
      val slack = with(density) { 12.dp.toPx() }
      val above = with(density) { 44.dp.toPx() }
      val below = with(density) { 64.dp.toPx() }
      if (at.x < frame.left - slack || at.x > frame.right + slack) continue
      if (at.y < frame.top - above || at.y > frame.bottom + below) continue
      val items = session.agents(device).byUrgency()
      if (items.isEmpty()) return null
      val (w, gap) = fitting(items.size)
      val index = ((at.x - frame.left) / (w + gap)).toInt().coerceIn(0, items.lastIndex)
      return items[index].ref
    }
    return null
  }

  /** Where the scrubbed bar's centre is across the score, as it shows on screen. */
  fun screenX(ref: PaneRef): Float? {
    val device = session.device(ref.address) ?: return null
    val frame = columns[ref.address] ?: return null
    val items = session.agents(device).byUrgency()
    val index = items.indexOfFirst { it.ref == ref }.takeIf { it >= 0 } ?: return null
    val (w, gap) = fitting(items.size)
    return with(density) { inset.toPx() } + frame.left + index * (w + gap) + w / 2 - scroll.value
  }

  // Columns past either edge fade out, so a computer off screen reads as more to swipe to.
  val fadeStart by animateDpAsState(if (scroll.value > 1) 36.dp else 0.dp, label = "start")
  val fadeEnd by
    animateDpAsState(if (scroll.value < scroll.maxValue - 1) 56.dp else 0.dp, label = "end")
  Box(
    modifier.edgeToEdge(16.dp).fillMaxWidth().semantics {
      contentDescription = "Agents"
      heading()
    }
  ) {
    Row(
      horizontalArrangement = Arrangement.spacedBy(24.dp),
      verticalAlignment = Alignment.Bottom,
      modifier =
        Modifier.graphicsLayer(compositingStrategy = CompositingStrategy.Offscreen)
          .drawWithContent {
            drawContent()
            val start = fadeStart.toPx()
            val end = fadeEnd.toPx()
            if (start > 0f)
              drawRect(
                Brush.horizontalGradient(0f to Color.Transparent, 1f to Color.Black, endX = start),
                size = size.copy(width = start),
                blendMode = BlendMode.DstIn,
              )
            if (end > 0f)
              drawRect(
                Brush.horizontalGradient(
                  0f to Color.Black,
                  1f to Color.Transparent,
                  startX = size.width - end,
                  endX = size.width,
                ),
                topLeft = Offset(size.width - end, 0f),
                size = size.copy(width = end),
                blendMode = BlendMode.DstIn,
              )
          }
          .horizontalScroll(scroll, enabled = !active)
          .padding(horizontal = inset)
          .pointerInput(devices) {
            detectDragGesturesAfterLongPress(
              onDragStart = { offset ->
                staged = null
                touch = offset
                scrub = pane(offset)
                haptics.performHapticFeedback(HapticFeedbackType.LongPress)
              },
              onDrag = { change, _ ->
                touch = change.position
                val next = pane(change.position)
                // Every change ticks, leaving the bars too.
                if (next != scrub) {
                  scrub = next
                  haptics.performHapticFeedback(HapticFeedbackType.SegmentTick)
                }
              },
              onDragEnd = {
                scrub?.let { router.open(it) }
                touch = null
                scrub = null
              },
              onDragCancel = {
                touch = null
                scrub = null
              },
            )
          },
    ) {
      var first = 0
      for (device in devices) {
        val items = session.agents(device).byUrgency()
        val statuses = items.map { it.agent.status }
        val (w, gap) = fitting(statuses.size)
        val highlight =
          when {
            !active -> null
            shown?.address == device.address -> items.indexOfFirst { it.ref == shown }
            else -> -1
          }
        val connected = device.connected
        Column(
          verticalArrangement = Arrangement.spacedBy(10.dp),
          modifier =
            Modifier.widthIn(min = 44.dp)
              .clickable(enabled = !active, onClickLabel = "Opens the computer") {
                router.agentsPath.add(DeviceRoute.Device(device.address))
              }
              .clearAndSetSemantics {
                contentDescription = "${device.label}, ${spoken(device, statuses)}"
              }
              .onGloballyPositioned { columns[device.address] = it.boundsInParent() },
        ) {
          Box(Modifier.height(tall), contentAlignment = Alignment.BottomStart) {
            if (connected && statuses.isNotEmpty()) {
              Bars(
                statuses,
                tall = tall,
                widest = with(density) { w.toDp() },
                spacing = with(density) { gap.toDp() },
                first = first,
                highlight = highlight,
                fold = fold,
              )
            } else {
              Fallen(width = 40.dp)
            }
          }
          val color =
            if (connected) MaterialTheme.colorScheme.onSurfaceVariant
            else MaterialTheme.colorScheme.outline
          Row(
            verticalAlignment = Alignment.CenterVertically,
            horizontalArrangement = Arrangement.spacedBy(4.dp),
            modifier = Modifier.padding(start = 2.dp).alpha(if (active) 0.4f else 1f),
          ) {
            if (!connected) {
              Icon(
                painterResource(
                  if (device.status is DeviceStatus.Revoked) R.drawable.ic_key
                  else R.drawable.ic_bolt
                ),
                contentDescription = null,
                tint = color,
                modifier = Modifier.size(10.dp),
              )
            }
            Text(
              device.label,
              style = MaterialTheme.typography.labelMedium,
              fontWeight = FontWeight.SemiBold,
              color = color,
              maxLines = 1,
              softWrap = false,
            )
          }
        }
        first += items.size
      }
    }
    // What the finger is on: level with the bars, off to the side away from the finger, so the
    // hand on the bars never covers it.
    BoxWithConstraints(Modifier.matchParentSize()) {
      val width = constraints.maxWidth.toFloat()
      val x = shown?.let(::screenX) ?: (width / 2)
      val away = with(density) { if (x < width / 2) x + 118.dp.toPx() else x - 118.dp.toPx() }
      val margin = with(density) { 110.dp.toPx() }
      val centre by
        animateFloatAsState(
          if (width > 2 * margin) away.coerceIn(margin, width - margin) else width / 2,
          tween(200),
          label = "caption",
        )
      val level = with(density) { tall.toPx() * 0.45f }
      AnimatedVisibility(
        visible = active,
        enter = scaleIn(initialScale = 0.85f) + fadeIn(),
        exit = scaleOut(targetScale = 0.85f) + fadeOut(),
        modifier =
          Modifier.layout { measurable, constraints ->
            val caption = measurable.measure(Constraints())
            layout(constraints.maxWidth, constraints.maxHeight) {
              caption.place(
                (centre - caption.width / 2).roundToInt(),
                (level - caption.height / 2).roundToInt(),
              )
            }
          },
      ) {
        ScrubCaption(shown, touch != null)
      }
    }
  }
}

/**
 * Lays the score out `margin` past each side of the list's padding, so its row runs from screen
 * edge to screen edge.
 */
private fun Modifier.edgeToEdge(margin: Dp) = layout { measurable, constraints ->
  val extra = (margin * 2).roundToPx()
  val wide =
    measurable.measure(
      constraints.copy(
        minWidth = constraints.minWidth + extra,
        maxWidth = constraints.maxWidth + extra,
      )
    )
  layout(wide.width - extra, wide.height) { wide.place(-extra / 2, 0) }
}

/** A computer's Agents in words, for TalkBack. */
private fun spoken(device: DeviceView, statuses: List<AgentStatus>): String =
  when (device.status) {
    is DeviceStatus.Connected -> {
      val parts =
        listOf(
            AgentStatus.BLOCKED to "need you",
            AgentStatus.ERROR to "failed",
            AgentStatus.WORKING to "working",
            AgentStatus.DONE to "done",
            AgentStatus.IDLE to "idle",
          )
          .map { (status, words) -> statuses.count { it == status } to words }
          .filter { it.first > 0 }
          .map { "${it.first} ${it.second}" }
      if (parts.isEmpty()) "no agents" else parts.joinToString(", ")
    }
    is DeviceStatus.Connecting,
    is DeviceStatus.Suspended -> "connecting"
    is DeviceStatus.Failed -> "can't connect"
    is DeviceStatus.Revoked -> "needs pairing again"
  }

/** What the finger is on while scrubbing, in a floating capsule. */
@Composable
private fun ScrubCaption(shown: PaneRef?, touching: Boolean) {
  val session = LocalStores.current.session
  Surface(
    shape = MaterialTheme.shapes.extraLarge,
    tonalElevation = 6.dp,
    shadowElevation = 6.dp,
    modifier = Modifier.clearAndSetSemantics {},
  ) {
    Row(
      verticalAlignment = Alignment.CenterVertically,
      horizontalArrangement = Arrangement.spacedBy(9.dp),
      modifier = Modifier.padding(horizontal = 16.dp, vertical = 10.dp),
    ) {
      val pane = shown?.let { session.pane(it) }
      val agent = pane?.agent
      val device = shown?.let { session.device(it.address) }
      if (pane != null && agent != null && device != null) {
        Icon(
          painterResource(AgentMarks.resource(agent.kind) ?: R.drawable.ic_terminal),
          contentDescription = null,
          modifier = Modifier.size(16.dp),
        )
        Column {
          Text(pane.name, style = MaterialTheme.typography.titleSmall, maxLines = 1)
          Row(
            verticalAlignment = Alignment.CenterVertically,
            horizontalArrangement = Arrangement.spacedBy(5.dp),
          ) {
            StatusGlyph(agent.status, 6.dp)
            Text(
              "${agent.status.title} · ${device.label}",
              style = MaterialTheme.typography.bodySmall,
              color = MaterialTheme.colorScheme.onSurfaceVariant,
            )
          }
        }
      } else {
        Text(
          if (touching) "Let go to cancel" else "Slide to an agent",
          style = MaterialTheme.typography.bodyMedium,
          color = MaterialTheme.colorScheme.onSurfaceVariant,
        )
      }
    }
  }
}

/** `-e scrub "<device index>:<bar index>"` holds a scrub for review. */
private fun stagedScrub(spec: String?, session: dev.condr.companion.model.Session): PaneRef? {
  val parts = spec?.split(":")?.mapNotNull { it.toIntOrNull() } ?: return null
  if (parts.size != 2) return null
  val device = session.devices.getOrNull(parts[0]) ?: return null
  return session.agents(device).byUrgency().getOrNull(parts[1])?.ref
}
