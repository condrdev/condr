package dev.condr.companion.ui

import android.content.ClipData
import android.content.ClipboardManager
import androidx.compose.animation.AnimatedVisibility
import androidx.compose.animation.fadeIn
import androidx.compose.animation.fadeOut
import androidx.compose.animation.slideInVertically
import androidx.compose.animation.slideOutVertically
import androidx.compose.foundation.clickable
import androidx.compose.foundation.gestures.detectVerticalDragGestures
import androidx.compose.foundation.layout.Arrangement
import androidx.compose.foundation.layout.Column
import androidx.compose.foundation.layout.Row
import androidx.compose.foundation.layout.fillMaxWidth
import androidx.compose.foundation.layout.padding
import androidx.compose.foundation.layout.size
import androidx.compose.foundation.layout.statusBarsPadding
import androidx.compose.foundation.shape.RoundedCornerShape
import androidx.compose.material.icons.Icons
import androidx.compose.material.icons.filled.Close
import androidx.compose.material.icons.filled.Warning
import androidx.compose.material3.Icon
import androidx.compose.material3.IconButton
import androidx.compose.material3.MaterialTheme
import androidx.compose.material3.Surface
import androidx.compose.material3.Text
import androidx.compose.material3.TextButton
import androidx.compose.material3.surfaceColorAtElevation
import androidx.compose.runtime.Composable
import androidx.compose.runtime.LaunchedEffect
import androidx.compose.runtime.getValue
import androidx.compose.runtime.mutableStateOf
import androidx.compose.runtime.remember
import androidx.compose.ui.Alignment
import androidx.compose.ui.Modifier
import androidx.compose.ui.hapticfeedback.HapticFeedbackType
import androidx.compose.ui.input.pointer.pointerInput
import androidx.compose.ui.platform.LocalContext
import androidx.compose.ui.platform.LocalHapticFeedback
import androidx.compose.ui.text.font.FontWeight
import androidx.compose.ui.text.style.TextOverflow
import androidx.compose.ui.unit.dp
import dev.condr.companion.model.Banner
import dev.condr.companion.ui.agents.AgentAvatar
import dev.condr.companion.ui.theme.tint
import uniffi.condr_mobile.AgentStatus

/**
 * An Agent elsewhere started to need you or finished, while you look at something else: a card at
 * the top, as on iOS, that opens its terminal and leaves on a swipe up.
 */
@Composable
fun BannerView(modifier: Modifier = Modifier) {
  val stores = LocalStores.current
  val alerts = stores.alerts
  val banner = alerts.banner
  val visible =
    banner != null && !banner.quiet && stores.session.shown != banner.ref && !stores.lock.locked
  val last = remember { mutableStateOf(banner) }
  if (banner != null) last.value = banner
  // Every banner is felt, a quiet one too, as iOS's warning, error and success.
  val haptics = LocalHapticFeedback.current
  LaunchedEffect(banner?.id) {
    when (banner?.status) {
      AgentStatus.BLOCKED -> haptics.performHapticFeedback(HapticFeedbackType.LongPress)
      AgentStatus.ERROR -> haptics.performHapticFeedback(HapticFeedbackType.Reject)
      AgentStatus.DONE -> haptics.performHapticFeedback(HapticFeedbackType.Confirm)
      else -> {}
    }
  }
  AnimatedVisibility(
    visible = visible,
    enter = slideInVertically { -it } + fadeIn(),
    exit = slideOutVertically { -it } + fadeOut(),
    modifier = modifier.statusBarsPadding().padding(horizontal = 10.dp, vertical = 6.dp),
  ) {
    val shown: Banner = last.value ?: return@AnimatedVisibility
    Surface(
      shape = RoundedCornerShape(28.dp),
      tonalElevation = 6.dp,
      shadowElevation = 8.dp,
      modifier =
        Modifier.fillMaxWidth()
          .pointerInput(shown.id) {
            detectVerticalDragGestures { _, delta -> if (delta < -4) alerts.dismissBanner() }
          }
          .clickable(onClickLabel = "Opens its terminal") {
            alerts.dismissBanner()
            stores.router.open(shown.ref)
          },
    ) {
      Row(
        verticalAlignment = Alignment.CenterVertically,
        horizontalArrangement = Arrangement.spacedBy(12.dp),
        modifier = Modifier.padding(start = 12.dp, end = 18.dp, top = 11.dp, bottom = 11.dp),
      ) {
        AgentAvatar(
          kind = shown.kind,
          status = shown.status,
          side = 38.dp,
          surface = MaterialTheme.colorScheme.surfaceColorAtElevation(6.dp),
          ripple = true,
        )
        Column(Modifier.weight(1f), verticalArrangement = Arrangement.spacedBy(1.dp)) {
          Text(
            shown.title,
            style = MaterialTheme.typography.titleSmall,
            fontWeight = FontWeight.SemiBold,
          )
          Text(
            shown.detail,
            style = MaterialTheme.typography.bodySmall,
            color = MaterialTheme.colorScheme.onSurfaceVariant,
            maxLines = 1,
            overflow = TextOverflow.Ellipsis,
          )
        }
        Text(
          "Open",
          style = MaterialTheme.typography.labelLarge,
          fontWeight = FontWeight.SemiBold,
          color = shown.status.tint,
        )
      }
    }
  }
}

/** A failed operation as one message to read, copy and dismiss (ADR 0020). */
@Composable
fun ErrorToast(modifier: Modifier = Modifier) {
  val alerts = LocalStores.current.alerts
  val context = LocalContext.current
  val message = alerts.error
  val last = remember { mutableStateOf(message) }
  if (message != null) last.value = message
  AnimatedVisibility(
    visible = message != null,
    enter = slideInVertically { it } + fadeIn(),
    exit = slideOutVertically { it } + fadeOut(),
    modifier = modifier,
  ) {
    val shown = last.value ?: return@AnimatedVisibility
    Surface(
      shape = RoundedCornerShape(24.dp),
      tonalElevation = 6.dp,
      shadowElevation = 8.dp,
      modifier = Modifier.fillMaxWidth().padding(horizontal = 12.dp),
    ) {
      Row(
        verticalAlignment = Alignment.CenterVertically,
        horizontalArrangement = Arrangement.spacedBy(8.dp),
        modifier = Modifier.padding(start = 16.dp, end = 6.dp, top = 4.dp, bottom = 4.dp),
      ) {
        Icon(
          Icons.Filled.Warning,
          contentDescription = null,
          tint = MaterialTheme.colorScheme.error,
          modifier = Modifier.size(20.dp),
        )
        Text(shown, style = MaterialTheme.typography.bodyMedium, modifier = Modifier.weight(1f))
        TextButton(
          onClick = {
            context
              .getSystemService(ClipboardManager::class.java)
              .setPrimaryClip(ClipData.newPlainText("Condr", shown))
          }
        ) {
          Text("Copy")
        }
        IconButton(onClick = { alerts.dismissError() }) {
          Icon(
            Icons.Filled.Close,
            contentDescription = "Dismiss",
            tint = MaterialTheme.colorScheme.onSurfaceVariant,
          )
        }
      }
    }
  }
}

/** A passing word, such as "Copied to clipboard". */
@Composable
fun NoticeToast(modifier: Modifier = Modifier) {
  val notice = LocalStores.current.alerts.notice
  val last = remember { mutableStateOf(notice) }
  if (notice != null) last.value = notice
  AnimatedVisibility(
    visible = notice != null,
    enter = slideInVertically { it } + fadeIn(),
    exit = slideOutVertically { it } + fadeOut(),
    modifier = modifier,
  ) {
    Surface(shape = RoundedCornerShape(50), tonalElevation = 6.dp, shadowElevation = 8.dp) {
      Text(
        last.value ?: "",
        style = MaterialTheme.typography.bodyMedium,
        fontWeight = FontWeight.Medium,
        modifier = Modifier.padding(horizontal = 16.dp, vertical = 10.dp),
      )
    }
  }
}

/**
 * The toasts at the bottom of the tabs, above the navigation bar; a Pane's page carries its own.
 */
@Composable
fun BottomToasts(modifier: Modifier = Modifier) {
  val session = LocalStores.current.session
  if (session.shown != null) return
  Column(
    modifier.fillMaxWidth().padding(bottom = 96.dp),
    horizontalAlignment = Alignment.CenterHorizontally,
    verticalArrangement = Arrangement.spacedBy(8.dp),
  ) {
    NoticeToast()
    ErrorToast()
  }
}
