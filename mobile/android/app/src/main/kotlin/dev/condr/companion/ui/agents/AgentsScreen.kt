package dev.condr.companion.ui.agents

import androidx.compose.foundation.background
import androidx.compose.foundation.clickable
import androidx.compose.foundation.layout.Arrangement
import androidx.compose.foundation.layout.Box
import androidx.compose.foundation.layout.Column
import androidx.compose.foundation.layout.PaddingValues
import androidx.compose.foundation.layout.Row
import androidx.compose.foundation.layout.Spacer
import androidx.compose.foundation.layout.WindowInsets
import androidx.compose.foundation.layout.fillMaxSize
import androidx.compose.foundation.layout.fillMaxWidth
import androidx.compose.foundation.layout.height
import androidx.compose.foundation.layout.padding
import androidx.compose.foundation.layout.size
import androidx.compose.foundation.layout.statusBars
import androidx.compose.foundation.lazy.LazyColumn
import androidx.compose.foundation.lazy.itemsIndexed
import androidx.compose.foundation.shape.RoundedCornerShape
import androidx.compose.material.icons.Icons
import androidx.compose.material.icons.automirrored.filled.KeyboardArrowRight
import androidx.compose.material.icons.filled.Check
import androidx.compose.material3.Card
import androidx.compose.material3.CardDefaults
import androidx.compose.material3.ExperimentalMaterial3Api
import androidx.compose.material3.HorizontalDivider
import androidx.compose.material3.Icon
import androidx.compose.material3.ListItem
import androidx.compose.material3.ListItemDefaults
import androidx.compose.material3.MaterialTheme
import androidx.compose.material3.Scaffold
import androidx.compose.material3.Text
import androidx.compose.runtime.Composable
import androidx.compose.ui.Alignment
import androidx.compose.ui.Modifier
import androidx.compose.ui.text.font.FontFamily
import androidx.compose.ui.text.font.FontWeight
import androidx.compose.ui.text.style.TextOverflow
import androidx.compose.ui.unit.dp
import dev.condr.companion.model.AgentItem
import dev.condr.companion.model.BlockedOn
import dev.condr.companion.model.DeviceRoute
import dev.condr.companion.model.name
import dev.condr.companion.model.place
import dev.condr.companion.ui.LocalStores
import dev.condr.companion.ui.theme.tint
import uniffi.condr_mobile.AgentStatus

/**
 * The first screen (ADR 0041): "Needs you", then every Agent by what it is doing, across every
 * paired Device.
 */
@OptIn(ExperimentalMaterial3Api::class)
@Composable
fun AgentsScreen() {
  val stores = LocalStores.current
  val session = stores.session
  val router = stores.router
  val needsYou = session.agents(AgentStatus.BLOCKED, AgentStatus.ERROR)
  val done = session.agents(AgentStatus.DONE)
  val working = session.agents(AgentStatus.WORKING)
  val idle = session.agents(AgentStatus.IDLE, AgentStatus.UNKNOWN)
  // The tab names the screen and the score heads it, as on iOS: no bar of its own.
  Scaffold(contentWindowInsets = WindowInsets.statusBars) { insets ->
    LazyColumn(
      modifier = Modifier.fillMaxSize(),
      contentPadding =
        PaddingValues(
          top = insets.calculateTopPadding() + 14.dp,
          bottom = insets.calculateBottomPadding() + 24.dp,
          start = 16.dp,
          end = 16.dp,
        ),
      verticalArrangement = Arrangement.spacedBy(12.dp),
    ) {
      item { Score(Modifier.padding(bottom = 18.dp)) }
      item { SectionTitle("Needs you", needsYou.size, AgentStatus.BLOCKED.tint) }
      if (needsYou.isEmpty()) {
        item { AllClear(working = working.size) }
      } else {
        itemsIndexed(needsYou, key = { _, it -> "${it.ref.address}#${it.ref.pane}" }) { _, item ->
          NeedsYouCard(item) { router.agentsPath.add(DeviceRoute.Pane(item.ref)) }
        }
      }
      group("Done", done) { router.agentsPath.add(DeviceRoute.Pane(it.ref)) }
      group("Working", working) { router.agentsPath.add(DeviceRoute.Pane(it.ref)) }
      group("Idle", idle) { router.agentsPath.add(DeviceRoute.Pane(it.ref)) }
    }
  }
}

private fun androidx.compose.foundation.lazy.LazyListScope.group(
  title: String,
  items: List<AgentItem>,
  open: (AgentItem) -> Unit,
) {
  if (items.isEmpty()) return
  item { Spacer(Modifier.height(12.dp)) }
  item { SectionTitle(title, items.size, null) }
  item {
    Card(shape = RoundedCornerShape(24.dp), colors = CardDefaults.cardColors()) {
      items.forEachIndexed { index, item ->
        AgentRow(item) { open(item) }
        if (index < items.lastIndex) HorizontalDivider(Modifier.padding(start = 72.dp))
      }
    }
  }
}

@Composable
fun SectionTitle(title: String, count: Int, tint: androidx.compose.ui.graphics.Color?) {
  Row(
    verticalAlignment = Alignment.Bottom,
    horizontalArrangement = Arrangement.spacedBy(8.dp),
    modifier = Modifier.padding(horizontal = 4.dp),
  ) {
    Text(title, style = MaterialTheme.typography.titleLarge, fontWeight = FontWeight.SemiBold)
    if (count > 0) {
      Text(
        "$count",
        style = MaterialTheme.typography.titleMedium,
        fontWeight = FontWeight.SemiBold,
        color = tint ?: MaterialTheme.colorScheme.onSurfaceVariant,
        modifier = Modifier.padding(bottom = 2.dp),
      )
    }
  }
}

@Composable
private fun NeedsYouCard(item: AgentItem, open: () -> Unit) {
  Card(onClick = open, shape = RoundedCornerShape(24.dp)) {
    Column(Modifier.padding(16.dp), verticalArrangement = Arrangement.spacedBy(14.dp)) {
      AgentHeading(item, side = 42.dp, titleStyle = MaterialTheme.typography.titleMedium)
      val raw = item.agent.detail
      if (raw != null) {
        if (item.agent.status == AgentStatus.ERROR) Failed(raw) else BlockedOnView(BlockedOn(raw))
      }
    }
  }
}

/** An Agent's avatar, its Pane's title, and "kind · workspace · computer" with a chevron. */
@Composable
fun AgentHeading(
  item: AgentItem,
  side: androidx.compose.ui.unit.Dp,
  titleStyle: androidx.compose.ui.text.TextStyle,
) {
  Row(
    verticalAlignment = Alignment.CenterVertically,
    horizontalArrangement = Arrangement.spacedBy(14.dp),
  ) {
    AgentAvatar(kind = item.agent.kind, status = item.agent.status, side = side)
    Column(Modifier.weight(1f), verticalArrangement = Arrangement.spacedBy(2.dp)) {
      Text(item.pane.name, style = titleStyle, maxLines = 1, overflow = TextOverflow.Ellipsis)
      Text(
        "${item.agent.label} · ${place(item.workspace, item.device)}",
        style = MaterialTheme.typography.bodyMedium,
        color = MaterialTheme.colorScheme.onSurfaceVariant,
        maxLines = 1,
        overflow = TextOverflow.Ellipsis,
      )
    }
    Icon(
      Icons.AutoMirrored.Filled.KeyboardArrowRight,
      contentDescription = null,
      tint = MaterialTheme.colorScheme.outline,
      modifier = Modifier.size(20.dp),
    )
  }
}

/** Why an Error Agent halted, in its own words (ADR 0044). */
@Composable
private fun Failed(text: String) {
  Column(verticalArrangement = Arrangement.spacedBy(6.dp)) {
    Text(
      "FAILED",
      style = MaterialTheme.typography.labelSmall,
      color = MaterialTheme.colorScheme.onSurfaceVariant,
    )
    Text(
      text,
      style = MaterialTheme.typography.bodyLarge,
      fontWeight = FontWeight.Medium,
      maxLines = 3,
    )
  }
}

/** What a Blocked Agent waits for: a tool and its command, or a question. */
@Composable
private fun BlockedOnView(detail: BlockedOn) {
  Column(Modifier.fillMaxWidth()) {
    androidx.compose.material3.Surface(
      shape = RoundedCornerShape(16.dp),
      color = MaterialTheme.colorScheme.surfaceContainerHighest,
      modifier = Modifier.fillMaxWidth(),
    ) {
      Column(
        Modifier.padding(horizontal = 13.dp, vertical = 11.dp),
        verticalArrangement = Arrangement.spacedBy(6.dp),
      ) {
        Text(
          if (detail.tool == null) "ASKS" else "WANTS TO RUN",
          style = MaterialTheme.typography.labelSmall,
          color = MaterialTheme.colorScheme.onSurfaceVariant,
        )
        val tool = detail.tool
        if (tool != null) {
          Row(
            horizontalArrangement = Arrangement.spacedBy(8.dp),
            verticalAlignment = Alignment.Top,
          ) {
            androidx.compose.material3.Surface(
              shape = RoundedCornerShape(5.dp),
              color = MaterialTheme.colorScheme.secondaryContainer,
            ) {
              Text(
                tool,
                style = MaterialTheme.typography.labelMedium,
                color = MaterialTheme.colorScheme.onSecondaryContainer,
                modifier = Modifier.padding(horizontal = 6.dp, vertical = 2.dp),
              )
            }
            Text(
              detail.text,
              style = MaterialTheme.typography.bodyMedium,
              fontFamily = FontFamily.Monospace,
              maxLines = 3,
              overflow = TextOverflow.Ellipsis,
            )
          }
        } else {
          Text(
            detail.text,
            style = MaterialTheme.typography.bodyLarge,
            fontWeight = FontWeight.Medium,
            maxLines = 3,
          )
        }
      }
    }
  }
}

@Composable
private fun AgentRow(item: AgentItem, open: () -> Unit) {
  ListItem(
    headlineContent = { Text(item.pane.name, maxLines = 1, overflow = TextOverflow.Ellipsis) },
    supportingContent = {
      Text(
        "${item.agent.label} · ${place(item.workspace, item.device)}",
        maxLines = 1,
        overflow = TextOverflow.Ellipsis,
      )
    },
    leadingContent = {
      AgentAvatar(kind = item.agent.kind, status = item.agent.status, side = 40.dp)
    },
    trailingContent = {
      Icon(
        Icons.AutoMirrored.Filled.KeyboardArrowRight,
        contentDescription = null,
        tint = MaterialTheme.colorScheme.outline,
        modifier = Modifier.size(20.dp),
      )
    },
    colors =
      ListItemDefaults.colors(containerColor = androidx.compose.ui.graphics.Color.Transparent),
    modifier = Modifier.clickable { open() },
  )
}

@Composable
private fun AllClear(working: Int) {
  Card(shape = RoundedCornerShape(24.dp)) {
    ListItem(
      headlineContent = { Text("Nothing needs you", style = MaterialTheme.typography.titleMedium) },
      supportingContent = {
        Text(
          if (working > 0) "Condr brings an agent here the moment it waits for you."
          else "Every agent is at rest."
        )
      },
      leadingContent = {
        Box(
          Modifier.size(42.dp)
            .background(
              MaterialTheme.colorScheme.surfaceContainerHighest,
              RoundedCornerShape(13.dp),
            ),
          contentAlignment = Alignment.Center,
        ) {
          Icon(
            Icons.Filled.Check,
            contentDescription = null,
            tint = MaterialTheme.colorScheme.onSurfaceVariant,
            modifier = Modifier.size(20.dp),
          )
        }
      },
      colors =
        ListItemDefaults.colors(containerColor = androidx.compose.ui.graphics.Color.Transparent),
    )
  }
}
