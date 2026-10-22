package dev.condr.companion.ui.settings

import android.content.ClipData
import android.content.ClipboardManager
import android.content.Intent
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
import androidx.compose.foundation.layout.padding
import androidx.compose.foundation.layout.size
import androidx.compose.foundation.rememberScrollState
import androidx.compose.foundation.shape.CircleShape
import androidx.compose.foundation.shape.RoundedCornerShape
import androidx.compose.foundation.text.KeyboardActions
import androidx.compose.foundation.text.KeyboardOptions
import androidx.compose.foundation.verticalScroll
import androidx.compose.material.icons.Icons
import androidx.compose.material.icons.automirrored.filled.ArrowBack
import androidx.compose.material.icons.automirrored.filled.KeyboardArrowRight
import androidx.compose.material.icons.filled.Share
import androidx.compose.material3.ExperimentalMaterial3Api
import androidx.compose.material3.Icon
import androidx.compose.material3.IconButton
import androidx.compose.material3.LargeTopAppBar
import androidx.compose.material3.ListItem
import androidx.compose.material3.MaterialTheme
import androidx.compose.material3.OutlinedTextField
import androidx.compose.material3.Scaffold
import androidx.compose.material3.Slider
import androidx.compose.material3.Switch
import androidx.compose.material3.Text
import androidx.compose.material3.TopAppBar
import androidx.compose.material3.TopAppBarDefaults
import androidx.compose.runtime.Composable
import androidx.compose.runtime.DisposableEffect
import androidx.compose.runtime.LaunchedEffect
import androidx.compose.runtime.getValue
import androidx.compose.runtime.mutableStateOf
import androidx.compose.runtime.remember
import androidx.compose.runtime.rememberCoroutineScope
import androidx.compose.runtime.setValue
import androidx.compose.ui.Alignment
import androidx.compose.ui.Modifier
import androidx.compose.ui.focus.FocusRequester
import androidx.compose.ui.focus.focusRequester
import androidx.compose.ui.graphics.Color
import androidx.compose.ui.input.nestedscroll.nestedScroll
import androidx.compose.ui.platform.LocalContext
import androidx.compose.ui.platform.LocalDensity
import androidx.compose.ui.res.painterResource
import androidx.compose.ui.text.SpanStyle
import androidx.compose.ui.text.buildAnnotatedString
import androidx.compose.ui.text.font.FontWeight
import androidx.compose.ui.text.input.ImeAction
import androidx.compose.ui.text.rememberTextMeasurer
import androidx.compose.ui.text.withStyle
import androidx.compose.ui.unit.dp
import androidx.compose.ui.unit.sp
import dev.condr.companion.BuildConfig
import dev.condr.companion.R
import dev.condr.companion.model.Preferences
import dev.condr.companion.model.SettingsRoute
import dev.condr.companion.ui.LocalStores
import dev.condr.companion.ui.terminal.TerminalFont
import dev.condr.companion.ui.terminal.TerminalPalette
import kotlinx.coroutines.launch

@OptIn(ExperimentalMaterial3Api::class)
@Composable
fun SettingsScreen() {
  val stores = LocalStores.current
  val preferences = stores.preferences
  val router = stores.router
  val session = stores.session
  val lock = stores.lock
  val context = LocalContext.current
  val scope = rememberCoroutineScope()
  val scroll = TopAppBarDefaults.exitUntilCollapsedScrollBehavior()
  Scaffold(
    modifier = Modifier.nestedScroll(scroll.nestedScrollConnection),
    topBar = { LargeTopAppBar(title = { Text("Settings") }, scrollBehavior = scroll) },
  ) { insets ->
    Column(Modifier.fillMaxSize().padding(insets).verticalScroll(rememberScrollState())) {
      ListItem(
        headlineContent = {
          Text(
            preferences.phoneName.ifBlank { Preferences.systemDeviceName(context) },
            fontWeight = FontWeight.Medium,
          )
        },
        supportingContent = {
          val count = session.devices.size
          Text("Paired with $count ${if (count == 1) "computer" else "computers"}")
        },
        leadingContent = {
          Box(
            Modifier.size(52.dp)
              .background(
                MaterialTheme.colorScheme.surfaceContainerHighest,
                RoundedCornerShape(14.dp),
              ),
            contentAlignment = Alignment.Center,
          ) {
            Icon(
              painterResource(R.drawable.ic_phone),
              contentDescription = null,
              modifier = Modifier.size(28.dp),
            )
          }
        },
        trailingContent = {
          Icon(
            Icons.AutoMirrored.Filled.KeyboardArrowRight,
            contentDescription = null,
            tint = MaterialTheme.colorScheme.outline,
          )
        },
        modifier = Modifier.clickable { router.settingsPath.add(SettingsRoute.PhoneName) },
      )

      SectionHeader("Terminal")
      TextSize(preferences)

      SectionHeader("Security")
      ListItem(
        headlineContent = { Text("Lock with ${lock.method}") },
        supportingContent = {
          Text(
            when {
              !lock.available -> "Set a screen lock on this phone to lock Condr."
              lock.enabled ->
                "Condr asks for your ${lock.method} when it opens and after five minutes away."
              else -> "Anyone who can open this phone can open Condr and reach your computers."
            }
          )
        },
        trailingContent = {
          Switch(
            checked = lock.enabled && lock.available,
            enabled = lock.available,
            onCheckedChange = { on -> scope.launch { lock.setEnabled(on) } },
          )
        },
      )
      // The start of the key is enough to tell this phone's row apart under a computer's
      // Paired devices, and for `condr server revoke` there.
      ListItem(
        headlineContent = { Text("Key") },
        supportingContent = { Text(session.fingerprint.take(8) + "…") },
        trailingContent = {
          IconButton(
            onClick = {
              context
                .getSystemService(ClipboardManager::class.java)
                .setPrimaryClip(ClipData.newPlainText("Condr key", session.fingerprint))
              stores.alerts.notify("Copied to clipboard")
            }
          ) {
            Icon(painterResource(R.drawable.ic_copy), contentDescription = "Copy the whole key")
          }
        },
      )
      ListItem(
        headlineContent = { Text("If you lose this phone") },
        trailingContent = {
          Icon(
            Icons.AutoMirrored.Filled.KeyboardArrowRight,
            contentDescription = null,
            tint = MaterialTheme.colorScheme.outline,
          )
        },
        modifier = Modifier.clickable { router.settingsPath.add(SettingsRoute.LostPhone) },
      )

      SectionHeader("About")
      ListItem(
        headlineContent = { Text("Version") },
        supportingContent = { Text("${BuildConfig.VERSION_NAME} (${BuildConfig.VERSION_CODE})") },
      )
      ListItem(
        headlineContent = { Text("condr.dev") },
        trailingContent = {
          Icon(
            Icons.Filled.Share,
            contentDescription = null,
            tint = MaterialTheme.colorScheme.outline,
            modifier = Modifier.size(18.dp),
          )
        },
        modifier =
          Modifier.clickable {
            context.startActivity(
              Intent(Intent.ACTION_VIEW, android.net.Uri.parse("https://condr.dev"))
            )
          },
      )
      Spacer(Modifier.size(24.dp))
    }
  }
}

@Composable
private fun SectionHeader(title: String) {
  Text(
    title,
    style = MaterialTheme.typography.titleSmall,
    color = MaterialTheme.colorScheme.primary,
    modifier = Modifier.padding(start = 16.dp, end = 16.dp, top = 20.dp, bottom = 4.dp),
  )
}

/**
 * The terminal's text size, with two lines at the chosen size and about how many columns the phone
 * fits in portrait at it.
 */
@Composable
private fun TextSize(preferences: Preferences) {
  val measurer = rememberTextMeasurer()
  val density = LocalDensity.current
  Column(
    Modifier.padding(horizontal = 16.dp, vertical = 8.dp),
    verticalArrangement = Arrangement.spacedBy(8.dp),
  ) {
    Row(verticalAlignment = Alignment.CenterVertically) {
      Text("Text size", style = MaterialTheme.typography.bodyLarge)
      Spacer(Modifier.weight(1f))
      Text(
        "%.1f sp".format(preferences.fontSize),
        color = MaterialTheme.colorScheme.onSurfaceVariant,
      )
    }
    Slider(
      value = preferences.fontSize,
      onValueChange = { preferences.fontSize = (it * 2).toInt() / 2f },
      valueRange = Preferences.SMALLEST_FONT..Preferences.LARGEST_FONT,
      steps = ((Preferences.LARGEST_FONT - Preferences.SMALLEST_FONT) * 2).toInt() - 1,
    )
    BoxWithConstraints(Modifier.fillMaxWidth()) {
      val style = TerminalFont.style(preferences.fontSize.sp)
      val cell = measurer.measure("0", style).size.width
      val columns = with(density) { ((maxWidth - 8.dp * 2).toPx() / cell).toInt() }
      Column(verticalArrangement = Arrangement.spacedBy(6.dp)) {
        Column(
          Modifier.fillMaxWidth()
            .background(Color(TerminalPalette.BACKGROUND), RoundedCornerShape(12.dp))
            .padding(10.dp),
          verticalArrangement = Arrangement.spacedBy(2.dp),
        ) {
          val fg = Color(TerminalPalette.FOREGROUND)
          Text(
            buildAnnotatedString {
              withStyle(SpanStyle(color = Color(TerminalPalette.GREEN))) { append("● ") }
              withStyle(
                SpanStyle(color = Color(TerminalPalette.BRIGHT_WHITE), fontWeight = FontWeight.Bold)
              ) {
                append("Bash")
              }
              append("(cargo test)")
            },
            style = style,
            color = fg,
            maxLines = 1,
          )
          Text(
            buildAnnotatedString {
              withStyle(SpanStyle(color = Color(TerminalPalette.GRAY))) { append("  ⎿  ") }
              append("test result: ")
              withStyle(
                SpanStyle(color = Color(TerminalPalette.GREEN), fontWeight = FontWeight.Bold)
              ) {
                append("ok")
              }
              append(". 46 passed")
            },
            style = style,
            color = fg,
            maxLines = 1,
          )
        }
        Text(
          "About $columns columns in portrait",
          style = MaterialTheme.typography.bodySmall,
          color = MaterialTheme.colorScheme.onSurfaceVariant,
        )
      }
    }
  }
}

/** This phone's name, as each computer's Paired devices lists it (ADR 0040). */
@OptIn(ExperimentalMaterial3Api::class)
@Composable
fun PhoneNameScreen() {
  val stores = LocalStores.current
  val preferences = stores.preferences
  val session = stores.session
  val router = stores.router
  val context = LocalContext.current
  var draft by remember {
    mutableStateOf(preferences.phoneName.ifBlank { Preferences.systemDeviceName(context) })
  }
  val focus = remember { FocusRequester() }
  val scope = rememberCoroutineScope()
  LaunchedEffect(Unit) { focus.requestFocus() }
  // As the system's own name field does, leaving keeps what was typed.
  DisposableEffect(Unit) {
    onDispose {
      val name = draft.trim()
      if (name.isNotEmpty() && name != preferences.phoneName) {
        preferences.phoneName = name
        scope.launch { session.rename(name) }
      }
    }
  }
  Scaffold(
    topBar = {
      TopAppBar(
        title = { Text("Name") },
        navigationIcon = {
          IconButton(onClick = { router.settingsPath.removeLastOrNull() }) {
            Icon(Icons.AutoMirrored.Filled.ArrowBack, contentDescription = "Back")
          }
        },
      )
    }
  ) { insets ->
    OutlinedTextField(
      value = draft,
      onValueChange = { draft = it },
      singleLine = true,
      label = { Text("Name") },
      keyboardOptions = KeyboardOptions(imeAction = ImeAction.Done),
      keyboardActions = KeyboardActions(onDone = { router.settingsPath.removeLastOrNull() }),
      modifier = Modifier.fillMaxWidth().padding(insets).padding(16.dp).focusRequester(focus),
    )
  }
}

/** What ADR 0040 asks the help to say: revocation is per computer. */
@OptIn(ExperimentalMaterial3Api::class)
@Composable
fun LostPhoneScreen() {
  val router = LocalStores.current.router
  Scaffold(
    topBar = {
      TopAppBar(
        title = {},
        navigationIcon = {
          IconButton(onClick = { router.settingsPath.removeLastOrNull() }) {
            Icon(Icons.AutoMirrored.Filled.ArrowBack, contentDescription = "Back")
          }
        },
      )
    }
  ) { insets ->
    Column(
      Modifier.fillMaxSize().padding(insets).verticalScroll(rememberScrollState()).padding(24.dp),
      verticalArrangement = Arrangement.spacedBy(18.dp),
    ) {
      Icon(
        painterResource(R.drawable.ic_phone),
        contentDescription = null,
        tint = MaterialTheme.colorScheme.onSurfaceVariant,
        modifier = Modifier.size(40.dp),
      )
      Text(
        "If you lose this phone",
        style = MaterialTheme.typography.headlineSmall,
        fontWeight = FontWeight.Bold,
      )
      Step(
        1,
        "On each computer this phone is paired with, open Condr's Settings › Paired devices and remove it. Each computer keeps its own list, so removing it on one doesn't remove it on the others.",
      )
      Step(2, "Then look over each list for a device you don't recognize, and remove that too.")
      Step(3, "A new phone is a new device: pair it with an invite as you did this one.")
    }
  }
}

@Composable
private fun Step(number: Int, text: String) {
  Row(horizontalArrangement = Arrangement.spacedBy(12.dp), verticalAlignment = Alignment.Top) {
    Box(
      Modifier.size(24.dp)
        .background(MaterialTheme.colorScheme.surfaceContainerHighest, CircleShape),
      contentAlignment = Alignment.Center,
    ) {
      Text("$number", style = MaterialTheme.typography.labelMedium, fontWeight = FontWeight.Bold)
    }
    Text(text, style = MaterialTheme.typography.bodyLarge, modifier = Modifier.weight(1f))
  }
}
