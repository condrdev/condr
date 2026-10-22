package dev.condr.companion.ui

import androidx.compose.foundation.background
import androidx.compose.foundation.layout.Arrangement
import androidx.compose.foundation.layout.Box
import androidx.compose.foundation.layout.Column
import androidx.compose.foundation.layout.fillMaxSize
import androidx.compose.foundation.layout.padding
import androidx.compose.foundation.layout.size
import androidx.compose.material.icons.Icons
import androidx.compose.material.icons.filled.Lock
import androidx.compose.material.icons.filled.Settings
import androidx.compose.material3.AlertDialog
import androidx.compose.material3.Badge
import androidx.compose.material3.BadgedBox
import androidx.compose.material3.Button
import androidx.compose.material3.Icon
import androidx.compose.material3.MaterialTheme
import androidx.compose.material3.NavigationBar
import androidx.compose.material3.NavigationBarItem
import androidx.compose.material3.Scaffold
import androidx.compose.material3.SnackbarDuration
import androidx.compose.material3.SnackbarHost
import androidx.compose.material3.SnackbarHostState
import androidx.compose.material3.SnackbarResult
import androidx.compose.material3.Text
import androidx.compose.material3.TextButton
import androidx.compose.runtime.Composable
import androidx.compose.runtime.CompositionLocalProvider
import androidx.compose.runtime.LaunchedEffect
import androidx.compose.runtime.getValue
import androidx.compose.runtime.mutableStateOf
import androidx.compose.runtime.remember
import androidx.compose.runtime.rememberCoroutineScope
import androidx.compose.runtime.setValue
import androidx.compose.runtime.staticCompositionLocalOf
import androidx.compose.ui.Alignment
import androidx.compose.ui.Modifier
import androidx.compose.ui.platform.LocalContext
import androidx.compose.ui.res.painterResource
import androidx.compose.ui.text.font.FontWeight
import androidx.compose.ui.unit.dp
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
import dev.condr.companion.ui.pairing.PairingFlow
import dev.condr.companion.ui.settings.LostPhoneScreen
import dev.condr.companion.ui.settings.PhoneNameScreen
import dev.condr.companion.ui.settings.SettingsScreen
import dev.condr.companion.ui.terminal.TerminalPage
import dev.condr.companion.ui.theme.TerminalTheme
import kotlinx.coroutines.launch
import kotlinx.serialization.Serializable
import uniffi.condr_mobile.AgentStatus

val LocalStores = staticCompositionLocalOf<Stores> { error("No Stores") }

/** Each tab's stack starts at its own first screen. */
@Serializable data object Home : NavKey

@Composable
fun App(stores: Stores) {
  CompositionLocalProvider(LocalStores provides stores) {
    Box(Modifier.fillMaxSize()) {
      // The first pairing's flow stays up through its last step, until the person moves on.
      var onboarding by remember(stores) { mutableStateOf(!stores.session.paired) }
      if (onboarding) PairingFlow(firstLaunch = true, finished = { onboarding = false })
      else MainTabs()
      if (stores.router.pairingAnother) {
        Box(Modifier.fillMaxSize().background(MaterialTheme.colorScheme.surface)) {
          PairingFlow(firstLaunch = false, finished = { stores.router.pairingAnother = false })
        }
      }
      Overlays()
      if (stores.lock.locked) LockScreen()
    }
  }
}

@Composable
private fun MainTabs() {
  val stores = LocalStores.current
  val router = stores.router
  val session = stores.session
  val inPane =
    when (router.tab) {
      AppTab.AGENTS -> router.agentsPath.lastOrNull() is DeviceRoute.Pane
      AppTab.DEVICES -> router.devicesPath.lastOrNull() is DeviceRoute.Pane
      AppTab.SETTINGS -> false
    }
  Scaffold(
    bottomBar = {
      if (inPane) return@Scaffold
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
    // A Pane's page draws to the bottom edge and keeps its own insets.
    Box(
      Modifier.fillMaxSize().padding(bottom = if (inPane) 0.dp else insets.calculateBottomPadding())
    ) {
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
        entry<DeviceRoute.Pane> { TerminalTheme { TerminalPage(it.ref) } }
      },
  )
}

/**
 * What the app says over its screens (ADR 0020): a banner for an Agent elsewhere that needs you or
 * finished, as a snackbar with Open; an error to read, copy and dismiss, as a dialog; and a passing
 * notice.
 */
@Composable
private fun Overlays() {
  val stores = LocalStores.current
  val alerts = stores.alerts
  val router = stores.router
  val context = LocalContext.current
  val host = remember { SnackbarHostState() }
  val banner = alerts.banner
  LaunchedEffect(banner?.id) {
    val shown = banner ?: return@LaunchedEffect
    if (shown.quiet) return@LaunchedEffect
    val result =
      host.showSnackbar(
        message = "${shown.title} · ${shown.detail}",
        actionLabel = "Open",
        duration = SnackbarDuration.Long,
        withDismissAction = true,
      )
    if (result == SnackbarResult.ActionPerformed) router.open(shown.ref)
    alerts.dismissBanner()
  }
  val notice = alerts.notice
  LaunchedEffect(notice) {
    val text = notice ?: return@LaunchedEffect
    host.showSnackbar(text, duration = SnackbarDuration.Short)
  }
  Box(Modifier.fillMaxSize()) {
    SnackbarHost(host, Modifier.align(Alignment.BottomCenter).padding(bottom = 80.dp))
  }
  val error = alerts.error
  if (error != null) {
    AlertDialog(
      onDismissRequest = { alerts.dismissError() },
      title = { Text("Something went wrong") },
      text = { Text(error) },
      confirmButton = { TextButton(onClick = { alerts.dismissError() }) { Text("OK") } },
      dismissButton = {
        TextButton(
          onClick = {
            context
              .getSystemService(android.content.ClipboardManager::class.java)
              .setPrimaryClip(android.content.ClipData.newPlainText("Condr", error))
          }
        ) {
          Text("Copy")
        }
      },
    )
  }
}

/** The lock over everything (ADR 0041): it asks as it appears, and again on the button. */
@Composable
private fun LockScreen() {
  val lock = LocalStores.current.lock
  val scope = rememberCoroutineScope()
  LaunchedEffect(Unit) { if (lock.promptsOnAppear) lock.unlock() }
  Column(
    Modifier.fillMaxSize().background(MaterialTheme.colorScheme.surface).padding(32.dp),
    verticalArrangement = Arrangement.Center,
    horizontalAlignment = Alignment.CenterHorizontally,
  ) {
    Icon(
      Icons.Filled.Lock,
      contentDescription = null,
      tint = MaterialTheme.colorScheme.onSurfaceVariant,
      modifier = Modifier.size(40.dp),
    )
    Text(
      "Condr is locked",
      style = MaterialTheme.typography.headlineSmall,
      fontWeight = FontWeight.Bold,
      modifier = Modifier.padding(top = 20.dp, bottom = 24.dp),
    )
    Button(onClick = { scope.launch { lock.unlock() } }, enabled = !lock.authenticating) {
      Text("Unlock")
    }
  }
}
