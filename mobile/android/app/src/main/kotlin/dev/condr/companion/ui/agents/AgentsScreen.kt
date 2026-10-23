package dev.condr.companion.ui.agents

import androidx.compose.animation.AnimatedContent
import androidx.compose.animation.fadeIn
import androidx.compose.animation.fadeOut
import androidx.compose.animation.slideInVertically
import androidx.compose.animation.slideOutVertically
import androidx.compose.animation.togetherWith
import androidx.compose.foundation.background
import androidx.compose.foundation.clickable
import androidx.compose.foundation.layout.Arrangement
import androidx.compose.foundation.layout.Box
import androidx.compose.foundation.layout.Column
import androidx.compose.foundation.layout.PaddingValues
import androidx.compose.foundation.layout.Row
import androidx.compose.foundation.layout.WindowInsets
import androidx.compose.foundation.layout.asPaddingValues
import androidx.compose.foundation.layout.fillMaxSize
import androidx.compose.foundation.layout.fillMaxWidth
import androidx.compose.foundation.layout.height
import androidx.compose.foundation.layout.padding
import androidx.compose.foundation.layout.size
import androidx.compose.foundation.layout.statusBars
import androidx.compose.foundation.lazy.LazyColumn
import androidx.compose.foundation.lazy.LazyListScope
import androidx.compose.foundation.lazy.items
import androidx.compose.foundation.lazy.itemsIndexed
import androidx.compose.foundation.lazy.rememberLazyListState
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
import androidx.compose.runtime.derivedStateOf
import androidx.compose.runtime.getValue
import androidx.compose.runtime.remember
import androidx.compose.ui.Alignment
import androidx.compose.ui.Modifier
import androidx.compose.ui.draw.clip
import androidx.compose.ui.graphics.Brush
import androidx.compose.ui.platform.LocalDensity
import androidx.compose.ui.semantics.heading
import androidx.compose.ui.semantics.semantics
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
 * paired Device. A row moves between the groups as its Agent changes state.
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
  val open: (AgentItem) -> Unit = { router.agentsPath.add(DeviceRoute.Pane(it.ref)) }
  val list = rememberLazyListState()
  val density = LocalDensity.current
  // 1 at rest, down to 0.3 as the page scrolls up: the score folds toward its baseline.
  val fold by remember {
    derivedStateOf {
      if (list.firstVisibleItemIndex > 0) 0.3f
      else {
        val lifted = with(density) { list.firstVisibleItemScrollOffset.toDp() }
        maxOf(0.3f, 1f - lifted.value / 150f)
      }
    }
  }
  // The tab names the screen and the score heads it, as on iOS: no bar of its own.
  Scaffold(contentWindowInsets = WindowInsets.statusBars) { insets ->
    Box(Modifier.fillMaxSize()) {
      LazyColumn(
        state = list,
        modifier = Modifier.fillMaxSize(),
        contentPadding =
          PaddingValues(
            top = insets.calculateTopPadding() + 14.dp,
            bottom = insets.calculateBottomPadding() + 24.dp,
            start = 16.dp,
            end = 16.dp,
          ),
      ) {
        item(key = "score") { Score(fold = fold) }
        item(key = "title-needs") {
          SectionTitle(
            "Needs you",
            needsYou.size,
            AgentStatus.BLOCKED.tint,
            Modifier.animateItem().padding(top = 30.dp, bottom = 10.dp),
          )
        }
        if (needsYou.isEmpty()) {
          item(key = "all-clear") { AllClear(working.size, Modifier.animateItem()) }
        } else {
          items(needsYou, key = { "card-${it.key}" }) { item ->
            NeedsYouCard(item, Modifier.animateItem().padding(bottom = 10.dp)) { open(item) }
          }
        }
        group("Done", done, open)
        group("Working", working, open)
        group("Idle", idle, open)
      }
      StatusBarFade()
    }
  }
}

private val AgentItem.key: String
  get() = "${ref.address}#${ref.pane}"

/**
 * A group's rows as one card, each row an item of its own, so a row slides from one group to the
 * next.
 */
private fun LazyListScope.group(title: String, items: List<AgentItem>, open: (AgentItem) -> Unit) {
  if (items.isEmpty()) return
  item(key = "title-$title") {
    SectionTitle(
      title,
      items.size,
      null,
      Modifier.animateItem().padding(top = 30.dp, bottom = 10.dp),
    )
  }
  itemsIndexed(items, key = { _, it -> "row-${it.key}" }) { index, item ->
    val top = if (index == 0) 24.dp else 0.dp
    val bottom = if (index == items.lastIndex) 24.dp else 0.dp
    Column(
      Modifier.animateItem()
        .clip(RoundedCornerShape(top, top, bottom, bottom))
        .background(CardDefaults.cardColors().containerColor)
    ) {
      if (index > 0) HorizontalDivider(Modifier.padding(start = 72.dp))
      AgentRow(item) { open(item) }
    }
  }
}

@Composable
fun SectionTitle(
  title: String,
  count: Int,
  tint: androidx.compose.ui.graphics.Color?,
  modifier: Modifier = Modifier,
) {
  Row(
    verticalAlignment = Alignment.Bottom,
    horizontalArrangement = Arrangement.spacedBy(8.dp),
    modifier = modifier.padding(horizontal = 4.dp).semantics(mergeDescendants = true) { heading() },
  ) {
    Text(title, style = MaterialTheme.typography.titleLarge, fontWeight = FontWeight.SemiBold)
    if (count > 0) {
      // The count ticks to its new value, as iOS's numeric transition does.
      AnimatedContent(
        count,
        transitionSpec = {
          val up = targetState > initialState
          (slideInVertically { if (up) it else -it } + fadeIn()) togetherWith
            (slideOutVertically { if (up) -it else it } + fadeOut())
        },
        label = "count",
      ) { shown ->
        Text(
          "$shown",
          style = MaterialTheme.typography.titleMedium,
          fontWeight = FontWeight.SemiBold,
          color = tint ?: MaterialTheme.colorScheme.onSurfaceVariant,
          modifier = Modifier.padding(bottom = 2.dp),
        )
      }
    }
  }
}

/** With no top bar, content scrolling up fades out under the status bar instead of crossing it. */
@Composable
private fun StatusBarFade() {
  val page = MaterialTheme.colorScheme.background
  val top = WindowInsets.statusBars.asPaddingValues().calculateTopPadding()
  Box(
    Modifier.fillMaxWidth()
      .height(top + 10.dp)
      .background(Brush.verticalGradient(0f to page, 0.55f to page, 1f to page.copy(alpha = 0f)))
  )
}

/** Whether text may take three lines: at the largest font sizes, as iOS's accessibility sizes. */
@Composable private fun lines(): Int = if (LocalDensity.current.fontScale >= 1.5f) 3 else 1

@Composable
private fun NeedsYouCard(item: AgentItem, modifier: Modifier = Modifier, open: () -> Unit) {
  val shape = RoundedCornerShape(24.dp)
  Card(
    shape = shape,
    modifier =
      modifier.fillMaxWidth().clip(shape).clickable(onClickLabel = "Opens the terminal") { open() },
  ) {
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
    AgentAvatar(
      kind = item.agent.kind,
      status = item.agent.status,
      side = side,
      surface = CardDefaults.cardColors().containerColor,
    )
    Column(Modifier.weight(1f), verticalArrangement = Arrangement.spacedBy(2.dp)) {
      Text(item.pane.name, style = titleStyle, maxLines = lines(), overflow = TextOverflow.Ellipsis)
      Text(
        "${item.agent.label} · ${place(item.workspace, item.device)}",
        style = MaterialTheme.typography.bodyMedium,
        color = MaterialTheme.colorScheme.onSurfaceVariant,
        maxLines = lines(),
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
      overflow = TextOverflow.Ellipsis,
    )
  }
}

/** What a Blocked Agent waits for: a tool and its command, or a question. */
@Composable
private fun BlockedOnView(detail: BlockedOn) {
  Column(Modifier.fillMaxWidth()) {
    // A well inset in the card: a shade over the card's colour, so it shows on it.
    androidx.compose.material3.Surface(
      shape = RoundedCornerShape(16.dp),
      color = MaterialTheme.colorScheme.onSurface.copy(alpha = 0.06f),
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
            overflow = TextOverflow.Ellipsis,
          )
        }
      }
    }
  }
}

@Composable
private fun AgentRow(item: AgentItem, open: () -> Unit) {
  Box(
    Modifier.fillMaxWidth().clickable(onClick = open).padding(horizontal = 16.dp, vertical = 12.dp)
  ) {
    AgentHeading(item, side = 40.dp, titleStyle = MaterialTheme.typography.bodyLarge)
  }
}

@Composable
private fun AllClear(working: Int, modifier: Modifier = Modifier) {
  Card(shape = RoundedCornerShape(24.dp), modifier = modifier.fillMaxWidth()) {
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
              MaterialTheme.colorScheme.onSurface.copy(alpha = 0.08f),
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
