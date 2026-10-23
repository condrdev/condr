package dev.condr.companion.ui.terminal

import androidx.compose.foundation.background
import androidx.compose.foundation.gestures.awaitEachGesture
import androidx.compose.foundation.gestures.awaitFirstDown
import androidx.compose.foundation.gestures.waitForUpOrCancellation
import androidx.compose.foundation.horizontalScroll
import androidx.compose.foundation.indication
import androidx.compose.foundation.interaction.MutableInteractionSource
import androidx.compose.foundation.interaction.PressInteraction
import androidx.compose.foundation.layout.Arrangement
import androidx.compose.foundation.layout.Box
import androidx.compose.foundation.layout.PaddingValues
import androidx.compose.foundation.layout.Row
import androidx.compose.foundation.layout.Spacer
import androidx.compose.foundation.layout.fillMaxWidth
import androidx.compose.foundation.layout.height
import androidx.compose.foundation.layout.padding
import androidx.compose.foundation.layout.size
import androidx.compose.foundation.layout.width
import androidx.compose.foundation.layout.widthIn
import androidx.compose.foundation.rememberScrollState
import androidx.compose.material.icons.Icons
import androidx.compose.material.icons.automirrored.filled.KeyboardArrowLeft
import androidx.compose.material.icons.automirrored.filled.KeyboardArrowRight
import androidx.compose.material.icons.filled.KeyboardArrowDown
import androidx.compose.material.icons.filled.KeyboardArrowUp
import androidx.compose.material3.ButtonColors
import androidx.compose.material3.ButtonDefaults
import androidx.compose.material3.FilledTonalButton
import androidx.compose.material3.Icon
import androidx.compose.material3.MaterialTheme
import androidx.compose.material3.Text
import androidx.compose.material3.VerticalDivider
import androidx.compose.material3.ripple
import androidx.compose.runtime.Composable
import androidx.compose.runtime.remember
import androidx.compose.runtime.rememberCoroutineScope
import androidx.compose.ui.Alignment
import androidx.compose.ui.Modifier
import androidx.compose.ui.draw.clip
import androidx.compose.ui.graphics.painter.Painter
import androidx.compose.ui.graphics.vector.rememberVectorPainter
import androidx.compose.ui.input.pointer.pointerInput
import androidx.compose.ui.res.painterResource
import androidx.compose.ui.semantics.Role
import androidx.compose.ui.semantics.contentDescription
import androidx.compose.ui.semantics.onClick
import androidx.compose.ui.semantics.role
import androidx.compose.ui.semantics.selected
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
 * The keys the soft keyboard lacks (ADR 0041), each one of Material's tonal buttons on the page
 * itself (ADR 0048): Esc, Tab, a sticky Ctrl and the arrows first, which is what a permission
 * prompt or a numbered question needs, then, a swipe along, Shift-Tab, Home, End and the page keys;
 * a key cut off at the edge shows that the row scrolls. Past a divider, Backspace and Return stay
 * put, since a line already in the program's composer is edited there, not in the field.
 */
@Composable
fun KeyRow(
  control: Boolean,
  onControl: (Boolean) -> Unit,
  press: (Key) -> Unit,
  modifier: Modifier = Modifier,
) {
  Row(modifier.fillMaxWidth(), verticalAlignment = Alignment.CenterVertically) {
    Row(
      horizontalArrangement = Arrangement.spacedBy(6.dp),
      verticalAlignment = Alignment.CenterVertically,
      modifier = Modifier.weight(1f).horizontalScroll(rememberScrollState()),
    ) {
      TextKey("esc", "Escape") { press(Key.Escape) }
      TextKey("tab", "Tab") { press(Key.Tab) }
      TextKey(
        "ctrl",
        "Control",
        colors =
          if (control)
            ButtonDefaults.filledTonalButtonColors(
              containerColor = MaterialTheme.colorScheme.primary,
              contentColor = MaterialTheme.colorScheme.onPrimary,
            )
          else ButtonDefaults.filledTonalButtonColors(),
        modifier = Modifier.semantics { selected = control },
      ) {
        onControl(!control)
      }
      IconKey(rememberVectorPainter(Icons.AutoMirrored.Filled.KeyboardArrowLeft), "Left") {
        press(Key.Left)
      }
      IconKey(rememberVectorPainter(Icons.Filled.KeyboardArrowUp), "Up") { press(Key.Up) }
      IconKey(rememberVectorPainter(Icons.Filled.KeyboardArrowDown), "Down") { press(Key.Down) }
      IconKey(rememberVectorPainter(Icons.AutoMirrored.Filled.KeyboardArrowRight), "Right") {
        press(Key.Right)
      }
      TextKey("⇧tab", "Shift-Tab") { press(Key.BackTab) }
      TextKey("home", "Home") { press(Key.Home) }
      TextKey("end", "End") { press(Key.End) }
      TextKey("pgup", "Page Up") { press(Key.PageUp) }
      TextKey("pgdn", "Page Down") { press(Key.PageDown) }
    }
    VerticalDivider(Modifier.padding(horizontal = 8.dp).height(24.dp))
    RepeatingKey(contentDescription = "Backspace") { press(Key.Backspace) }
    Spacer(Modifier.width(6.dp))
    IconKey(painterResource(R.drawable.ic_return), "Return") { press(Key.Enter) }
  }
}

/** Every key the same height, and wide enough for a thumb. */
private val KeyModifier = Modifier.height(40.dp).widthIn(min = 48.dp)

@Composable
private fun TextKey(
  title: String,
  label: String,
  colors: ButtonColors = ButtonDefaults.filledTonalButtonColors(),
  modifier: Modifier = Modifier,
  action: () -> Unit,
) {
  FilledTonalButton(
    onClick = action,
    colors = colors,
    contentPadding = PaddingValues(horizontal = 10.dp),
    modifier = modifier.then(KeyModifier).semantics { contentDescription = label },
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
private fun IconKey(icon: Painter, label: String, action: () -> Unit) {
  FilledTonalButton(
    onClick = action,
    contentPadding = PaddingValues(horizontal = 10.dp),
    modifier = KeyModifier.semantics { contentDescription = label },
  ) {
    Icon(icon, contentDescription = null, modifier = Modifier.size(20.dp))
  }
}

/**
 * A key that goes down on touch and repeats while held, as a hardware key does, drawn as the other
 * keys are.
 */
@Composable
private fun RepeatingKey(contentDescription: String, press: () -> Unit) {
  val scope = rememberCoroutineScope()
  val interactions = remember { MutableInteractionSource() }
  val colors = ButtonDefaults.filledTonalButtonColors()
  Box(
    KeyModifier.clip(ButtonDefaults.shape)
      .background(colors.containerColor)
      .indication(interactions, ripple())
      .semantics {
        this.contentDescription = contentDescription
        role = Role.Button
        onClick {
          press()
          true
        }
      }
      .pointerInput(Unit) {
        awaitEachGesture {
          val down = awaitFirstDown()
          val touch = PressInteraction.Press(down.position)
          interactions.tryEmit(touch)
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
          interactions.tryEmit(PressInteraction.Release(touch))
        }
      },
    contentAlignment = Alignment.Center,
  ) {
    Icon(
      painterResource(R.drawable.ic_backspace),
      contentDescription = null,
      tint = colors.contentColor,
      modifier = Modifier.size(20.dp),
    )
  }
}
