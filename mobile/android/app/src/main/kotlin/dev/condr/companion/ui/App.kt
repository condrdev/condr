package dev.condr.companion.ui

import android.os.Build
import androidx.activity.compose.rememberLauncherForActivityResult
import androidx.activity.result.contract.ActivityResultContracts
import androidx.compose.foundation.background
import androidx.compose.foundation.layout.Arrangement
import androidx.compose.foundation.layout.Box
import androidx.compose.foundation.layout.Column
import androidx.compose.foundation.layout.Row
import androidx.compose.foundation.layout.fillMaxSize
import androidx.compose.foundation.layout.fillMaxWidth
import androidx.compose.foundation.layout.padding
import androidx.compose.foundation.layout.size
import androidx.compose.foundation.layout.statusBarsPadding
import androidx.compose.material.icons.Icons
import androidx.compose.material.icons.filled.Lock
import androidx.compose.material.icons.filled.Settings
import androidx.compose.material3.Badge
import androidx.compose.material3.BadgedBox
import androidx.compose.material3.Button
import androidx.compose.material3.Icon
import androidx.compose.material3.MaterialTheme
import androidx.compose.material3.NavigationBar
import androidx.compose.material3.NavigationBarItem
import androidx.compose.material3.Scaffold
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
import dev.condr.companion.push.Notifications
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
      // The first pairing's flow stays up through its last step, until the person moves on,
      // which is when the app asks to notify (ADR 0048).
      var onboarding by remember(stores) { mutableStateOf(!stores.session.paired) }
      var trying by remember { mutableStateOf(false) }
      val pushes = rememberPushes()
      if (onboarding) {
        PairingFlow(
          firstLaunch = true,
          finished = {
            onboarding = false
            pushes.ask()
          },
          tryDemo = { trying = true },
        )
      } else MainTabs()
      if (trying) DemoExperience(exit = { trying = false })
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

/**
 * Keeps Condr Cloud told what this phone accepts (ADR 0047): the permission as it stands, the
 * Devices paired, and FCM's token, fetched once per launch.
 */
@Composable
private fun rememberPushes(): Pushes {
  val stores = LocalStores.current
  val push = stores.push
  val context = LocalContext.current
  var granted by remember { mutableStateOf(Notifications.granted(context)) }
  val ask =
    rememberLauncherForActivityResult(ActivityResultContracts.RequestPermission()) { granted = it }
  val servers = stores.session.devices.mapNotNull { it.deviceId }
  LaunchedEffect(push) {
    val push = push ?: return@LaunchedEffect
    Notifications.fetchToken(context, received = { push.received(it) }, failed = { push.failed() })
  }
  LaunchedEffect(push, granted, servers) {
    android.util.Log.i(
      "condr.push",
      "update granted=$granted servers=${servers.size} push=${push != null}",
    )
    push?.update(granted, servers)
  }
  return remember(ask) {
    Pushes {
      if (Build.VERSION.SDK_INT >= Build.VERSION_CODES.TIRAMISU && !granted) {
        ask.launch(android.Manifest.permission.POST_NOTIFICATIONS)
      }
    }
  }
}

class Pushes(val ask: () -> Unit)

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
 * What the app says over its screens (ADR 0020): the banner at the top, the toasts at the bottom.
 */
@Composable
private fun Overlays() {
  Box(Modifier.fillMaxSize()) {
    BannerView(Modifier.align(Alignment.TopCenter))
    BottomToasts(Modifier.align(Alignment.BottomCenter))
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

/**
 * A visit to the sample world, over the pairing page: its preferences and Devices live apart from
 * the real app, and leaving returns to the same step of pairing.
 */
@Composable
private fun DemoExperience(exit: () -> Unit) {
  val context = LocalContext.current
  val stores = remember { Stores.demo(context, dev.condr.companion.demo.DemoScene.AGENTS) }
  androidx.compose.runtime.DisposableEffect(stores) { onDispose { stores.client.shutdown() } }
  // Pairing always happens in the real app, never against sample Devices.
  LaunchedEffect(stores.router.pairingAnother, stores.session.paired) {
    if (stores.router.pairingAnother || !stores.session.paired) exit()
  }
  androidx.activity.compose.BackHandler(onBack = exit)
  CompositionLocalProvider(LocalStores provides stores) {
    Column(Modifier.fillMaxSize().background(MaterialTheme.colorScheme.surface)) {
      Row(
        verticalAlignment = Alignment.CenterVertically,
        modifier =
          Modifier.fillMaxWidth().statusBarsPadding().padding(horizontal = 20.dp, vertical = 4.dp),
      ) {
        Column(Modifier.weight(1f)) {
          Text(
            "Demo Mode",
            style = MaterialTheme.typography.titleSmall,
            fontWeight = FontWeight.SemiBold,
          )
          Text(
            "Sample data · No live connection",
            style = MaterialTheme.typography.bodySmall,
            color = MaterialTheme.colorScheme.onSurfaceVariant,
          )
        }
        TextButton(onClick = exit) { Text("Exit Demo") }
      }
      Box(Modifier.fillMaxSize()) {
        MainTabs()
        Overlays()
      }
    }
  }
}
