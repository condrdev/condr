package dev.condr.companion.model

import androidx.compose.runtime.getValue
import androidx.compose.runtime.mutableStateListOf
import androidx.compose.runtime.mutableStateOf
import androidx.compose.runtime.setValue
import androidx.navigation3.runtime.NavKey
import kotlinx.serialization.Serializable

enum class AppTab {
  AGENTS,
  DEVICES,
  SETTINGS,
}

sealed interface DeviceRoute : NavKey {
  @Serializable data class Device(val address: String) : DeviceRoute

  @Serializable data class Pane(val ref: PaneRef) : DeviceRoute
}

sealed interface SettingsRoute : NavKey {
  @Serializable data object PhoneName : SettingsRoute

  @Serializable data object LostPhone : SettingsRoute
}

/**
 * Where the app is: the tab and each tab's stack. Opening a Pane from a banner or the score lands
 * on the Agents tab, and forgetting a Device drops every route into it. The stacks live here, in
 * the process, so a recreated Activity finds them where it left them.
 */
class Router {
  var tab by mutableStateOf(AppTab.AGENTS)
  val agentsPath = mutableStateListOf<DeviceRoute>()
  val devicesPath = mutableStateListOf<DeviceRoute>()
  val settingsPath = mutableStateListOf<SettingsRoute>()
  var pairingAnother by mutableStateOf(false)

  /**
   * The first screen is showing, which shows a change itself: a banner there is felt, not shown.
   */
  val showsHome: Boolean
    get() = tab == AppTab.AGENTS && agentsPath.isEmpty()

  /** Opens a Pane from anywhere: a banner, the score. */
  fun open(ref: PaneRef) {
    tab = AppTab.AGENTS
    agentsPath.clear()
    agentsPath.add(DeviceRoute.Pane(ref))
  }

  /**
   * Opens a Pane from a notification, over its Device's Workspace list, where the app stays if the
   * Pane is gone (ADR 0047).
   */
  fun openNotified(ref: PaneRef) {
    tab = AppTab.AGENTS
    agentsPath.clear()
    agentsPath.add(DeviceRoute.Device(ref.address))
    agentsPath.add(DeviceRoute.Pane(ref))
  }

  /** The Pane is gone: its page leaves whichever stack shows it. */
  fun close(ref: PaneRef) {
    agentsPath.removeAll { it == DeviceRoute.Pane(ref) }
    devicesPath.removeAll { it == DeviceRoute.Pane(ref) }
  }

  fun forget(address: String) {
    val leaves = { route: DeviceRoute ->
      when (route) {
        is DeviceRoute.Device -> route.address == address
        is DeviceRoute.Pane -> route.ref.address == address
      }
    }
    agentsPath.removeAll(leaves)
    devicesPath.removeAll(leaves)
  }
}
