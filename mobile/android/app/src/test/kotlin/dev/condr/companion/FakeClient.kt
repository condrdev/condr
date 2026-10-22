package dev.condr.companion

import uniffi.condr_mobile.Change
import uniffi.condr_mobile.CompanionInterface
import uniffi.condr_mobile.DeviceView
import uniffi.condr_mobile.Event
import uniffi.condr_mobile.Key
import uniffi.condr_mobile.NeedsYouItem
import uniffi.condr_mobile.SavedDevice
import uniffi.condr_mobile.TerminalScreen
import uniffi.condr_mobile.WorkspaceView

/** The library's shape with nothing behind it, for tests of what the app does around it. */
class FakeClient(private val onPush: (Boolean) -> Unit = {}) : CompanionInterface {
  override fun clearSelection(address: String, pane: ULong) {}

  override fun click(address: String, pane: ULong, row: UShort, column: UShort) {}

  override fun shutdown() {}

  override fun cloudToken(): String = "token"

  override fun copySelection(address: String, pane: ULong) {}

  override fun devices(): List<DeviceView> = emptyList()

  override fun forget(address: String) {}

  override fun needsYou(): List<NeedsYouItem> = emptyList()

  override fun networkChanged() {}

  override fun pair(link: String, label: String?): SavedDevice = SavedDevice(link, label ?: "")

  override fun paste(address: String, pane: ULong, text: String) {}

  override fun rename(address: String, label: String) {}

  override fun resume() {}

  override fun scrollToBottom(address: String, pane: ULong) {}

  override fun selectRange(
    address: String,
    pane: ULong,
    fromRow: UShort,
    fromColumn: UShort,
    toRow: UShort,
    toColumn: UShort,
  ) {}

  override fun selectWord(address: String, pane: ULong, row: UShort, column: UShort) {}

  override fun sendKey(address: String, pane: ULong, key: Key, control: Boolean) {}

  override fun sendText(address: String, pane: ULong, text: String) {}

  override fun setActive(active: Boolean) {}

  override fun setName(name: String) {}

  override fun setPush(push: Boolean) = onPush(push)

  override fun showPane(address: String?, pane: ULong, rows: UShort, columns: UShort) {}

  override fun submit(address: String, pane: ULong, text: String) {}

  override fun suspend() {}

  override fun takeChanges(): List<Change> = emptyList()

  override fun takeEvents(): List<Event> = emptyList()

  override fun terminal(address: String, pane: ULong): TerminalScreen? = null

  override fun wheel(address: String, pane: ULong, row: UShort, column: UShort, lines: Int) {}

  override fun workspaces(address: String): List<WorkspaceView> = emptyList()
}
