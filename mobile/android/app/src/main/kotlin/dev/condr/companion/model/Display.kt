package dev.condr.companion.model

import uniffi.condr_mobile.AgentStatus
import uniffi.condr_mobile.DeviceStatus
import uniffi.condr_mobile.DeviceView
import uniffi.condr_mobile.NeedsYouItem
import uniffi.condr_mobile.PaneView

// The library's records as the lists show them.

val DeviceView.connected: Boolean
  get() = status == DeviceStatus.Connected

/** The title its program set, else what stands in: its Agent's name or "Shell". */
fun PaneView.name(fallback: String): String = title ?: fallback

val PaneView.name: String
  get() = name(agent?.label ?: "Shell")

val NeedsYouItem.ref: PaneRef
  get() = PaneRef(address, pane)

/** Waiting for an answer or halted on a failure: the person has to come (ADR 0044). */
val AgentStatus.needsYou: Boolean
  get() = this == AgentStatus.BLOCKED || this == AgentStatus.ERROR

/** Most urgent first, the order a score draws them in, so each colour reads as a run. */
val AgentStatus.urgency: Int
  get() =
    when (this) {
      AgentStatus.BLOCKED -> 0
      AgentStatus.ERROR -> 1
      AgentStatus.DONE -> 2
      AgentStatus.WORKING -> 3
      AgentStatus.UNKNOWN -> 4
      AgentStatus.IDLE -> 5
    }

val AgentStatus.title: String
  get() =
    when (this) {
      AgentStatus.WORKING -> "Working"
      AgentStatus.BLOCKED -> "Needs you"
      AgentStatus.ERROR -> "Failed"
      AgentStatus.DONE -> "Done"
      AgentStatus.IDLE -> "Idle"
      AgentStatus.UNKNOWN -> "Not reporting"
    }

fun urgencyOf(status: AgentStatus): Int = status.urgency

/** In the order a score draws them: most urgent first, stable within a state. */
fun List<AgentItem>.byUrgency(): List<AgentItem> = sortedBy { it.agent.status.urgency }

/** "condr · macmini" */
fun place(workspace: String, device: String): String = "$workspace · $device"

/**
 * A Blocked Agent's `detail` as the agent wrote it (ADR 0024): "Tool: command" for a permission,
 * else a question or the notification's text.
 */
class BlockedOn(raw: String) {
  val tool: String?
  val text: String

  init {
    val colon = raw.indexOf(": ")
    val head = if (colon > 0) raw.substring(0, colon) else ""
    if (colon > 0 && head.all { it.isLetterOrDigit() || it == '_' }) {
      tool = head
      text = raw.substring(colon + 2)
    } else {
      tool = null
      text = raw
    }
  }
}
