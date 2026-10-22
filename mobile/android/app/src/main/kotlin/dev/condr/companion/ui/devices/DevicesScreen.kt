package dev.condr.companion.ui.devices

import androidx.compose.foundation.clickable
import androidx.compose.foundation.layout.fillMaxSize
import androidx.compose.foundation.layout.padding
import androidx.compose.foundation.lazy.LazyColumn
import androidx.compose.foundation.lazy.items
import androidx.compose.material.icons.Icons
import androidx.compose.material.icons.automirrored.filled.KeyboardArrowRight
import androidx.compose.material.icons.filled.Add
import androidx.compose.material3.ExperimentalMaterial3Api
import androidx.compose.material3.FloatingActionButton
import androidx.compose.material3.Icon
import androidx.compose.material3.LargeTopAppBar
import androidx.compose.material3.ListItem
import androidx.compose.material3.MaterialTheme
import androidx.compose.material3.Scaffold
import androidx.compose.material3.Text
import androidx.compose.material3.TopAppBarDefaults
import androidx.compose.runtime.Composable
import androidx.compose.ui.Modifier
import androidx.compose.ui.input.nestedscroll.nestedScroll
import androidx.compose.ui.unit.dp
import dev.condr.companion.model.DeviceRoute
import dev.condr.companion.model.name
import dev.condr.companion.ui.LocalStores
import uniffi.condr_mobile.DeviceStatus
import uniffi.condr_mobile.DeviceView

/** Every paired Device and how it is reached, with a way to add another (ADR 0040). */
@OptIn(ExperimentalMaterial3Api::class)
@Composable
fun DevicesScreen() {
  val stores = LocalStores.current
  val router = stores.router
  val scroll = TopAppBarDefaults.exitUntilCollapsedScrollBehavior()
  Scaffold(
    modifier = Modifier.nestedScroll(scroll.nestedScrollConnection),
    topBar = { LargeTopAppBar(title = { Text("Devices") }, scrollBehavior = scroll) },
    floatingActionButton = {
      FloatingActionButton(onClick = { router.pairingAnother = true }) {
        Icon(Icons.Filled.Add, contentDescription = "Add a computer")
      }
    },
  ) { insets ->
    LazyColumn(Modifier.fillMaxSize().padding(insets)) {
      items(stores.session.devices, key = { it.address }) { device ->
        ListItem(
          headlineContent = { Text(device.label) },
          supportingContent = { Text(device.status.description) },
          trailingContent = {
            Icon(
              Icons.AutoMirrored.Filled.KeyboardArrowRight,
              contentDescription = null,
              tint = MaterialTheme.colorScheme.outline,
            )
          },
          modifier =
            Modifier.clickable { router.devicesPath.add(DeviceRoute.Device(device.address)) },
        )
      }
    }
  }
}

val DeviceStatus.description: String
  get() =
    when (this) {
      is DeviceStatus.Connected -> "Connected"
      is DeviceStatus.Connecting -> "Connecting…"
      is DeviceStatus.Suspended -> "Not connected"
      is DeviceStatus.Failed -> reason
      is DeviceStatus.Revoked -> "This phone was revoked there"
    }

/** A Device's Workspaces and their Panes, each opening its terminal (ADR 0041). */
@OptIn(ExperimentalMaterial3Api::class)
@Composable
fun DeviceScreen(address: String) {
  val stores = LocalStores.current
  val session = stores.session
  val router = stores.router
  val device: DeviceView? = session.device(address)
  val path =
    if (router.tab == dev.condr.companion.model.AppTab.AGENTS) router.agentsPath
    else router.devicesPath
  Scaffold(topBar = { LargeTopAppBar(title = { Text(device?.label ?: "Computer") }) }) { insets ->
    LazyColumn(Modifier.fillMaxSize().padding(insets)) {
      for (workspace in session.workspaces[address] ?: emptyList()) {
        item(key = "w${workspace.id}") {
          Text(
            workspace.name,
            style = MaterialTheme.typography.titleMedium,
            modifier = Modifier.padding(horizontal = 16.dp, vertical = 8.dp),
          )
        }
        items(workspace.panes, key = { "p${it.id}" }) { pane ->
          ListItem(
            headlineContent = { Text(pane.name) },
            supportingContent = { pane.agent?.let { Text(it.label) } },
            modifier =
              Modifier.clickable {
                path.add(DeviceRoute.Pane(dev.condr.companion.model.PaneRef(address, pane.id)))
              },
          )
        }
      }
    }
  }
}
