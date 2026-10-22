package dev.condr.companion.ui.terminal

import android.content.Intent
import android.net.Uri
import androidx.compose.animation.AnimatedVisibility
import androidx.compose.foundation.background
import androidx.compose.foundation.gestures.detectTransformGestures
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
import androidx.compose.runtime.setValue
import androidx.compose.ui.Alignment
import androidx.compose.ui.Modifier
import androidx.compose.ui.focus.FocusRequester
import androidx.compose.ui.focus.focusRequester
import androidx.compose.ui.focus.onFocusChanged
import androidx.compose.ui.graphics.Color
import androidx.compose.ui.graphics.SolidColor
import androidx.compose.ui.hapticfeedback.HapticFeedbackType
import androidx.compose.ui.input.pointer.pointerInput
import androidx.compose.ui.platform.LocalContext
import androidx.compose.ui.platform.LocalFocusManager
import androidx.compose.ui.platform.LocalHapticFeedback
import androidx.compose.ui.res.painterResource
import androidx.compose.ui.text.font.FontWeight
import androidx.compose.ui.text.input.KeyboardCapitalization
import androidx.compose.ui.text.style.TextOverflow
import androidx.compose.ui.unit.dp
import androidx.compose.ui.window.Popup
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
import dev.condr.companion.ui.theme.StatusGlyph
import dev.condr.companion.ui.theme.tint
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
  var text by remember { mutableStateOf("") }
  var control by remember { mutableStateOf(false) }
  var focused by remember { mutableStateOf(false) }
  var copyAt by remember { mutableStateOf<androidx.compose.ui.unit.IntOffset?>(null) }
  var baseSize by remember { mutableStateOf<Float?>(null) }
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
  // The page is dark whatever the theme, so the status bar's icons go light while it shows.
  val view = androidx.compose.ui.platform.LocalView.current
  DisposableEffect(view) {
    val window = (view.context as? android.app.Activity)?.window
    val controller = window?.let { androidx.core.view.WindowCompat.getInsetsController(it, view) }
    val wasLight = controller?.isAppearanceLightStatusBars
    controller?.isAppearanceLightStatusBars = false
    onDispose { if (wasLight != null) controller.isAppearanceLightStatusBars = wasLight }
  }

  val screen = session.screen
  val agent = pane.agent
  Scaffold(
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
          ) {
            AgentMarks.resource(agent?.kind)?.let {
              Icon(painterResource(it), contentDescription = null, modifier = Modifier.size(18.dp))
            }
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
                  StatusGlyph(agent.status, 6.dp)
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
            DropdownMenuItem(
              text = { Text("Smaller text") },
              onClick = {
                preferences.fontSize -= 1f
                menu = false
              },
            )
            DropdownMenuItem(
              text = { Text("Larger text") },
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
                      maxLines = 1,
                      overflow = TextOverflow.Ellipsis,
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
    Box(
      Modifier.fillMaxSize().padding(insets).padding(horizontal = GUTTER).pointerInput(Unit) {
        detectTransformGestures { _, _, zoom, _ ->
          if (zoom == 1f) return@detectTransformGestures
          val base = baseSize ?: preferences.fontSize.also { baseSize = it }
          preferences.fontSize =
            ((base * zoom * 2).toInt() / 2f).coerceIn(
              Preferences.SMALLEST_FONT,
              Preferences.LARGEST_FONT,
            )
        }
      }
    ) {
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
                cell?.link != null ->
                  context.startActivity(Intent(Intent.ACTION_VIEW, Uri.parse(cell.link)))
                screen?.mouseTracking == true -> session.click(at.row, at.column)
                else -> focusManager.clearFocus()
              }
            },
            longPress = { at ->
              copyAt = null
              session.selectWord(at.row, at.column)
            },
            drag = { from, to -> session.selectRange(from.row, from.column, to.row, to.column) },
            released = { copyAt = androidx.compose.ui.unit.IntOffset(0, 0) },
            wheel = { at, lines -> session.wheel(at.row, at.column, lines) },
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
      if (copyAt != null && screen?.hasSelection == true) {
        Popup(
          alignment = Alignment.TopCenter,
          properties = PopupProperties(focusable = false),
          onDismissRequest = { copyAt = null },
        ) {
          Surface(
            shape = MaterialTheme.shapes.large,
            tonalElevation = 6.dp,
            shadowElevation = 6.dp,
            modifier = Modifier.padding(top = 8.dp),
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
