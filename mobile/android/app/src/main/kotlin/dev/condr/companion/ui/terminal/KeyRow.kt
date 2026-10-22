package dev.condr.companion.ui.terminal

import androidx.compose.foundation.gestures.awaitEachGesture
import androidx.compose.foundation.gestures.awaitFirstDown
import androidx.compose.foundation.gestures.waitForUpOrCancellation
import androidx.compose.foundation.horizontalScroll
import androidx.compose.foundation.layout.Arrangement
import androidx.compose.foundation.layout.Box
import androidx.compose.foundation.layout.PaddingValues
import androidx.compose.foundation.layout.Row
import androidx.compose.foundation.layout.height
import androidx.compose.foundation.layout.padding
import androidx.compose.foundation.layout.size
import androidx.compose.foundation.layout.widthIn
import androidx.compose.foundation.rememberScrollState
import androidx.compose.material.icons.Icons
import androidx.compose.material.icons.automirrored.filled.KeyboardArrowLeft
import androidx.compose.material.icons.automirrored.filled.KeyboardArrowRight
import androidx.compose.material.icons.filled.KeyboardArrowDown
import androidx.compose.material.icons.filled.KeyboardArrowUp
import androidx.compose.material3.ButtonDefaults
import androidx.compose.material3.FilledTonalButton
import androidx.compose.material3.HorizontalFloatingToolbar
import androidx.compose.material3.Icon
import androidx.compose.material3.IconButton
import androidx.compose.material3.MaterialTheme
import androidx.compose.material3.Text
import androidx.compose.material3.VerticalDivider
import androidx.compose.runtime.Composable
import androidx.compose.runtime.rememberCoroutineScope
import androidx.compose.ui.Alignment
import androidx.compose.ui.Modifier
import androidx.compose.ui.graphics.vector.ImageVector
import androidx.compose.ui.input.pointer.pointerInput
import androidx.compose.ui.res.painterResource
import androidx.compose.ui.semantics.contentDescription
import androidx.compose.ui.semantics.semantics
import androidx.compose.ui.text.font.FontFamily
import androidx.compose.ui.text.font.FontWeight
import androidx.compose.ui.unit.dp
import dev.condr.companion.R
import kotlinx.coroutines.Job
import kotlinx.coroutines.delay
import kotlinx.coroutines.isActive
import kotlinx.coroutines.launch
import uniffi.condr_mobile.Key

/**
 * The keys the soft keyboard lacks (ADR 0041), on Material's floating toolbar (ADR 0048): Esc, Tab,
 * a sticky Ctrl and the arrows first, which is what a permission prompt or a numbered question
 * needs, then, a swipe along, Shift-Tab, Home, End and the page keys; a key cut off at the edge
 * shows that the row scrolls. Past a divider, Backspace and Return stay put, since a line already
 * in the program's composer is edited there, not in the field.
 */
@Composable
fun KeyRow(
  control: Boolean,
  onControl: (Boolean) -> Unit,
  press: (Key) -> Unit,
  modifier: Modifier = Modifier,
) {
  HorizontalFloatingToolbar(
    expanded = true,
    modifier = modifier,
    trailingContent = {
      VerticalDivider(Modifier.height(24.dp).padding(horizontal = 4.dp))
      RepeatingKey(contentDescription = "Backspace") { press(Key.Backspace) }
      IconButton(
        onClick = { press(Key.Enter) },
        modifier = Modifier.semantics { contentDescription = "Return" },
      ) {
        Icon(painterResource(R.drawable.ic_return), contentDescription = null)
      }
    },
  ) {
    Row(
      horizontalArrangement = Arrangement.spacedBy(2.dp),
      verticalAlignment = Alignment.CenterVertically,
      modifier = Modifier.widthIn(max = 260.dp).horizontalScroll(rememberScrollState()),
    ) {
      TextKey("esc", "Escape") { press(Key.Escape) }
      TextKey("tab", "Tab") { press(Key.Tab) }
      FilledTonalButton(
        onClick = { onControl(!control) },
        contentPadding = PaddingValues(horizontal = 10.dp),
        colors =
          if (control)
            ButtonDefaults.filledTonalButtonColors(
              containerColor = MaterialTheme.colorScheme.primary,
              contentColor = MaterialTheme.colorScheme.onPrimary,
            )
          else ButtonDefaults.filledTonalButtonColors(),
        modifier =
          Modifier.height(40.dp).widthIn(min = 48.dp).semantics { contentDescription = "Control" },
      ) {
        Text(
          "ctrl",
          fontFamily = FontFamily.Monospace,
          fontWeight = FontWeight.SemiBold,
          style = MaterialTheme.typography.labelLarge,
          maxLines = 1,
          softWrap = false,
        )
      }
      ArrowKey(Icons.AutoMirrored.Filled.KeyboardArrowLeft, "Left") { press(Key.Left) }
      ArrowKey(Icons.Filled.KeyboardArrowUp, "Up") { press(Key.Up) }
      ArrowKey(Icons.Filled.KeyboardArrowDown, "Down") { press(Key.Down) }
      ArrowKey(Icons.AutoMirrored.Filled.KeyboardArrowRight, "Right") { press(Key.Right) }
      TextKey("⇧tab", "Shift-Tab") { press(Key.BackTab) }
      TextKey("home", "Home") { press(Key.Home) }
      TextKey("end", "End") { press(Key.End) }
      TextKey("pgup", "Page Up") { press(Key.PageUp) }
      TextKey("pgdn", "Page Down") { press(Key.PageDown) }
    }
  }
}

@Composable
private fun TextKey(title: String, label: String, action: () -> Unit) {
  FilledTonalButton(
    onClick = action,
    contentPadding = PaddingValues(horizontal = 10.dp),
    modifier = Modifier.height(40.dp).widthIn(min = 48.dp).semantics { contentDescription = label },
  ) {
    Text(
      title,
      fontFamily = FontFamily.Monospace,
      fontWeight = FontWeight.SemiBold,
      style = MaterialTheme.typography.labelLarge,
      maxLines = 1,
      softWrap = false,
    )
  }
}

@Composable
private fun ArrowKey(icon: ImageVector, label: String, action: () -> Unit) {
  IconButton(
    onClick = action,
    modifier = Modifier.size(40.dp).semantics { contentDescription = label },
  ) {
    Icon(icon, contentDescription = null)
  }
}

/** A key that goes down on touch and repeats while held, as a hardware key does. */
@Composable
private fun RepeatingKey(contentDescription: String, press: () -> Unit) {
  val scope = rememberCoroutineScope()
  Box(
    Modifier.size(48.dp)
      .semantics { this.contentDescription = contentDescription }
      .pointerInput(Unit) {
        awaitEachGesture {
          awaitFirstDown()
          press()
          val repeating: Job = scope.launch {
            delay(450)
            while (isActive) {
              press()
              delay(70)
            }
          }
          waitForUpOrCancellation()
          repeating.cancel()
        }
      },
    contentAlignment = Alignment.Center,
  ) {
    Icon(painterResource(R.drawable.ic_backspace), contentDescription = null)
  }
}
