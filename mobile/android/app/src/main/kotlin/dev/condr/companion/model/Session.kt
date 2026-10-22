package dev.condr.companion.model

import android.content.ClipData
import android.content.ClipboardManager
import android.content.Context
import android.os.Handler
import android.os.Looper
import androidx.compose.runtime.getValue
import androidx.compose.runtime.mutableStateMapOf
import androidx.compose.runtime.mutableStateOf
import androidx.compose.runtime.setValue
import dev.condr.companion.persistence.SavedDeviceStore
import kotlinx.coroutines.Dispatchers
import kotlinx.coroutines.withContext
import uniffi.condr_mobile.AgentStatus
import uniffi.condr_mobile.AgentView
import uniffi.condr_mobile.Change
import uniffi.condr_mobile.CompanionInterface
import uniffi.condr_mobile.DeviceView
import uniffi.condr_mobile.Event
import uniffi.condr_mobile.Key
import uniffi.condr_mobile.Listener
import uniffi.condr_mobile.PaneView
import uniffi.condr_mobile.SavedDevice
import uniffi.condr_mobile.TerminalScreen
import uniffi.condr_mobile.WorkspaceView

/** A Pane on a Device, as the lists and routes name one. */
@kotlinx.serialization.Serializable data class PaneRef(val address: String, val pane: ULong)

/** One Agent with where it runs, for the lists. */
data class AgentItem(
  val ref: PaneRef,
  /** The Device's name. */
  val device: String,
  val workspace: String,
  val pane: PaneView,
  val agent: AgentView,
)

/** The Pane a notification names (ADR 0047). */
data class PushedPane(val device: String, val pane: ULong)

/**
 * What the library tells the app: the paired Devices, their Workspaces and Agents, the "Needs you"
 * list, and the screen of the one Pane on display (ADR 0039, ADR 0041). It re-reads what `changed`
 * names and turns what it saw change into banners. Every member runs on the main thread; the
 * library is the state and this projects it into Compose (ADR 0048).
 */
class Session(
  private val client: CompanionInterface,
  /** This phone's key, as a Device's Paired devices shows it. */
  val fingerprint: String,
  private val saved: SavedDeviceStore,
  private val alerts: Alerts,
  private val router: Router,
  private val context: Context,
) {
  var devices: List<DeviceView> by mutableStateOf(client.devices())
    private set

  val workspaces = mutableStateMapOf<String, List<WorkspaceView>>()
  var needsYou by mutableStateOf(client.needsYou())
    private set

  /** The Pane on screen: the one the phone watches and sizes (ADR 0041). */
  var shown: PaneRef? by mutableStateOf(null)
    private set

  /** The shown Pane's screen, once its first full frame arrived. */
  var screen: TerminalScreen? by mutableStateOf(null)
    private set

  /** Every Agent's last seen status, to notice the changes worth a banner. */
  private var statuses: Map<PaneRef, AgentStatus>

  init {
    for (device in devices) workspaces[device.address] = client.workspaces(device.address)
    statuses = agents.associate { it.ref to it.agent.status }
  }

  /** At least one Device is paired, so the app shows its tabs rather than the welcome. */
  val paired: Boolean
    get() = devices.isNotEmpty()

  // Hearing the library

  /** The library said something changed: read what, on the main thread. */
  fun refresh() {
    for (change in client.takeChanges()) {
      when (change) {
        is Change.Devices -> {
          devices = client.devices()
          for (device in devices) {
            if (device.address !in workspaces) {
              workspaces[device.address] = client.workspaces(device.address)
            }
          }
          val known = devices.map { it.address }.toSet()
          workspaces.keys.filter { it !in known }.forEach { workspaces.remove(it) }
        }
        is Change.NeedsYou -> needsYou = client.needsYou()
        is Change.Device -> workspaces[change.address] = client.workspaces(change.address)
        is Change.Terminal -> {
          if (shown == PaneRef(change.address, change.pane)) {
            screen = client.terminal(change.address, change.pane)
          }
        }
      }
    }
    for (event in client.takeEvents()) {
      when (event) {
        is Event.Error -> alerts.report(event.message)
        is Event.Copied -> copied(event.text)
        is Event.Clipboard -> copied(event.text)
      }
    }
    noticeChanges()
  }

  private fun copied(text: String) {
    val clipboard = context.getSystemService(ClipboardManager::class.java)
    clipboard.setPrimaryClip(ClipData.newPlainText("Condr", text))
    alerts.notify("Copied to clipboard")
  }

  /**
   * An Agent elsewhere that came to need you or finished since the last look earns a banner (ADR
   * 0024); one first seen in that state does not, the list shows it.
   */
  private fun noticeChanges() {
    val seen = HashMap<PaneRef, AgentStatus>()
    for (item in agents) {
      seen[item.ref] = item.agent.status
      val before = statuses[item.ref] ?: continue
      val status = item.agent.status
      if (before == status || !(status.needsYou || status == AgentStatus.DONE)) continue
      if (shown == item.ref) continue
      val title =
        when (status) {
          AgentStatus.BLOCKED -> "${item.agent.label} needs you"
          AgentStatus.ERROR -> "${item.agent.label} failed"
          else -> "${item.agent.label} is done"
        }
      alerts.raise(
        Banner(
          ref = item.ref,
          status = status,
          kind = item.agent.kind,
          title = title,
          detail = "${item.pane.name(item.agent.label)} · ${place(item.workspace, item.device)}",
          quiet = router.showsHome,
        )
      )
    }
    statuses = seen
  }

  // Lifecycle (ADR 0040)

  /** The app came to the foreground: connect to every Device, each with a fresh Bootstrap. */
  fun resume() = client.resume()

  /**
   * The app is leaving the screen: close every connection now, off the main thread, and return once
   * they are closed.
   */
  suspend fun suspend() = withContext(Dispatchers.IO) { client.suspend() }

  fun networkChanged() = client.networkChanged()

  /**
   * The app is on screen but not active: the shown Pane's focus and size go back until it is active
   * again, and are taken back then (ADR 0041).
   */
  fun setActive(active: Boolean) = client.setActive(active)

  /**
   * The phone's new name (ADR 0040). A Server takes the name each `Hello` carries, so every
   * connection is made again now and each computer lists the phone by it at once.
   */
  suspend fun rename(name: String) {
    client.setName(name)
    suspend()
    resume()
  }

  // Devices

  /**
   * Pairs with the Device an Invite link names (ADR 0040), as the phone named `phoneName`; the
   * handshake runs off the main thread. The Device is kept here under the name its Server gave, and
   * the library connects to it.
   */
  suspend fun pair(link: String, phoneName: String): SavedDevice {
    client.setName(phoneName)
    val device = withContext(Dispatchers.IO) { client.pair(link, null) }
    saved.add(device)
    devices = client.devices()
    workspaces[device.address] = client.workspaces(device.address)
    return device
  }

  /** The name this phone shows a Device under from now on; a blank one changes nothing. */
  fun rename(address: String, label: String) {
    val trimmed = label.trim()
    if (trimmed.isEmpty()) return
    client.rename(address, trimmed)
    saved.rename(address, trimmed)
    devices = client.devices()
    needsYou = client.needsYou()
  }

  /** Stops using a Device here; revoking the phone is done on the Device (ADR 0040). */
  fun forget(address: String) {
    if (shown?.address == address) {
      shown = null
      screen = null
    }
    client.forget(address)
    saved.remove(address)
    devices = client.devices()
    workspaces.remove(address)
    needsYou = client.needsYou()
    statuses = statuses.filterKeys { it.address != address }
    router.forget(address)
  }

  fun device(address: String): DeviceView? = devices.firstOrNull { it.address == address }

  /** The Pane a notification names, on a Device paired here. */
  fun ref(pushed: PushedPane): PaneRef? =
    devices.firstOrNull { it.deviceId == pushed.device }?.let { PaneRef(it.address, pushed.pane) }

  /**
   * A notification shows in the foreground only while its Device is not connected here: a connected
   * one's change already raises a banner, and the Pane on screen none.
   */
  fun presents(pushed: PushedPane): Boolean {
    val ref = ref(pushed) ?: return true
    return device(ref.address)?.connected != true && shown != ref
  }

  fun pane(ref: PaneRef): PaneView? =
    workspaces[ref.address]?.asSequence()?.flatMap { it.panes }?.firstOrNull { it.id == ref.pane }

  fun workspace(ref: PaneRef): WorkspaceView? =
    workspaces[ref.address]?.firstOrNull { workspace -> workspace.panes.any { it.id == ref.pane } }

  // Agents

  /** Every Agent on every connected Device, in Device and Workspace order. */
  val agents: List<AgentItem>
    get() = devices.filter { it.connected }.flatMap(::agents)

  fun agents(vararg statuses: AgentStatus): List<AgentItem> = agents.filter {
    it.agent.status in statuses
  }

  /** A Device's Agents, whatever its connection. */
  fun agents(device: DeviceView): List<AgentItem> =
    (workspaces[device.address] ?: emptyList()).flatMap { workspace ->
      workspace.panes.mapNotNull { pane ->
        pane.agent?.let { agent ->
          AgentItem(PaneRef(device.address, pane.id), device.label, workspace.name, pane, agent)
        }
      }
    }

  // The Pane on screen (ADR 0041)

  /**
   * The Pane now on screen, at the grid its font and visible area give: the phone reports focus and
   * sizes the terminal like a window. Opening it marks it seen.
   */
  fun show(ref: PaneRef, rows: UShort, columns: UShort) {
    if (shown != ref) {
      shown = ref
      screen = client.terminal(ref.address, ref.pane)
      alerts.dismissBanner(ref)
    }
    client.showPane(ref.address, ref.pane, rows, columns)
  }

  /** The Pane left the screen or the app went inactive: focus and sizing go back. */
  fun hide(ref: PaneRef) {
    if (shown != ref) return
    client.showPane(null, ref.pane, 0u, 0u)
    shown = null
    screen = null
  }

  fun send(text: String) {
    val shown = shown ?: return
    client.sendText(shown.address, shown.pane, text)
  }

  /** A key from the row above the keyboard, with its sticky Ctrl (ADR 0041). */
  fun send(key: Key, control: Boolean = false) {
    val shown = shown ?: return
    client.sendKey(shown.address, shown.pane, key, control)
  }

  /** Pasted text, which the Server brackets when the program asked (ADR 0041). */
  fun paste(text: String) {
    val shown = shown ?: return
    client.paste(shown.address, shown.pane, text)
  }

  /** The field's draft: a paste, then Return once the program has taken it (ADR 0041). */
  fun submit(text: String) {
    val shown = shown ?: return
    client.submit(shown.address, shown.pane, text)
  }

  /** A swipe of `lines` over a cell, positive towards what came before: wheel motion. */
  fun wheel(row: UShort, column: UShort, lines: Int) {
    val shown = shown ?: return
    client.wheel(shown.address, shown.pane, row, column, lines)
  }

  /** A tap where the program tracks the mouse: a click on the cell (ADR 0041). */
  fun click(row: UShort, column: UShort) {
    val shown = shown ?: return
    client.click(shown.address, shown.pane, row, column)
  }

  // The Pane's selection, the one every viewer shares (ADR 0008)

  fun selectWord(row: UShort, column: UShort) {
    val shown = shown ?: return
    client.selectWord(shown.address, shown.pane, row, column)
    refresh()
  }

  fun selectRange(fromRow: UShort, fromColumn: UShort, toRow: UShort, toColumn: UShort) {
    val shown = shown ?: return
    client.selectRange(shown.address, shown.pane, fromRow, fromColumn, toRow, toColumn)
    refresh()
  }

  fun clearSelection() {
    val shown = shown ?: return
    client.clearSelection(shown.address, shown.pane)
    refresh()
  }

  /** Copies the selection; its text comes back as an event, onto the clipboard. */
  fun copySelection() {
    val shown = shown ?: return
    client.copySelection(shown.address, shown.pane)
    refresh()
  }

  fun scrollToBottom() {
    val shown = shown ?: return
    client.scrollToBottom(shown.address, shown.pane)
  }
}

/**
 * Wakes the Session on the main thread when the library reports a change. The library calls
 * `changed` from its own thread, once until `takeChanges` ran (ADR 0039). It is handed to the
 * library before the Session exists, so the Session binds itself after.
 */
class SessionListener : Listener {
  private val main = Handler(Looper.getMainLooper())
  @Volatile private var handler: (() -> Unit)? = null

  fun bind(handler: () -> Unit) {
    this.handler = handler
  }

  override fun changed() {
    main.post { handler?.invoke() }
  }
}
