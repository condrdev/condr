package dev.condr.companion.ui.agents

import androidx.compose.foundation.clickable
import androidx.compose.foundation.gestures.detectDragGesturesAfterLongPress
import androidx.compose.foundation.horizontalScroll
import androidx.compose.foundation.layout.Arrangement
import androidx.compose.foundation.layout.Box
import androidx.compose.foundation.layout.Column
import androidx.compose.foundation.layout.Row
import androidx.compose.foundation.layout.fillMaxWidth
import androidx.compose.foundation.layout.height
import androidx.compose.foundation.layout.padding
import androidx.compose.foundation.layout.widthIn
import androidx.compose.foundation.rememberScrollState
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
import androidx.compose.ui.geometry.Offset
import androidx.compose.ui.geometry.Rect
import androidx.compose.ui.hapticfeedback.HapticFeedbackType
import androidx.compose.ui.input.pointer.pointerInput
import androidx.compose.ui.layout.boundsInParent
import androidx.compose.ui.layout.onGloballyPositioned
import androidx.compose.ui.platform.LocalDensity
import androidx.compose.ui.platform.LocalHapticFeedback
import androidx.compose.ui.text.font.FontWeight
import androidx.compose.ui.unit.dp
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

/**
 * The first screen's head: each computer as a column of the logo's bars, one per Agent, most urgent
 * first, so a colour reads as a run however many there are. A tap on a column opens its computer.
 * Press, hold and slide to scrub the bars: each Agent passed under the finger ticks and names
 * itself, and letting go opens its terminal.
 */
@Composable
fun Score(modifier: Modifier = Modifier) {
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

  Box(modifier.fillMaxWidth()) {
    Row(
      horizontalArrangement = Arrangement.spacedBy(24.dp),
      verticalAlignment = Alignment.Bottom,
      modifier =
        Modifier.horizontalScroll(rememberScrollState(), enabled = !active)
          .padding(horizontal = 20.dp)
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
                if (next != scrub) {
                  scrub = next
                  if (next != null) haptics.performHapticFeedback(HapticFeedbackType.SegmentTick)
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
        Column(
          verticalArrangement = Arrangement.spacedBy(10.dp),
          modifier =
            Modifier.widthIn(min = 44.dp)
              .clickable(enabled = !active) {
                router.agentsPath.add(DeviceRoute.Device(device.address))
              }
              .onGloballyPositioned { columns[device.address] = it.boundsInParent() },
        ) {
          Box(Modifier.height(tall), contentAlignment = Alignment.BottomStart) {
            if (device.connected && statuses.isNotEmpty()) {
              Bars(
                statuses,
                tall = tall,
                widest = with(density) { w.toDp() },
                spacing = with(density) { gap.toDp() },
                first = first,
                highlight = highlight,
              )
            } else {
              Fallen(width = 40.dp)
            }
          }
          Text(
            device.label,
            style = MaterialTheme.typography.labelMedium,
            fontWeight = FontWeight.SemiBold,
            color =
              if (device.connected) MaterialTheme.colorScheme.onSurfaceVariant
              else MaterialTheme.colorScheme.outline,
            modifier = Modifier.padding(start = 2.dp),
          )
        }
        first += items.size
      }
    }
    if (active) ScrubCaption(shown, touch != null)
  }
}

/** What the finger is on while scrubbing, in a floating capsule above the bars. */
@Composable
private fun ScrubCaption(shown: PaneRef?, touching: Boolean) {
  val session = LocalStores.current.session
  Surface(
    shape = MaterialTheme.shapes.extraLarge,
    tonalElevation = 6.dp,
    shadowElevation = 6.dp,
    modifier = Modifier.padding(top = 4.dp, start = 24.dp),
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
        AgentAvatar(kind = agent.kind, status = agent.status, side = 28.dp)
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
