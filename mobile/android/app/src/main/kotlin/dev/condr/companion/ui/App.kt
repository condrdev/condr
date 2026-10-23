package dev.condr.companion.ui

import android.content.ClipData
import android.content.ClipboardManager
import android.os.Build
import androidx.activity.compose.BackHandler
import androidx.activity.compose.LocalActivity
import androidx.activity.compose.rememberLauncherForActivityResult
import androidx.activity.result.contract.ActivityResultContracts
import androidx.compose.animation.Crossfade
import androidx.compose.animation.core.RepeatMode
import androidx.compose.animation.core.animateFloat
import androidx.compose.animation.core.infiniteRepeatable
import androidx.compose.animation.core.rememberInfiniteTransition
import androidx.compose.animation.core.tween
import androidx.compose.foundation.background
import androidx.compose.foundation.layout.Arrangement
import androidx.compose.foundation.layout.Box
import androidx.compose.foundation.layout.Column
import androidx.compose.foundation.layout.Row
import androidx.compose.foundation.layout.RowScope
import androidx.compose.foundation.layout.Spacer
import androidx.compose.foundation.layout.fillMaxSize
import androidx.compose.foundation.layout.fillMaxWidth
import androidx.compose.foundation.layout.height
import androidx.compose.foundation.layout.padding
import androidx.compose.foundation.layout.size
import androidx.compose.foundation.layout.statusBarsPadding
import androidx.compose.foundation.layout.systemBarsPadding
import androidx.compose.material.icons.Icons
import androidx.compose.material.icons.filled.Lock
import androidx.compose.material.icons.filled.Settings
import androidx.compose.material.icons.filled.Warning
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
import androidx.compose.runtime.saveable.rememberSaveableStateHolder
import androidx.compose.runtime.setValue
import androidx.compose.runtime.staticCompositionLocalOf
import androidx.compose.ui.Alignment
import androidx.compose.ui.Modifier
import androidx.compose.ui.draw.alpha
import androidx.compose.ui.input.pointer.pointerInput
import androidx.compose.ui.platform.LocalContext
import androidx.compose.ui.platform.LocalFocusManager
import androidx.compose.ui.res.painterResource
import androidx.compose.ui.semantics.clearAndSetSemantics
import androidx.compose.ui.text.font.FontWeight
import androidx.compose.ui.text.style.TextAlign
import androidx.compose.ui.unit.dp
import androidx.lifecycle.Lifecycle
import androidx.lifecycle.compose.LifecycleEventEffect
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
import dev.condr.companion.ui.theme.AppMark
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
    val session = stores.session
    val lock = stores.lock
    // The welcome and its pairing, until they are done; again once every Device is forgotten.
    // Settled at the first look, so pairing from the welcome does not swap the welcome for the
    // tabs before its last step is shown.
    var onboarding by remember(stores) { mutableStateOf(!session.paired) }
    LaunchedEffect(session.paired) { if (!session.paired) onboarding = true }
    var trying by remember { mutableStateOf(false) }
    // What Recents and a returning glance see once paired: the mark, and nothing of the work.
    var shielded by remember { mutableStateOf(false) }
    PushRegistration(asks = session.paired && !onboarding)
    LifecycleEventEffect(Lifecycle.Event.ON_RESUME) {
      // The app lock (ADR 0041): on start, and after five minutes away.
      lock.sceneBecameActive(session.paired)
      shielded = false
    }
    LifecycleEventEffect(Lifecycle.Event.ON_PAUSE) {
      // A lock already hides it all, and its own prompt pauses the app.
      if (session.paired && !lock.locked) shielded = true
    }
    LifecycleEventEffect(Lifecycle.Event.ON_STOP) { lock.sceneLeft() }
    val covered = lock.locked || shielded
    Box(Modifier.fillMaxSize()) {
      // Under the lock or the shield, TalkBack reads only them.
      Box(
        Modifier.fillMaxSize().then(if (covered) Modifier.clearAndSetSemantics {} else Modifier)
      ) {
        Crossfade(onboarding, animationSpec = tween(600), label = "onboarding") { welcome ->
          if (welcome) {
            PairingFlow(
              firstLaunch = true,
              finished = { onboarding = false },
              tryDemo = { trying = true },
            )
          } else MainTabs()
        }
        if (trying) DemoExperience(exit = { trying = false })
        Overlays()
        // Pairing another computer covers the banner and the toasts, as iOS's sheet does.
        if (stores.router.pairingAnother) {
          Box(Modifier.fillMaxSize().background(MaterialTheme.colorScheme.surface)) {
            PairingFlow(firstLaunch = false, finished = { stores.router.pairingAnother = false })
          }
        }
      }
      if (lock.locked) LockScreen() else if (shielded) Shield()
    }
  }
}

@Composable
private fun MainTabs() {
  val stores = LocalStores.current
  val router = stores.router
  val session = stores.session
  val tabs = rememberSaveableStateHolder()
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
        TabItem(AppTab.AGENTS, "Agents") {
          BadgedBox(badge = { if (needsYou > 0) Badge { Text("$needsYou") } }) {
            Icon(painterResource(R.drawable.ic_tab_agents), contentDescription = null)
          }
        }
        TabItem(AppTab.DEVICES, "Devices") {
          Icon(painterResource(R.drawable.ic_tab_devices), contentDescription = null)
        }
        TabItem(AppTab.SETTINGS, "Settings") {
          Icon(Icons.Filled.Settings, contentDescription = null)
        }
      }
    }
  ) { insets ->
    // A Pane's page draws to the bottom edge and keeps its own insets.
    Box(
      Modifier.fillMaxSize().padding(bottom = if (inPane) 0.dp else insets.calculateBottomPadding())
    ) {
      // Each tab keeps its screens' state, scroll included, while another shows.
      tabs.SaveableStateProvider(router.tab.name) {
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
}

/** A tab; choosing the one already showing goes back to its first screen. */
@Composable
private fun RowScope.TabItem(tab: AppTab, label: String, icon: @Composable () -> Unit) {
  val router = LocalStores.current.router
  NavigationBarItem(
    selected = router.tab == tab,
    onClick = {
      if (router.tab != tab) router.tab = tab
      else
        when (tab) {
          AppTab.AGENTS -> router.agentsPath.clear()
          AppTab.DEVICES -> router.devicesPath.clear()
          AppTab.SETTINGS -> router.settingsPath.clear()
        }
    },
    icon = icon,
    label = { Text(label) },
  )
}

/**
 * Keeps Condr Cloud told what this phone accepts (ADR 0047): the permission as it stands, read
 * again on every return, the Devices paired, and FCM's token, fetched once per launch. A
 * registration that failed is tried again on every return. The system's prompt shows once there is
 * a Device to be told about and the first pairing is done with, and never again once answered.
 */
@Composable
private fun PushRegistration(asks: Boolean) {
  val stores = LocalStores.current
  val push = stores.push
  val preferences = stores.preferences
  val context = LocalContext.current
  var granted by remember { mutableStateOf(Notifications.granted(context)) }
  var returns by remember { mutableStateOf(0) }
  val ask =
    rememberLauncherForActivityResult(ActivityResultContracts.RequestPermission()) { granted = it }
  val servers = stores.session.devices.mapNotNull { it.deviceId }
  LifecycleEventEffect(Lifecycle.Event.ON_RESUME) {
    granted = Notifications.granted(context)
    returns += 1
  }
  LaunchedEffect(push) {
    val push = push ?: return@LaunchedEffect
    Notifications.fetchToken(context, received = { push.received(it) }, failed = { push.failed() })
  }
  LaunchedEffect(push, granted, servers, returns) {
    android.util.Log.i(
      "condr.push",
      "update granted=$granted servers=${servers.size} push=${push != null}",
    )
    push?.update(granted, servers)
  }
  LaunchedEffect(asks, push) {
    if (
      asks &&
        push != null &&
        !preferences.pushAsked &&
        !granted &&
        Build.VERSION.SDK_INT >= Build.VERSION_CODES.TIRAMISU
    ) {
      preferences.pushAsked = true
      ask.launch(android.Manifest.permission.POST_NOTIFICATIONS)
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
 * What the app says over its screens (ADR 0020): the banner at the top, the toasts at the bottom.
 */
@Composable
private fun Overlays() {
  Box(Modifier.fillMaxSize()) {
    BannerView(Modifier.align(Alignment.TopCenter))
    BottomToasts(Modifier.align(Alignment.BottomCenter))
  }
}

/**
 * The lock over everything (ADR 0041): it asks as it appears, and again on the button. Nothing
 * beneath it takes a touch, Back or the keyboard.
 */
@Composable
private fun LockScreen() {
  val lock = LocalStores.current.lock
  val scope = rememberCoroutineScope()
  val focus = LocalFocusManager.current
  val activity = LocalActivity.current
  LaunchedEffect(Unit) {
    focus.clearFocus(force = true)
    if (lock.promptsOnAppear) lock.unlock()
  }
  BackHandler { activity?.moveTaskToBack(true) }
  val pulse by
    rememberInfiniteTransition(label = "asking")
      .animateFloat(1f, 0.4f, infiniteRepeatable(tween(700), RepeatMode.Reverse), label = "pulse")
  Cover {
    Column(
      Modifier.fillMaxSize().systemBarsPadding().padding(horizontal = 28.dp),
      horizontalAlignment = Alignment.CenterHorizontally,
    ) {
      Spacer(Modifier.weight(1f))
      AppMark(80.dp)
      Text(
        "Condr is locked",
        style = MaterialTheme.typography.headlineSmall,
        fontWeight = FontWeight.Bold,
        modifier = Modifier.padding(top = 24.dp),
      )
      Text(
        "Unlock to reach your computers.",
        color = MaterialTheme.colorScheme.onSurfaceVariant,
        modifier = Modifier.padding(top = 6.dp),
      )
      Icon(
        Icons.Filled.Lock,
        contentDescription = null,
        tint =
          if (lock.authenticating) MaterialTheme.colorScheme.primary
          else MaterialTheme.colorScheme.onSurfaceVariant,
        modifier =
          Modifier.padding(top = 44.dp).size(44.dp).alpha(if (lock.authenticating) pulse else 1f),
      )
      Spacer(Modifier.weight(1f))
      Button(
        onClick = { scope.launch { lock.unlock() } },
        enabled = !lock.authenticating,
        modifier = Modifier.fillMaxWidth().height(56.dp),
      ) {
        Text("Unlock", style = MaterialTheme.typography.titleMedium)
      }
      Spacer(Modifier.height(12.dp))
    }
  }
}

/** What Recents and a returning glance see: the mark, and nothing of the work. */
@Composable
private fun Shield() {
  Cover { Box(Modifier.fillMaxSize(), contentAlignment = Alignment.Center) { AppMark(80.dp) } }
}

/** A surface over the whole app that keeps every touch from what is beneath. */
@Composable
private fun Cover(content: @Composable () -> Unit) {
  Box(
    Modifier.fillMaxSize().background(MaterialTheme.colorScheme.surface).pointerInput(Unit) {
      awaitPointerEventScope { while (true) awaitPointerEvent().changes.forEach { it.consume() } }
    }
  ) {
    content()
  }
}

/** The Device key could not be kept: nothing can be paired without it. */
@Composable
fun StartupFailure(message: String) {
  val context = LocalContext.current
  Column(
    Modifier.fillMaxSize()
      .background(MaterialTheme.colorScheme.surface)
      .systemBarsPadding()
      .padding(32.dp),
    verticalArrangement = Arrangement.Center,
    horizontalAlignment = Alignment.CenterHorizontally,
  ) {
    Icon(
      Icons.Filled.Warning,
      contentDescription = null,
      tint = MaterialTheme.colorScheme.onSurfaceVariant,
      modifier = Modifier.size(44.dp),
    )
    Text(
      "Condr can't start",
      style = MaterialTheme.typography.headlineSmall,
      fontWeight = FontWeight.Bold,
      modifier = Modifier.padding(top = 20.dp),
    )
    Text(
      message,
      color = MaterialTheme.colorScheme.onSurfaceVariant,
      textAlign = TextAlign.Center,
      modifier = Modifier.padding(top = 10.dp),
    )
    TextButton(
      onClick = {
        context
          .getSystemService(ClipboardManager::class.java)
          .setPrimaryClip(ClipData.newPlainText("Condr", message))
      },
      modifier = Modifier.padding(top = 12.dp),
    ) {
      Text("Copy error")
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
            "Demo mode",
            style = MaterialTheme.typography.titleSmall,
            fontWeight = FontWeight.SemiBold,
          )
          Text(
            "Sample data · No live connection",
            style = MaterialTheme.typography.bodySmall,
            color = MaterialTheme.colorScheme.onSurfaceVariant,
          )
        }
        TextButton(onClick = exit) { Text("Exit demo") }
      }
      Box(Modifier.fillMaxSize()) {
        MainTabs()
        Overlays()
      }
    }
  }
}
