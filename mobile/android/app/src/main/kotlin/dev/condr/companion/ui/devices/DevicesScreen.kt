package dev.condr.companion.ui.devices

import android.content.ClipData
import android.content.ClipboardManager
import androidx.compose.foundation.background
import androidx.compose.foundation.clickable
import androidx.compose.foundation.layout.Arrangement
import androidx.compose.foundation.layout.Box
import androidx.compose.foundation.layout.BoxWithConstraints
import androidx.compose.foundation.layout.Column
import androidx.compose.foundation.layout.Row
import androidx.compose.foundation.layout.Spacer
import androidx.compose.foundation.layout.fillMaxSize
import androidx.compose.foundation.layout.fillMaxWidth
import androidx.compose.foundation.layout.height
import androidx.compose.foundation.layout.padding
import androidx.compose.foundation.layout.size
import androidx.compose.foundation.lazy.LazyColumn
import androidx.compose.foundation.lazy.items
import androidx.compose.foundation.shape.RoundedCornerShape
import androidx.compose.material.icons.Icons
import androidx.compose.material.icons.automirrored.filled.KeyboardArrowRight
import androidx.compose.material.icons.filled.MoreVert
import androidx.compose.material.icons.filled.Notifications
import androidx.compose.material3.AlertDialog
import androidx.compose.material3.Button
import androidx.compose.material3.CircularProgressIndicator
import androidx.compose.material3.DropdownMenu
import androidx.compose.material3.DropdownMenuItem
import androidx.compose.material3.ExperimentalMaterial3Api
import androidx.compose.material3.HorizontalDivider
import androidx.compose.material3.Icon
import androidx.compose.material3.IconButton
import androidx.compose.material3.ListItem
import androidx.compose.material3.ListItemDefaults
import androidx.compose.material3.MaterialTheme
import androidx.compose.material3.MediumFlexibleTopAppBar
import androidx.compose.material3.OutlinedTextField
import androidx.compose.material3.Scaffold
import androidx.compose.material3.Text
import androidx.compose.material3.TextButton
import androidx.compose.material3.TopAppBarDefaults
import androidx.compose.runtime.Composable
import androidx.compose.runtime.getValue
import androidx.compose.runtime.mutableStateOf
import androidx.compose.runtime.remember
import androidx.compose.runtime.setValue
import androidx.compose.ui.Alignment
import androidx.compose.ui.Modifier
import androidx.compose.ui.graphics.Color
import androidx.compose.ui.input.nestedscroll.nestedScroll
import androidx.compose.ui.platform.LocalContext
import androidx.compose.ui.platform.LocalDensity
import androidx.compose.ui.res.painterResource
import androidx.compose.ui.text.font.FontWeight
import androidx.compose.ui.text.style.TextAlign
import androidx.compose.ui.text.style.TextOverflow
import androidx.compose.ui.unit.Dp
import androidx.compose.ui.unit.dp
import dev.condr.companion.R
import dev.condr.companion.model.AppTab
import dev.condr.companion.model.DeviceRoute
import dev.condr.companion.model.PaneRef
import dev.condr.companion.model.name
import dev.condr.companion.ui.LocalStores
import dev.condr.companion.ui.agents.AgentAvatar
import dev.condr.companion.ui.theme.Bars
import dev.condr.companion.ui.theme.Fallen
import dev.condr.companion.ui.theme.WorkspaceAvatar
import dev.condr.companion.ui.theme.agentStatusLine
import dev.condr.companion.ui.theme.agentSummary
import dev.condr.companion.ui.theme.fitBars
import uniffi.condr_mobile.AgentStatus
import uniffi.condr_mobile.DeviceStatus
import uniffi.condr_mobile.DeviceView
import uniffi.condr_mobile.PaneView

/**
 * Every paired Device and, inside one, its Workspaces and Panes (ADR 0041). Nothing here changes
 * Session structure: the phone opens Panes, it never creates or closes them.
 */
@OptIn(ExperimentalMaterial3Api::class)
@Composable
fun DevicesScreen() {
  val stores = LocalStores.current
  val router = stores.router
  val scroll = TopAppBarDefaults.exitUntilCollapsedScrollBehavior()
  Scaffold(
    modifier = Modifier.nestedScroll(scroll.nestedScrollConnection),
    topBar = { MediumFlexibleTopAppBar(title = { Text("Devices") }, scrollBehavior = scroll) },
  ) { insets ->
    LazyColumn(Modifier.fillMaxSize().padding(insets)) {
      items(stores.session.devices, key = { it.address }) { device ->
        DeviceRow(device) { router.devicesPath.add(DeviceRoute.Device(device.address)) }
      }
      item {
        HorizontalDivider(Modifier.padding(horizontal = 16.dp, vertical = 8.dp))
        ListItem(
          headlineContent = { Text("Pair a computer", color = MaterialTheme.colorScheme.primary) },
          leadingContent = {
            Icon(
              painterResource(R.drawable.ic_qr_scanner),
              contentDescription = null,
              tint = MaterialTheme.colorScheme.primary,
            )
          },
          modifier = Modifier.clickable { router.pairingAnother = true },
        )
      }
    }
  }
}

/**
 * A computer drawn as its own score: its Agents' bars on a tile, so the Devices list speaks the
 * first screen's language and green or red never stand for a connection. A computer it can't reach
 * is the score's dashed placeholder.
 */
@Composable
fun DeviceTile(device: DeviceView, side: Dp = 44.dp) {
  val session = LocalStores.current.session
  val statuses =
    session
      .agents(device)
      .map { it.agent.status }
      .sortedBy { dev.condr.companion.model.urgencyOf(it) }
  Box(
    Modifier.size(side)
      .background(
        MaterialTheme.colorScheme.surfaceContainerHighest,
        RoundedCornerShape(side * 0.28f),
      ),
    contentAlignment = Alignment.Center,
  ) {
    when (device.status) {
      is DeviceStatus.Connected ->
        if (statuses.isEmpty()) {
          Icon(
            painterResource(R.drawable.ic_tab_devices),
            contentDescription = null,
            tint = MaterialTheme.colorScheme.onSurfaceVariant,
            modifier = Modifier.size(side * 0.45f),
          )
        } else {
          val density = LocalDensity.current
          val (w, gap) =
            with(density) {
              fitBars(
                statuses.size,
                (side * 0.66f).toPx(),
                (side * 0.13f).toPx(),
                (side * 0.06f).toPx(),
              )
            }
          Bars(
            statuses,
            tall = side * 0.5f,
            widest = with(density) { w.toDp() },
            spacing = with(density) { gap.toDp() },
            animated = false,
          )
        }
      is DeviceStatus.Connecting,
      is DeviceStatus.Suspended ->
        CircularProgressIndicator(Modifier.size(side * 0.4f), strokeWidth = 2.dp)
      is DeviceStatus.Failed,
      is DeviceStatus.Revoked -> Fallen(width = side * 0.6f)
    }
  }
}

/** A computer: its score as the tile, its name, and its Agents in words. */
@Composable
private fun DeviceRow(device: DeviceView, open: () -> Unit) {
  val session = LocalStores.current.session
  ListItem(
    headlineContent = {
      Text(
        device.label,
        fontWeight = FontWeight.Medium,
        maxLines = 1,
        overflow = TextOverflow.Ellipsis,
      )
    },
    supportingContent = {
      val statuses = session.agents(device).map { it.agent.status }
      Text(
        when (device.status) {
          is DeviceStatus.Connected -> agentSummary(statuses)
          is DeviceStatus.Connecting,
          is DeviceStatus.Suspended -> androidx.compose.ui.text.AnnotatedString("Connecting…")
          is DeviceStatus.Failed -> androidx.compose.ui.text.AnnotatedString("Can't connect")
          is DeviceStatus.Revoked -> androidx.compose.ui.text.AnnotatedString("Needs pairing again")
        },
        maxLines = 1,
        overflow = TextOverflow.Ellipsis,
      )
    },
    leadingContent = { DeviceTile(device) },
    trailingContent = {
      Icon(
        Icons.AutoMirrored.Filled.KeyboardArrowRight,
        contentDescription = null,
        tint = MaterialTheme.colorScheme.outline,
      )
    },
    modifier = Modifier.clickable(onClick = open),
  )
}

/** A Device's Workspaces and their Panes, each opening its terminal (ADR 0041). */
@OptIn(ExperimentalMaterial3Api::class)
@Composable
fun DeviceScreen(address: String) {
  val stores = LocalStores.current
  val session = stores.session
  val router = stores.router
  val context = LocalContext.current
  // Forgotten while this screen was up: the stack is about to drop it.
  val device = session.device(address) ?: return
  val path = if (router.tab == AppTab.AGENTS) router.agentsPath else router.devicesPath
  var menu by remember { mutableStateOf(false) }
  var renaming by remember { mutableStateOf(false) }
  var forgetting by remember { mutableStateOf(false) }
  var newLabel by remember { mutableStateOf("") }
  val scroll = TopAppBarDefaults.exitUntilCollapsedScrollBehavior()
  Scaffold(
    modifier = Modifier.nestedScroll(scroll.nestedScrollConnection),
    topBar = {
      MediumFlexibleTopAppBar(
        title = { Text(device.label, maxLines = 1, overflow = TextOverflow.Ellipsis) },
        scrollBehavior = scroll,
        actions = {
          IconButton(onClick = { menu = true }) {
            Icon(Icons.Filled.MoreVert, contentDescription = "More")
          }
          DropdownMenu(expanded = menu, onDismissRequest = { menu = false }) {
            DropdownMenuItem(
              text = { Text("Rename…") },
              onClick = {
                menu = false
                newLabel = device.label
                renaming = true
              },
            )
            DropdownMenuItem(
              text = { Text("Forget ${device.label}") },
              onClick = {
                menu = false
                forgetting = true
              },
            )
          }
        },
      )
    },
  ) { insets ->
    Box(Modifier.fillMaxSize().padding(insets)) {
      when (val status = device.status) {
        is DeviceStatus.Connected -> Workspaces(device, path)
        is DeviceStatus.Connecting,
        is DeviceStatus.Suspended ->
          Column(
            Modifier.fillMaxSize(),
            verticalArrangement = Arrangement.Center,
            horizontalAlignment = Alignment.CenterHorizontally,
          ) {
            CircularProgressIndicator()
            Spacer(Modifier.height(16.dp))
            Text(
              "Connecting to ${device.label}…",
              color = MaterialTheme.colorScheme.onSurfaceVariant,
            )
          }
        is DeviceStatus.Failed ->
          Gone(title = "Can't reach ${device.label}", description = status.reason) {
            Button(onClick = { session.resume() }) { Text("Try again") }
            TextButton(
              onClick = {
                context
                  .getSystemService(ClipboardManager::class.java)
                  .setPrimaryClip(ClipData.newPlainText("Condr", status.reason))
              }
            ) {
              Text("Copy error")
            }
          }
        is DeviceStatus.Revoked ->
          Gone(
            title = "${device.label} doesn't know this phone",
            description =
              "It was removed from Paired devices on ${device.label}. Create an invite there and scan it to connect again.",
          ) {
            Button(onClick = { router.pairingAnother = true }) { Text("Pair again") }
          }
      }
    }
  }
  if (renaming) {
    AlertDialog(
      onDismissRequest = { renaming = false },
      title = { Text("Rename ${device.label}") },
      text = {
        OutlinedTextField(
          value = newLabel,
          onValueChange = { newLabel = it },
          singleLine = true,
          label = { Text("Name") },
        )
      },
      confirmButton = {
        TextButton(
          enabled = newLabel.isNotBlank(),
          onClick = {
            session.rename(address, newLabel)
            renaming = false
          },
        ) {
          Text("Rename")
        }
      },
      dismissButton = { TextButton(onClick = { renaming = false }) { Text("Cancel") } },
    )
  }
  if (forgetting) {
    AlertDialog(
      onDismissRequest = { forgetting = false },
      title = { Text("Forget ${device.label}?") },
      text = {
        Text(
          "This phone stops connecting to it. To revoke this phone as well, remove it under Paired devices in Condr's Settings on ${device.label}."
        )
      },
      confirmButton = {
        TextButton(
          onClick = {
            forgetting = false
            session.forget(address)
          }
        ) {
          Text("Forget", color = MaterialTheme.colorScheme.error)
        }
      },
      dismissButton = { TextButton(onClick = { forgetting = false }) { Text("Cancel") } },
    )
  }
}

@Composable
private fun Workspaces(device: DeviceView, path: MutableList<DeviceRoute>) {
  val session = LocalStores.current.session
  val statuses = session.agents(device).map { it.agent.status }
  val density = LocalDensity.current
  LazyColumn(
    Modifier.fillMaxSize(),
    contentPadding = androidx.compose.foundation.layout.PaddingValues(bottom = 24.dp),
  ) {
    if (statuses.isNotEmpty()) {
      // The home row's small score, grown: this computer at a glance.
      item {
        Column(
          Modifier.padding(horizontal = 20.dp, vertical = 8.dp),
          verticalArrangement = Arrangement.spacedBy(12.dp),
        ) {
          BoxWithConstraints(
            Modifier.fillMaxWidth().height(64.dp),
            contentAlignment = Alignment.BottomStart,
          ) {
            val (w, gap) =
              with(density) { fitBars(statuses.size, maxWidth.toPx(), 16.dp.toPx(), 7.dp.toPx()) }
            Bars(
              statuses.sortedBy { dev.condr.companion.model.urgencyOf(it) },
              tall = 64.dp,
              widest = with(density) { w.toDp() },
              spacing = with(density) { gap.toDp() },
            )
          }
          Text(
            agentSummary(statuses),
            style = MaterialTheme.typography.bodyMedium,
            fontWeight = FontWeight.Medium,
            color = MaterialTheme.colorScheme.onSurfaceVariant,
          )
        }
      }
    }
    for (workspace in session.workspaces[device.address] ?: emptyList()) {
      item(key = "w${workspace.id}") {
        Row(
          verticalAlignment = Alignment.CenterVertically,
          horizontalArrangement = Arrangement.spacedBy(9.dp),
          modifier = Modifier.padding(start = 16.dp, end = 16.dp, top = 20.dp, bottom = 6.dp),
        ) {
          WorkspaceAvatar(workspace.name)
          Text(
            workspace.name,
            style = MaterialTheme.typography.titleSmall,
            fontWeight = FontWeight.SemiBold,
          )
        }
      }
      items(workspace.panes, key = { "p${it.id}" }) { pane ->
        PaneRow(pane) { path.add(DeviceRoute.Pane(PaneRef(device.address, pane.id))) }
      }
    }
  }
}

/** The head of a computer it can't reach: the score's fallen bars over what went wrong. */
@Composable
private fun Gone(title: String, description: String, actions: @Composable () -> Unit) {
  Column(
    Modifier.fillMaxSize().padding(horizontal = 32.dp),
    verticalArrangement = Arrangement.Center,
    horizontalAlignment = Alignment.CenterHorizontally,
  ) {
    Fallen(width = 96.dp)
    Spacer(Modifier.height(22.dp))
    Text(
      title,
      style = MaterialTheme.typography.headlineSmall,
      fontWeight = FontWeight.Bold,
      textAlign = TextAlign.Center,
    )
    Spacer(Modifier.height(10.dp))
    Text(
      description,
      style = MaterialTheme.typography.bodyMedium,
      color = MaterialTheme.colorScheme.onSurfaceVariant,
      textAlign = TextAlign.Center,
    )
    Spacer(Modifier.height(20.dp))
    Column(
      horizontalAlignment = Alignment.CenterHorizontally,
      verticalArrangement = Arrangement.spacedBy(4.dp),
    ) {
      actions()
    }
  }
}

@Composable
private fun PaneRow(pane: PaneView, open: () -> Unit) {
  val agent = pane.agent
  ListItem(
    headlineContent = { Text(pane.name, maxLines = 1, overflow = TextOverflow.Ellipsis) },
    supportingContent = {
      Text(
        when {
          agent != null -> agentStatusLine(agent.label, agent.status, agent.detail)
          pane.exited -> androidx.compose.ui.text.AnnotatedString("Exited")
          else -> androidx.compose.ui.text.AnnotatedString("Shell")
        },
        maxLines = 1,
        overflow = TextOverflow.Ellipsis,
      )
    },
    leadingContent = {
      AgentAvatar(
        kind = agent?.kind,
        status = agent?.status ?: AgentStatus.UNKNOWN,
        side = 36.dp,
        shell = agent == null,
      )
    },
    trailingContent = {
      if (pane.attention) {
        Icon(
          Icons.Filled.Notifications,
          contentDescription = "Rang the bell",
          tint = MaterialTheme.colorScheme.onSurfaceVariant,
          modifier = Modifier.size(18.dp),
        )
      }
    },
    colors = ListItemDefaults.colors(containerColor = Color.Transparent),
    modifier = Modifier.clickable(onClick = open),
  )
}
