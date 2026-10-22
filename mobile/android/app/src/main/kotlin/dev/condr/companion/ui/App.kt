package dev.condr.companion.ui

import androidx.compose.foundation.layout.Box
import androidx.compose.foundation.layout.fillMaxSize
import androidx.compose.foundation.layout.padding
import androidx.compose.material.icons.Icons
import androidx.compose.material.icons.filled.Settings
import androidx.compose.material3.Badge
import androidx.compose.material3.BadgedBox
import androidx.compose.material3.Icon
import androidx.compose.material3.NavigationBar
import androidx.compose.material3.NavigationBarItem
import androidx.compose.material3.Scaffold
import androidx.compose.material3.Text
import androidx.compose.runtime.Composable
import androidx.compose.runtime.CompositionLocalProvider
import androidx.compose.runtime.staticCompositionLocalOf
import androidx.compose.ui.Modifier
import androidx.compose.ui.res.painterResource
import androidx.navigation3.runtime.NavKey
import androidx.navigation3.runtime.entryProvider
import androidx.navigation3.ui.NavDisplay
import dev.condr.companion.R
import dev.condr.companion.Stores
import dev.condr.companion.model.AppTab
import dev.condr.companion.model.DeviceRoute
import dev.condr.companion.model.SettingsRoute
import dev.condr.companion.ui.agents.AgentsScreen
import dev.condr.companion.ui.devices.DeviceScreen
import dev.condr.companion.ui.devices.DevicesScreen
import dev.condr.companion.ui.pairing.WelcomeScreen
import dev.condr.companion.ui.settings.LostPhoneScreen
import dev.condr.companion.ui.settings.PhoneNameScreen
import dev.condr.companion.ui.settings.SettingsScreen
import dev.condr.companion.ui.terminal.TerminalPage
import kotlinx.serialization.Serializable
import uniffi.condr_mobile.AgentStatus

val LocalStores = staticCompositionLocalOf<Stores> { error("No Stores") }

/** Each tab's stack starts at its own first screen. */
@Serializable data object Home : NavKey

@Composable
fun App(stores: Stores) {
  CompositionLocalProvider(LocalStores provides stores) {
    if (stores.session.paired) MainTabs() else WelcomeScreen()
  }
}

@Composable
private fun MainTabs() {
  val stores = LocalStores.current
  val router = stores.router
  val session = stores.session
  Scaffold(
    bottomBar = {
      NavigationBar {
        val needsYou = session.agents(AgentStatus.BLOCKED, AgentStatus.ERROR).size
        NavigationBarItem(
          selected = router.tab == AppTab.AGENTS,
          onClick = { router.tab = AppTab.AGENTS },
          icon = {
            BadgedBox(badge = { if (needsYou > 0) Badge { Text("$needsYou") } }) {
              Icon(painterResource(R.drawable.ic_tab_agents), contentDescription = null)
            }
          },
          label = { Text("Agents") },
        )
        NavigationBarItem(
          selected = router.tab == AppTab.DEVICES,
          onClick = { router.tab = AppTab.DEVICES },
          icon = { Icon(painterResource(R.drawable.ic_tab_devices), contentDescription = null) },
          label = { Text("Devices") },
        )
        NavigationBarItem(
          selected = router.tab == AppTab.SETTINGS,
          onClick = { router.tab = AppTab.SETTINGS },
          icon = { Icon(Icons.Filled.Settings, contentDescription = null) },
          label = { Text("Settings") },
        )
      }
    }
  ) { insets ->
    Box(Modifier.fillMaxSize().padding(bottom = insets.calculateBottomPadding())) {
      when (router.tab) {
        AppTab.AGENTS -> DeviceStack(router.agentsPath, root = { AgentsScreen() })
        AppTab.DEVICES -> DeviceStack(router.devicesPath, root = { DevicesScreen() })
        AppTab.SETTINGS ->
          NavDisplay(
            backStack = listOf<NavKey>(Home) + router.settingsPath,
            onBack = { router.settingsPath.removeLastOrNull() },
            entryProvider =
              entryProvider {
                entry<Home> { SettingsScreen() }
                entry<SettingsRoute.PhoneName> { PhoneNameScreen() }
                entry<SettingsRoute.LostPhone> { LostPhoneScreen() }
              },
          )
      }
    }
  }
}

/** The Agents and Devices tabs both open a Device's Workspaces and a Pane (ADR 0041). */
@Composable
private fun DeviceStack(path: MutableList<DeviceRoute>, root: @Composable () -> Unit) {
  NavDisplay(
    backStack = listOf<NavKey>(Home) + path,
    onBack = { path.removeLastOrNull() },
    entryProvider =
      entryProvider {
        entry<Home> { root() }
        entry<DeviceRoute.Device> { DeviceScreen(it.address) }
        entry<DeviceRoute.Pane> { TerminalPage(it.ref) }
      },
  )
}
