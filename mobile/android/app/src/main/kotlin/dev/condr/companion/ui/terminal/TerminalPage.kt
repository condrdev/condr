package dev.condr.companion.ui.terminal

import android.content.ActivityNotFoundException
import android.content.Context
import android.content.Intent
import android.net.Uri
import androidx.compose.animation.AnimatedVisibility
import androidx.compose.foundation.background
import androidx.compose.foundation.gestures.awaitEachGesture
import androidx.compose.foundation.gestures.awaitFirstDown
import androidx.compose.foundation.layout.Arrangement
import androidx.compose.foundation.layout.Box
import androidx.compose.foundation.layout.Column
import androidx.compose.foundation.layout.Row
import androidx.compose.foundation.layout.fillMaxSize
import androidx.compose.foundation.layout.fillMaxWidth
import androidx.compose.foundation.layout.heightIn
import androidx.compose.foundation.layout.imePadding
import androidx.compose.foundation.layout.navigationBarsPadding
import androidx.compose.foundation.layout.padding
import androidx.compose.foundation.layout.size
import androidx.compose.foundation.shape.RoundedCornerShape
import androidx.compose.foundation.text.BasicTextField
import androidx.compose.foundation.text.KeyboardOptions
import androidx.compose.material.icons.Icons
import androidx.compose.material.icons.automirrored.filled.ArrowBack
import androidx.compose.material.icons.filled.MoreVert
import androidx.compose.material3.CircularProgressIndicator
import androidx.compose.material3.DropdownMenu
import androidx.compose.material3.DropdownMenuItem
import androidx.compose.material3.ExperimentalMaterial3Api
import androidx.compose.material3.FilledIconButton
import androidx.compose.material3.FilledTonalIconButton
import androidx.compose.material3.Icon
import androidx.compose.material3.IconButton
import androidx.compose.material3.MaterialTheme
import androidx.compose.material3.Scaffold
import androidx.compose.material3.SmallFloatingActionButton
import androidx.compose.material3.Surface
import androidx.compose.material3.Text
import androidx.compose.material3.TextButton
import androidx.compose.material3.TopAppBar
import androidx.compose.material3.TopAppBarDefaults
import androidx.compose.runtime.Composable
import androidx.compose.runtime.DisposableEffect
import androidx.compose.runtime.LaunchedEffect
import androidx.compose.runtime.getValue
import androidx.compose.runtime.mutableStateOf
import androidx.compose.runtime.remember
import androidx.compose.runtime.saveable.rememberSaveable
import androidx.compose.runtime.setValue
import androidx.compose.ui.Alignment
import androidx.compose.ui.Modifier
import androidx.compose.ui.focus.FocusRequester
import androidx.compose.ui.focus.focusRequester
import androidx.compose.ui.focus.onFocusChanged
import androidx.compose.ui.geometry.Offset
import androidx.compose.ui.graphics.Color
import androidx.compose.ui.graphics.SolidColor
import androidx.compose.ui.hapticfeedback.HapticFeedbackType
import androidx.compose.ui.input.pointer.PointerEventPass
import androidx.compose.ui.input.pointer.pointerInput
import androidx.compose.ui.platform.LocalContext
import androidx.compose.ui.platform.LocalDensity
import androidx.compose.ui.platform.LocalFocusManager
import androidx.compose.ui.platform.LocalHapticFeedback
import androidx.compose.ui.res.painterResource
import androidx.compose.ui.semantics.semantics
import androidx.compose.ui.text.font.FontWeight
import androidx.compose.ui.text.input.KeyboardCapitalization
import androidx.compose.ui.text.style.TextOverflow
import androidx.compose.ui.unit.IntOffset
import androidx.compose.ui.unit.IntRect
import androidx.compose.ui.unit.IntSize
import androidx.compose.ui.unit.LayoutDirection
import androidx.compose.ui.unit.dp
import androidx.compose.ui.window.Popup
import androidx.compose.ui.window.PopupPositionProvider
import androidx.compose.ui.window.PopupProperties
import dev.condr.companion.R
import dev.condr.companion.model.PaneRef
import dev.condr.companion.model.Preferences
import dev.condr.companion.model.connected
import dev.condr.companion.model.name
import dev.condr.companion.model.needsYou
import dev.condr.companion.model.place
import dev.condr.companion.model.title
import dev.condr.companion.ui.ErrorToast
import dev.condr.companion.ui.LocalStores
import dev.condr.companion.ui.NoticeToast
import dev.condr.companion.ui.agents.AgentMarks
import dev.condr.companion.ui.theme.DarkSystemBars
import dev.condr.companion.ui.theme.StatusGlyph
import dev.condr.companion.ui.theme.tint
import kotlin.math.roundToInt
import kotlinx.coroutines.delay
import uniffi.condr_mobile.AgentStatus
import uniffi.condr_mobile.Key

/** The space either side of the grid. */
private val GUTTER = 10.dp

/**
 * A Pane's terminal, where the phone is one more window (ADR 0041): it reports focus and sizes the
 * Pane to its own grid, draws what the Server sends, and answers through the key row and the field.
 */
@OptIn(ExperimentalMaterial3Api::class)
@Composable
fun TerminalPage(ref: PaneRef) {
  val stores = LocalStores.current
  val session = stores.session
  val preferences = stores.preferences
  val router = stores.router
  val context = LocalContext.current
  val haptics = LocalHapticFeedback.current
  val focusManager = LocalFocusManager.current
  val pane = session.pane(ref)
  val device = session.device(ref.address)
  val workspace = session.workspace(ref)
  if (pane == null || device == null || workspace == null) {
    // Forgotten while this page was up, or a Pane that closed: the stack drops it once its
    // Device is reached and has no such Pane (ADR 0047).
    LaunchedEffect(device?.connected) { if (device?.connected == true) router.close(ref) }
    Box(Modifier.fillMaxSize().background(Color(TerminalPalette.BACKGROUND)))
    return
  }
  var grid by remember { mutableStateOf<Pair<Int, Int>?>(null) }
  // The draft and Ctrl outlive a rotation.
  var text by rememberSaveable { mutableStateOf("") }
  var control by rememberSaveable { mutableStateOf(false) }
  var focused by remember { mutableStateOf(false) }
  /** Where a long press let go with a selection: Copy shows there. */
  var copyAt by remember { mutableStateOf<Offset?>(null) }
  val focus = remember { FocusRequester() }
  val path =
    if (router.tab == dev.condr.companion.model.AppTab.AGENTS) router.agentsPath
    else router.devicesPath

  fun press(key: Key) {
    haptics.performHapticFeedback(HapticFeedbackType.KeyboardTap)
    session.send(key, control)
    control = false
  }

  /** Send is the draft as the Server submits an agent prompt: a paste, then Return (ADR 0041). */
  fun submit() {
    if (text.isEmpty()) return
    haptics.performHapticFeedback(HapticFeedbackType.KeyboardTap)
    session.submit(text)
    text = ""
    control = false
  }

  // The grid settles after the keyboard, a rotation or a pinch; one Resize then, not one per
  // frame of the animation (ADR 0041).
  LaunchedEffect(grid) {
    val (rows, columns) = grid ?: return@LaunchedEffect
    delay(250)
    session.show(ref, rows.toUShort(), columns.toUShort())
  }
  DisposableEffect(ref) {
    session.show(ref, 0u, 0u)
    onDispose { session.hide(ref) }
  }
  LaunchedEffect(Unit) { if (stores.focus) focus.requestFocus() }
  // The page is dark whatever the theme.
  DarkSystemBars()

  val screen = session.screen
  val agent = pane.agent
  Scaffold(
    // A pinch anywhere on the page sizes the text.
    modifier = Modifier.pinchToSize(preferences),
    containerColor = Color(TerminalPalette.BACKGROUND),
    topBar = {
      TopAppBar(
        colors =
          TopAppBarDefaults.topAppBarColors(containerColor = Color(TerminalPalette.BACKGROUND)),
        navigationIcon = {
          IconButton(onClick = { path.removeAt(path.lastIndex) }) {
            Icon(Icons.AutoMirrored.Filled.ArrowBack, contentDescription = "Back")
          }
        },
        title = {
          Row(
            verticalAlignment = Alignment.CenterVertically,
            horizontalArrangement = Arrangement.spacedBy(9.dp),
            modifier = Modifier.semantics(mergeDescendants = true) {},
          ) {
            Icon(
              painterResource(AgentMarks.resource(agent?.kind) ?: R.drawable.ic_terminal),
              contentDescription = null,
              modifier = Modifier.size(18.dp),
            )
            Column {
              Text(
                pane.name,
                style = MaterialTheme.typography.titleSmall,
                maxLines = 1,
                overflow = TextOverflow.Ellipsis,
              )
              Row(
                verticalAlignment = Alignment.CenterVertically,
                horizontalArrangement = Arrangement.spacedBy(5.dp),
              ) {
                if (agent != null) {
                  StatusGlyph(agent.status, 6.dp, ripple = true)
                  Text(
                    agent.status.title,
                    style = MaterialTheme.typography.labelSmall,
                    fontWeight =
                      if (agent.status.needsYou) FontWeight.SemiBold else FontWeight.Medium,
                    color =
                      if (agent.status.needsYou) agent.status.tint
                      else MaterialTheme.colorScheme.onSurfaceVariant,
                  )
                  Text(
                    "·",
                    style = MaterialTheme.typography.labelSmall,
                    color = MaterialTheme.colorScheme.outline,
                  )
                }
                Text(
                  place(workspace.name, device.label),
                  style = MaterialTheme.typography.labelSmall,
                  color = MaterialTheme.colorScheme.onSurfaceVariant,
                  maxLines = 1,
                  overflow = TextOverflow.Ellipsis,
                )
              }
            }
          }
        },
        actions = {
          var menu by remember { mutableStateOf(false) }
          IconButton(onClick = { menu = true }) {
            Icon(Icons.Filled.MoreVert, contentDescription = "More")
          }
          DropdownMenu(expanded = menu, onDismissRequest = { menu = false }) {
            Text(
              "Text size",
              style = MaterialTheme.typography.labelMedium,
              color = MaterialTheme.colorScheme.onSurfaceVariant,
              modifier = Modifier.padding(horizontal = 12.dp, vertical = 8.dp),
            )
            DropdownMenuItem(
              text = { Text("Smaller") },
              leadingIcon = { Text("A", style = MaterialTheme.typography.labelSmall) },
              onClick = {
                preferences.fontSize -= 1f
                menu = false
              },
            )
            DropdownMenuItem(
              text = { Text("Larger") },
              leadingIcon = { Text("A", style = MaterialTheme.typography.titleLarge) },
              onClick = {
                preferences.fontSize += 1f
                menu = false
              },
            )
          }
        },
      )
    },
    bottomBar = {
      Column(
        Modifier.fillMaxWidth().navigationBarsPadding().imePadding(),
        verticalArrangement = Arrangement.spacedBy(8.dp),
        horizontalAlignment = Alignment.CenterHorizontally,
      ) {
        NoticeToast()
        ErrorToast()
        KeyRow(
          control = control,
          onControl = { control = it },
          press = ::press,
          modifier = Modifier.padding(horizontal = 12.dp),
        )
        Row(
          verticalAlignment = Alignment.Bottom,
          horizontalArrangement = Arrangement.spacedBy(8.dp),
          modifier = Modifier.padding(start = 12.dp, end = 12.dp, bottom = 8.dp),
        ) {
          // The keyboard moves only when asked, since every appearance resizes the Pane for
          // every viewer (ADR 0041).
          AnimatedVisibility(visible = focused) {
            FilledTonalIconButton(
              onClick = { focusManager.clearFocus() },
              modifier = Modifier.size(48.dp),
            ) {
              Icon(
                painterResource(R.drawable.ic_keyboard_hide),
                contentDescription = "Hide keyboard",
              )
            }
          }
          // A draft of as many lines as the reply needs; Return is a line break, and the
          // terminal's Return is on the key row. The field stands as tall as the buttons.
          Surface(
            shape = RoundedCornerShape(24.dp),
            color = MaterialTheme.colorScheme.surfaceContainerHigh,
            modifier = Modifier.weight(1f).heightIn(min = 48.dp),
          ) {
            Box(
              Modifier.padding(horizontal = 16.dp, vertical = 12.dp),
              contentAlignment = Alignment.CenterStart,
            ) {
              BasicTextField(
                value = text,
                onValueChange = { new ->
                  // With Ctrl on, the key just typed goes as Ctrl+key and the draft stays as it
                  // was.
                  if (control && new.length > text.length) {
                    val kept = text.commonPrefixWith(new).length
                    new.getOrNull(kept)?.let { press(Key.Character(it.toString())) }
                  } else {
                    text = new
                  }
                },
                maxLines = 5,
                textStyle =
                  MaterialTheme.typography.bodyLarge.copy(
                    color = MaterialTheme.colorScheme.onSurface
                  ),
                cursorBrush = SolidColor(MaterialTheme.colorScheme.primary),
                keyboardOptions =
                  KeyboardOptions(
                    capitalization =
                      if (agent == null) KeyboardCapitalization.None
                      else KeyboardCapitalization.Sentences,
                    autoCorrectEnabled = agent != null,
                  ),
                modifier =
                  Modifier.fillMaxWidth().focusRequester(focus).onFocusChanged {
                    focused = it.isFocused
                  },
                decorationBox = { inner ->
                  if (text.isEmpty()) {
                    Text(
                      when {
                        agent == null -> "Command"
                        agent.status == AgentStatus.BLOCKED -> "Reply, or pick with ↑ ↓ and return"
                        else -> "Message ${agent.label}"
                      },
                      style = MaterialTheme.typography.bodyLarge,
                      color = MaterialTheme.colorScheme.onSurfaceVariant,
                    )
                  }
                  inner()
                },
              )
            }
          }
          FilledIconButton(
            onClick = ::submit,
            enabled = text.isNotEmpty(),
            modifier = Modifier.size(48.dp),
          ) {
            Icon(painterResource(R.drawable.ic_send), contentDescription = "Send")
          }
        }
      }
    },
  ) { insets ->
    Box(Modifier.fillMaxSize().padding(insets).padding(horizontal = GUTTER)) {
      TerminalCanvas(
        screen = screen,
        fontSizeSp = preferences.fontSize,
        onGrid = { rows, columns -> grid = rows to columns },
        touches =
          TerminalTouches(
            tap = { at, cell ->
              copyAt = null
              when {
                screen?.hasSelection == true -> session.clearSelection()
                cell?.link != null -> cell.link?.let { openLink(context, it) }
                screen?.mouseTracking == true -> session.click(at.row, at.column)
                else -> focusManager.clearFocus()
              }
            },
            longPress = { at ->
              copyAt = null
              session.selectWord(at.row, at.column)
            },
            drag = { from, to -> session.selectRange(from.row, from.column, to.row, to.column) },
            released = { copyAt = it },
            wheel = { at, lines -> session.wheel(at.row, at.column, lines) },
            swiped = { copyAt = null },
          ),
        modifier = Modifier.fillMaxSize(),
      )
      if (screen == null)
        CircularProgressIndicator(Modifier.align(Alignment.Center), color = Color.White)
      // Back to the newest output, once the Pane has been scrolled up (ADR 0036).
      if ((screen?.scrolledLines ?: 0u) > 0u) {
        SmallFloatingActionButton(
          onClick = { session.scrollToBottom() },
          modifier = Modifier.align(Alignment.BottomEnd).padding(16.dp),
        ) {
          Icon(
            painterResource(R.drawable.ic_arrow_down),
            contentDescription = "Jump to the newest output",
          )
        }
      }
      if (!device.connected) {
        Box(
          Modifier.fillMaxSize().background(Color(TerminalPalette.BACKGROUND).copy(alpha = 0.75f)),
          contentAlignment = Alignment.Center,
        ) {
          Column(
            horizontalAlignment = Alignment.CenterHorizontally,
            verticalArrangement = Arrangement.spacedBy(10.dp),
          ) {
            CircularProgressIndicator(color = Color.White)
            Text(
              "Reconnecting to ${device.label}…",
              color = Color.White,
              fontWeight = FontWeight.Medium,
            )
          }
        }
      }
      // Copy, offered once a long press lets go with a selection (ADR 0008).
      val finger = copyAt
      if (finger != null && screen?.hasSelection == true) {
        val margin = with(LocalDensity.current) { 12.dp.roundToPx() }
        Popup(
          popupPositionProvider = AboveFinger(finger, margin),
          properties = PopupProperties(focusable = false),
          onDismissRequest = { copyAt = null },
        ) {
          Surface(
            shape = MaterialTheme.shapes.large,
            tonalElevation = 6.dp,
            shadowElevation = 6.dp,
          ) {
            TextButton(
              onClick = {
                session.copySelection()
                copyAt = null
              }
            ) {
              Icon(
                painterResource(R.drawable.ic_copy),
                contentDescription = null,
                modifier = Modifier.size(18.dp),
              )
              Text("Copy", modifier = Modifier.padding(start = 8.dp))
            }
          }
        }
      }
    }
  }
}

/**
 * Copy over the finger where it let go, as iOS's edit menu sits, or under it when there is no room
 * above; `finger` is in the terminal's own coordinates.
 */
private class AboveFinger(private val finger: Offset, private val margin: Int) :
  PopupPositionProvider {
  override fun calculatePosition(
    anchorBounds: IntRect,
    windowSize: IntSize,
    layoutDirection: LayoutDirection,
    popupContentSize: IntSize,
  ): IntOffset {
    val x =
      (anchorBounds.left + finger.x.roundToInt() - popupContentSize.width / 2).coerceIn(
        0,
        (windowSize.width - popupContentSize.width).coerceAtLeast(0),
      )
    val above = anchorBounds.top + finger.y.roundToInt() - popupContentSize.height - margin
    val y =
      if (above >= anchorBounds.top) above else anchorBounds.top + finger.y.roundToInt() + margin
    return IntOffset(x, y)
  }
}

/** A link from the terminal, if the phone has an app for it; one it can't open does nothing. */
private fun openLink(context: Context, link: String) {
  val uri = Uri.parse(link)
  if (uri.scheme.isNullOrEmpty()) return
  try {
    context.startActivity(Intent(Intent.ACTION_VIEW, uri))
  } catch (_: ActivityNotFoundException) {}
}

/**
 * Two fingers size the text, from where it stood when the pinch began; they reach the page before
 * the terminal's own touches, so a pinch never scrolls or selects.
 */
private fun Modifier.pinchToSize(preferences: Preferences) =
  pointerInput(preferences) {
    awaitEachGesture {
      awaitFirstDown(requireUnconsumed = false, pass = PointerEventPass.Initial)
      var start: Pair<Float, Float>? = null
      do {
        val event = awaitPointerEvent(PointerEventPass.Initial)
        val fingers = event.changes.filter { it.pressed }
        if (fingers.size >= 2) {
          val span = (fingers[0].position - fingers[1].position).getDistance()
          val (size, from) = start ?: (preferences.fontSize to span).also { start = it }
          if (from > 0f) {
            val next =
              ((size * span / from) * 2)
                .roundToInt()
                .div(2f)
                .coerceIn(
                  Preferences.SMALLEST_FONT,
                  Preferences.LARGEST_FONT,
                )
            if (next != preferences.fontSize) preferences.fontSize = next
          }
          event.changes.forEach { it.consume() }
        }
      } while (event.changes.any { it.pressed })
    }
  }
