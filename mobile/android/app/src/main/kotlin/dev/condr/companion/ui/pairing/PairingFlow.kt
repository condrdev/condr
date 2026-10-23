package dev.condr.companion.ui.pairing

import android.content.Context
import androidx.activity.compose.BackHandler
import androidx.compose.animation.AnimatedVisibility
import androidx.compose.animation.Crossfade
import androidx.compose.animation.core.animateDpAsState
import androidx.compose.animation.core.animateFloatAsState
import androidx.compose.animation.core.spring
import androidx.compose.animation.core.tween
import androidx.compose.animation.core.withInfiniteAnimationFrameMillis
import androidx.compose.foundation.BorderStroke
import androidx.compose.foundation.Canvas
import androidx.compose.foundation.Image
import androidx.compose.foundation.background
import androidx.compose.foundation.border
import androidx.compose.foundation.layout.Arrangement
import androidx.compose.foundation.layout.Box
import androidx.compose.foundation.layout.Column
import androidx.compose.foundation.layout.Row
import androidx.compose.foundation.layout.Spacer
import androidx.compose.foundation.layout.fillMaxHeight
import androidx.compose.foundation.layout.fillMaxSize
import androidx.compose.foundation.layout.fillMaxWidth
import androidx.compose.foundation.layout.height
import androidx.compose.foundation.layout.imePadding
import androidx.compose.foundation.layout.navigationBarsPadding
import androidx.compose.foundation.layout.offset
import androidx.compose.foundation.layout.padding
import androidx.compose.foundation.layout.size
import androidx.compose.foundation.layout.statusBarsPadding
import androidx.compose.foundation.layout.width
import androidx.compose.foundation.rememberScrollState
import androidx.compose.foundation.shape.CircleShape
import androidx.compose.foundation.shape.RoundedCornerShape
import androidx.compose.foundation.text.KeyboardActions
import androidx.compose.foundation.text.KeyboardOptions
import androidx.compose.foundation.verticalScroll
import androidx.compose.material.icons.Icons
import androidx.compose.material.icons.automirrored.filled.ArrowBack
import androidx.compose.material.icons.filled.Close
import androidx.compose.material3.AlertDialog
import androidx.compose.material3.Button
import androidx.compose.material3.ButtonDefaults
import androidx.compose.material3.ExperimentalMaterial3Api
import androidx.compose.material3.HorizontalDivider
import androidx.compose.material3.Icon
import androidx.compose.material3.IconButton
import androidx.compose.material3.ListItem
import androidx.compose.material3.ListItemDefaults
import androidx.compose.material3.LocalTextStyle
import androidx.compose.material3.MaterialTheme
import androidx.compose.material3.OutlinedTextField
import androidx.compose.material3.Scaffold
import androidx.compose.material3.SnackbarDuration
import androidx.compose.material3.SnackbarHost
import androidx.compose.material3.SnackbarHostState
import androidx.compose.material3.SnackbarResult
import androidx.compose.material3.Surface
import androidx.compose.material3.Text
import androidx.compose.material3.TextButton
import androidx.compose.material3.TopAppBar
import androidx.compose.runtime.Composable
import androidx.compose.runtime.CompositionLocalProvider
import androidx.compose.runtime.LaunchedEffect
import androidx.compose.runtime.getValue
import androidx.compose.runtime.mutableFloatStateOf
import androidx.compose.runtime.mutableStateListOf
import androidx.compose.runtime.mutableStateOf
import androidx.compose.runtime.remember
import androidx.compose.runtime.rememberCoroutineScope
import androidx.compose.runtime.setValue
import androidx.compose.ui.Alignment
import androidx.compose.ui.Modifier
import androidx.compose.ui.draw.alpha
import androidx.compose.ui.draw.clipToBounds
import androidx.compose.ui.focus.FocusRequester
import androidx.compose.ui.focus.focusRequester
import androidx.compose.ui.graphics.Color
import androidx.compose.ui.graphics.drawscope.Stroke
import androidx.compose.ui.hapticfeedback.HapticFeedbackType
import androidx.compose.ui.platform.LocalContext
import androidx.compose.ui.platform.LocalHapticFeedback
import androidx.compose.ui.res.painterResource
import androidx.compose.ui.semantics.clearAndSetSemantics
import androidx.compose.ui.semantics.contentDescription
import androidx.compose.ui.text.SpanStyle
import androidx.compose.ui.text.TextStyle
import androidx.compose.ui.text.buildAnnotatedString
import androidx.compose.ui.text.font.FontFamily
import androidx.compose.ui.text.font.FontWeight
import androidx.compose.ui.text.input.ImeAction
import androidx.compose.ui.text.style.TextAlign
import androidx.compose.ui.text.withStyle
import androidx.compose.ui.unit.dp
import androidx.compose.ui.unit.sp
import androidx.navigation3.runtime.NavKey
import androidx.navigation3.runtime.entryProvider
import androidx.navigation3.ui.NavDisplay
import com.google.android.gms.common.ConnectionResult
import com.google.android.gms.common.GoogleApiAvailability
import com.google.mlkit.vision.barcode.common.Barcode
import com.google.mlkit.vision.codescanner.GmsBarcodeScannerOptions
import com.google.mlkit.vision.codescanner.GmsBarcodeScanning
import dev.condr.companion.R
import dev.condr.companion.demo.DemoScene
import dev.condr.companion.demo.DemoWorld
import dev.condr.companion.model.AgentItem
import dev.condr.companion.model.Preferences
import dev.condr.companion.model.Session
import dev.condr.companion.model.byUrgency
import dev.condr.companion.model.name
import dev.condr.companion.ui.LocalStores
import dev.condr.companion.ui.agents.AgentAvatar
import dev.condr.companion.ui.terminal.TerminalPalette
import dev.condr.companion.ui.theme.DarkSystemBars
import dev.condr.companion.ui.theme.Logo
import dev.condr.companion.ui.theme.NumberedStep
import dev.condr.companion.ui.theme.StepTitle
import dev.condr.companion.ui.theme.agentStatusLine
import dev.condr.companion.ui.theme.reducedMotion
import kotlin.coroutines.cancellation.CancellationException
import kotlinx.coroutines.Job
import kotlinx.coroutines.launch
import kotlinx.serialization.Serializable
import uniffi.condr_mobile.SavedDevice

/** The pairing steps (ADR 0040), as the flow's back stack names them. */
sealed interface PairingStep : NavKey {
  @Serializable data object Welcome : PairingStep

  @Serializable data object Code : PairingStep

  @Serializable data object Name : PairingStep

  @Serializable data object Connecting : PairingStep
}

/** Whether a scanned or pasted code is an Invite link worth trying. */
fun isInvite(text: String): Boolean = text.startsWith("p2p://") || text.startsWith("tcp://")

/** Whether Google Play services can scan for the app; without them the link is pasted. */
fun canScan(context: Context): Boolean =
  GoogleApiAvailability.getInstance().isGooglePlayServicesAvailable(context) ==
    ConnectionResult.SUCCESS

/**
 * Pairing with a computer: the Invite scanned or pasted, the phone named on the first time, then
 * the handshake and the computer's Agents arriving. The first launch opens with the welcome; from
 * Devices it opens on the code step with a way out.
 */
@Composable
fun PairingFlow(firstLaunch: Boolean, finished: () -> Unit, tryDemo: (() -> Unit)? = null) {
  val stores = LocalStores.current
  val session = stores.session
  val preferences = stores.preferences
  val scope = rememberCoroutineScope()
  val path = remember {
    // A demo scene opens the flow on one of its steps.
    val order =
      listOf(PairingStep.Welcome, PairingStep.Code, PairingStep.Name, PairingStep.Connecting)
    val start =
      when (stores.scene) {
        DemoScene.PAIR,
        DemoScene.SCAN -> 1
        DemoScene.NAME -> 2
        DemoScene.CONNECTING,
        DemoScene.PAIRED -> 3
        else -> null
      }
    val from = if (firstLaunch) 0 else 1
    mutableStateListOf<PairingStep>().apply {
      addAll(if (start == null) listOf(order[from]) else order.subList(from, start + 1))
    }
  }
  var link by remember {
    mutableStateOf(if (stores.scene?.unpaired == true) DemoWorld.JPDEV + ".f3a91c0d" else "")
  }
  var name by remember {
    mutableStateOf(preferences.phoneName.ifBlank { Preferences.systemDeviceName(stores.context) })
  }
  /** The handshake under way, or done: one per visit to the last step. */
  var pairing by remember { mutableStateOf<Pairing?>(null) }

  /** The last step: the handshake starts once, as the person moves on to it. */
  fun connect() {
    if (pairing == null) pairing = Pairing(scope, session, preferences, link, name)
    if (path.last() != PairingStep.Connecting) path.add(PairingStep.Connecting)
  }

  /**
   * An Invite link moves the flow on to naming the phone, or straight to pairing once it has a
   * name.
   */
  fun take(found: String) {
    link = found
    if (preferences.named) connect()
    else if (path.last() != PairingStep.Name) path.add(PairingStep.Name)
  }

  LaunchedEffect(Unit) {
    // A demo scene opens on the last step; it pairs as the person would have.
    if (path.last() == PairingStep.Connecting && pairing == null) connect()
  }

  // Pairing another computer: Back on its first step closes it, as the sheet's Cancel does.
  BackHandler(enabled = !firstLaunch && path.size == 1, onBack = finished)
  NavDisplay(
    backStack = path,
    onBack = {
      if (path.last() == PairingStep.Connecting) pairing = null
      if (path.size > 1) path.removeAt(path.lastIndex) else finished()
    },
    entryProvider =
      entryProvider {
        entry<PairingStep.Welcome> { Welcome { path.add(PairingStep.Code) } }
        entry<PairingStep.Code> {
          InviteStep(
            cancel = if (firstLaunch) null else finished,
            back = if (firstLaunch) ({ path.removeAt(path.lastIndex) }) else null,
            found = ::take,
            tryDemo = if (firstLaunch) tryDemo else null,
          )
        }
        entry<PairingStep.Name> {
          NameStep(
            name = name,
            onName = { name = it },
            back = { path.removeAt(path.lastIndex) },
            next = ::connect,
          )
        }
        entry<PairingStep.Connecting> {
          pairing?.let { current ->
            ConnectingStep(
              pairing = current,
              firstLaunch = firstLaunch,
              finish = finished,
              back = {
                pairing = null
                path.removeAll { it == PairingStep.Connecting }
              },
            )
          }
        }
      },
  )
}

// Welcome

/**
 * The brand's one loud moment: ink, the mark, and what the app is for before a word of it is read.
 */
@Composable
private fun Welcome(next: () -> Unit) {
  val paper = Color(0xFFF5F5F7)
  val ink = Color(0xFF1D1D1F)
  val still = reducedMotion()
  var stage by remember { mutableStateOf(if (still) 3 else 0) }
  LaunchedEffect(Unit) {
    if (still) return@LaunchedEffect
    kotlinx.coroutines.delay(300)
    stage = 1
    kotlinx.coroutines.delay(500)
    stage = 2
    kotlinx.coroutines.delay(400)
    stage = 3
  }
  DarkSystemBars()
  // The mark builds itself from its bars, then the score plays, then the words rise into place.
  val assembly by
    animateFloatAsState(
      if (stage >= 1) 1f else 0f,
      spring(dampingRatio = 0.6f, stiffness = 120f),
      label = "assembly",
    )
  val logoAlpha by animateFloatAsState(if (stage >= 1) 1f else 0f, label = "logo")
  val stageAlpha by animateFloatAsState(if (stage >= 2) 1f else 0f, tween(900), label = "stage")
  val wordsAlpha by animateFloatAsState(if (stage >= 3) 1f else 0f, tween(800), label = "words")
  val wordsRise by animateDpAsState(if (stage >= 3) 0.dp else 14.dp, tween(800), label = "rise")
  Column(Modifier.fillMaxSize().background(ink)) {
    Column(Modifier.weight(1f).statusBarsPadding().padding(horizontal = 28.dp)) {
      Row(
        verticalAlignment = Alignment.CenterVertically,
        horizontalArrangement = Arrangement.spacedBy(9.dp),
        modifier = Modifier.padding(top = 8.dp),
      ) {
        Logo(30.dp, paper, assembly = assembly)
        Text(
          "Condr",
          style = MaterialTheme.typography.titleMedium,
          fontWeight = FontWeight.SemiBold,
          color = paper,
          modifier = Modifier.alpha(logoAlpha),
        )
      }
      Spacer(Modifier.weight(1f))
      WelcomeStage(
        playing = stage >= 2,
        modifier = Modifier.fillMaxWidth().height(250.dp).alpha(stageAlpha),
      )
      Spacer(Modifier.weight(1f))
      Column(Modifier.alpha(wordsAlpha).offset(y = wordsRise)) {
        Text(
          "Every agent,\nwithin reach.",
          style = MaterialTheme.typography.displaySmall,
          fontWeight = FontWeight.Bold,
          color = paper,
          lineHeight = 44.sp,
        )
        Spacer(Modifier.height(14.dp))
        Text(
          "See which agent waits for you, read what it did, and answer it. From anywhere.",
          style = MaterialTheme.typography.titleMedium,
          color = paper.copy(alpha = 0.6f),
        )
      }
    }
    // The steps' footer, so the button sits where theirs do.
    StepFooter(
      primary = {
        Button(
          onClick = next,
          colors = ButtonDefaults.buttonColors(containerColor = paper, contentColor = ink),
          modifier = Modifier.fillMaxWidth().height(56.dp).alpha(wordsAlpha),
        ) {
          Text(
            "Pair with your computer",
            style = MaterialTheme.typography.titleMedium,
            fontWeight = FontWeight.SemiBold,
          )
        }
      }
    )
  }
}

// The code

/**
 * Where the pairing code is on the computer, and the two ways to take it: the camera or a pasted
 * link.
 */
@OptIn(ExperimentalMaterial3Api::class)
@Composable
private fun InviteStep(
  cancel: (() -> Unit)?,
  back: (() -> Unit)?,
  found: (String) -> Unit,
  tryDemo: (() -> Unit)? = null,
) {
  val context = LocalContext.current
  val stores = LocalStores.current
  var pasting by remember { mutableStateOf(false) }
  // A demo scene shows the step as a phone with Google Play services would.
  val scannable = remember { stores.scene != null || canScan(context) }
  val snackbars = remember { SnackbarHostState() }
  val scope = rememberCoroutineScope()
  /** The system's scanner closes on the first code it sees: one that isn't an Invite says so. */
  fun scanCode() {
    scan(
      context,
      found = found,
      other = {
        scope.launch {
          val again =
            snackbars.showSnackbar(
              "That isn't a pairing code.",
              actionLabel = "Scan again",
              duration = SnackbarDuration.Long,
            )
          if (again == SnackbarResult.ActionPerformed) scanCode()
        }
      },
      failed = {
        scope.launch { snackbars.showSnackbar("The scanner didn't open. Paste the link instead.") }
      },
    )
  }
  Scaffold(
    snackbarHost = { SnackbarHost(snackbars) },
    topBar = {
      TopAppBar(
        title = {},
        navigationIcon = {
          if (back != null)
            IconButton(onClick = back) {
              Icon(Icons.AutoMirrored.Filled.ArrowBack, contentDescription = "Back")
            }
        },
        actions = {
          if (cancel != null)
            IconButton(onClick = cancel) { Icon(Icons.Filled.Close, contentDescription = "Cancel") }
        },
      )
    },
    bottomBar = {
      StepFooter(
        primary = {
          if (scannable) {
            Button(
              onClick = ::scanCode,
              modifier = Modifier.fillMaxWidth().height(56.dp),
            ) {
              Icon(painterResource(R.drawable.ic_qr_scanner), contentDescription = null)
              Spacer(Modifier.size(8.dp))
              Text("Scan code", style = MaterialTheme.typography.titleMedium)
            }
          } else {
            Button(onClick = { pasting = true }, modifier = Modifier.fillMaxWidth().height(56.dp)) {
              Text("Paste a link", style = MaterialTheme.typography.titleMedium)
            }
          }
        },
        secondary = {
          if (scannable) TextButton(onClick = { pasting = true }) { Text("Paste a link instead") }
        },
        below = {
          if (tryDemo != null) {
            TextButton(onClick = tryDemo) {
              Text(
                buildAnnotatedString {
                  withStyle(SpanStyle(color = MaterialTheme.colorScheme.onSurfaceVariant)) {
                    append("No computer to pair? ")
                  }
                  append("Try demo")
                },
                style = MaterialTheme.typography.bodySmall,
              )
            }
          }
        },
      )
    },
  ) { insets ->
    Column(
      Modifier.fillMaxSize()
        .padding(insets)
        .verticalScroll(rememberScrollState())
        .padding(horizontal = 24.dp, vertical = 8.dp),
      verticalArrangement = Arrangement.spacedBy(28.dp),
    ) {
      StepTitle("Show the pairing code on your computer")
      InviteIllustration()
      Column(verticalArrangement = Arrangement.spacedBy(16.dp)) {
        NumberedStep(1, bold("Open ", "Condr", " on your computer."))
        NumberedStep(2, bold("Go to ", "Settings › Paired devices", "."))
        NumberedStep(3, bold("Choose ", "Show pairing code", "."))
      }
    }
  }
  if (pasting) {
    PasteLink(dismiss = { pasting = false }) { link ->
      pasting = false
      found(link)
    }
  }
}

private fun bold(before: String, strong: String, after: String) = buildAnnotatedString {
  append(before)
  withStyle(SpanStyle(fontWeight = FontWeight.SemiBold)) { append(strong) }
  append(after)
}

/**
 * Google's code scanner: the system's own UI, no camera permission of the app's (ADR 0048). It
 * closes on the first QR code it sees, an Invite or `other`; a scanner that can't open is `failed`.
 */
private fun scan(context: Context, found: (String) -> Unit, other: () -> Unit, failed: () -> Unit) {
  val options =
    GmsBarcodeScannerOptions.Builder()
      .setBarcodeFormats(Barcode.FORMAT_QR_CODE)
      .enableAutoZoom()
      .build()
  GmsBarcodeScanning.getClient(context, options)
    .startScan()
    .addOnSuccessListener { barcode ->
      val text = barcode.rawValue
      if (text != null && isInvite(text)) found(text) else other()
    }
    .addOnFailureListener { failed() }
}

/** The desktop's Settings window as the person will find it, invite code showing. */
@Composable
private fun InviteIllustration() {
  val white = Color.White
  Surface(
    shape = RoundedCornerShape(16.dp),
    color = Color(0xFF16181C),
    contentColor = white,
    border = BorderStroke(1.dp, white.copy(alpha = 0.1f)),
    shadowElevation = 12.dp,
    modifier =
      Modifier.fillMaxWidth().clearAndSetSemantics {
        contentDescription =
          "Condr's Settings on your computer, with Paired devices open and a pairing code showing"
      },
  ) {
    // Desktop sizes: the text's own line height, not Material's roomy one.
    CompositionLocalProvider(LocalTextStyle provides TextStyle.Default) { WindowContent() }
  }
}

@Composable
private fun WindowContent() {
  val white = Color.White
  Column {
    Box(Modifier.fillMaxWidth().padding(horizontal = 12.dp, vertical = 10.dp)) {
      Row(horizontalArrangement = Arrangement.spacedBy(6.dp)) {
        for (color in listOf(0xFFFF5F57, 0xFFFEBC2E, 0xFF28C840)) {
          Box(Modifier.size(9.dp).background(Color(color), CircleShape))
        }
      }
      Text(
        "Settings",
        fontSize = 11.sp,
        fontWeight = FontWeight.SemiBold,
        color = white.copy(alpha = 0.55f),
        modifier = Modifier.align(Alignment.Center),
      )
    }
    Row(Modifier.height(170.dp)) {
      Column(
        Modifier.width(120.dp).padding(horizontal = 8.dp),
        verticalArrangement = Arrangement.spacedBy(3.dp),
      ) {
        for (item in listOf("General", "Appearance", "Terminal", "Paired devices", "About")) {
          val selected = item == "Paired devices"
          Text(
            item,
            fontSize = 10.5.sp,
            fontWeight = if (selected) FontWeight.SemiBold else FontWeight.Normal,
            color = white.copy(alpha = if (selected) 1f else 0.55f),
            maxLines = 1,
            modifier =
              Modifier.fillMaxWidth()
                .background(
                  if (selected) white.copy(alpha = 0.1f) else Color.Transparent,
                  RoundedCornerShape(6.dp),
                )
                .padding(horizontal = 8.dp, vertical = 4.dp),
          )
        }
      }
      Box(Modifier.width(1.dp).fillMaxHeight().background(white.copy(alpha = 0.08f)))
      Column(
        Modifier.weight(1f).fillMaxHeight().padding(vertical = 14.dp),
        horizontalAlignment = Alignment.CenterHorizontally,
        verticalArrangement = Arrangement.spacedBy(10.dp, Alignment.CenterVertically),
      ) {
        Image(
          painterResource(R.drawable.qr_invite),
          contentDescription = null,
          modifier =
            Modifier.size(106.dp).background(white, RoundedCornerShape(8.dp)).padding(7.dp),
        )
        Column(
          horizontalAlignment = Alignment.CenterHorizontally,
          verticalArrangement = Arrangement.spacedBy(3.dp),
        ) {
          Text("Scan with Condr on your phone", fontSize = 10.sp, fontWeight = FontWeight.Medium)
          Text("Expires in 9:12", fontSize = 9.sp, color = white.copy(alpha = 0.5f))
        }
      }
    }
  }
}

/**
 * The link under the code, pasted, in a dialog: it keeps clear of the keyboard on every Android.
 */
@Composable
private fun PasteLink(dismiss: () -> Unit, done: (String) -> Unit) {
  var link by remember { mutableStateOf("") }
  var problem by remember { mutableStateOf<String?>(null) }
  val focus = remember { FocusRequester() }
  LaunchedEffect(Unit) { focus.requestFocus() }
  val submit = {
    // A paste may bring whitespace or invisible marks along; a link is letters, digits and a few
    // signs.
    val trimmed = link.filter { it.isLetterOrDigit() || it in "-_.:@/[]" }
    if (isInvite(trimmed)) done(trimmed)
    else
      problem =
        if (trimmed.isEmpty()) "Paste the link first."
        else "That isn't a pairing link. It starts with p2p:// or tcp://."
  }
  AlertDialog(
    onDismissRequest = dismiss,
    title = { Text("Paste a link") },
    text = {
      Column(verticalArrangement = Arrangement.spacedBy(12.dp)) {
        Text("Copy the link under the code on your computer and paste it here.")
        OutlinedTextField(
          value = link,
          onValueChange = {
            link = it
            problem = null
          },
          placeholder = { Text("p2p://… or tcp://…") },
          isError = problem != null,
          supportingText = problem?.let { { Text(it) } },
          textStyle = MaterialTheme.typography.bodyMedium.copy(fontFamily = FontFamily.Monospace),
          keyboardOptions = KeyboardOptions(imeAction = ImeAction.Done, autoCorrectEnabled = false),
          keyboardActions = KeyboardActions(onDone = { submit() }),
          modifier = Modifier.fillMaxWidth().focusRequester(focus),
        )
      }
    },
    confirmButton = { TextButton(onClick = submit) { Text("Continue") } },
    dismissButton = { TextButton(onClick = dismiss) { Text("Cancel") } },
  )
}

// The name

/** This phone's name, as the computer's Paired devices will list it (ADR 0040). */
@OptIn(ExperimentalMaterial3Api::class)
@Composable
private fun NameStep(name: String, onName: (String) -> Unit, back: () -> Unit, next: () -> Unit) {
  val session = LocalStores.current.session
  val trimmed = name.trim()
  val focus = remember { FocusRequester() }
  LaunchedEffect(Unit) { focus.requestFocus() }
  Scaffold(
    topBar = {
      TopAppBar(
        title = {},
        navigationIcon = {
          IconButton(onClick = back) {
            Icon(Icons.AutoMirrored.Filled.ArrowBack, contentDescription = "Back")
          }
        },
      )
    },
    bottomBar = {
      StepFooter(
        primary = {
          Button(
            onClick = next,
            enabled = trimmed.isNotEmpty(),
            modifier = Modifier.fillMaxWidth().height(56.dp),
          ) {
            Text("Continue", style = MaterialTheme.typography.titleMedium)
          }
        }
      )
    },
  ) { insets ->
    Column(
      Modifier.fillMaxSize().padding(insets).padding(horizontal = 24.dp, vertical = 8.dp),
      verticalArrangement = Arrangement.spacedBy(28.dp),
    ) {
      StepTitle("Name this phone", "Your computer lists it under Paired devices by this name.")
      OutlinedTextField(
        value = name,
        onValueChange = onName,
        singleLine = true,
        label = { Text("Name") },
        leadingIcon = { Icon(painterResource(R.drawable.ic_phone), contentDescription = null) },
        trailingIcon = {
          if (name.isNotEmpty()) {
            IconButton(onClick = { onName("") }) {
              Icon(Icons.Filled.Close, contentDescription = "Clear")
            }
          }
        },
        keyboardOptions = KeyboardOptions(imeAction = ImeAction.Next),
        keyboardActions = KeyboardActions(onNext = { if (trimmed.isNotEmpty()) next() }),
        modifier = Modifier.fillMaxWidth().focusRequester(focus),
      )
      PairedPreview(name = trimmed.ifEmpty { "Android" }, key = session.fingerprint)
    }
  }
}

/**
 * The row the computer's Settings › Paired devices will show, drawn as the desktop draws it and
 * following the name as it is typed.
 */
@Composable
private fun PairedPreview(name: String, key: String) {
  val white = Color.White
  Surface(
    shape = RoundedCornerShape(14.dp),
    color = Color(0xFF16181C),
    contentColor = white,
    border = BorderStroke(1.dp, white.copy(alpha = 0.1f)),
    modifier =
      Modifier.fillMaxWidth().clearAndSetSemantics {
        contentDescription = "Your computer will list this phone as $name"
      },
  ) {
    Column(Modifier.padding(14.dp), verticalArrangement = Arrangement.spacedBy(10.dp)) {
      CompositionLocalProvider(LocalTextStyle provides TextStyle.Default) { PreviewRow(name, key) }
    }
  }
}

@Composable
private fun PreviewRow(name: String, key: String) {
  val white = Color.White
  Text(
    "Paired devices on your computer",
    fontSize = 11.sp,
    fontWeight = FontWeight.SemiBold,
    color = white.copy(alpha = 0.5f),
  )
  Row(
    verticalAlignment = Alignment.CenterVertically,
    horizontalArrangement = Arrangement.spacedBy(12.dp),
  ) {
    Box(
      Modifier.size(30.dp).background(white.copy(alpha = 0.08f), RoundedCornerShape(7.dp)),
      contentAlignment = Alignment.Center,
    ) {
      Icon(
        painterResource(R.drawable.ic_phone),
        contentDescription = null,
        modifier = Modifier.size(18.dp),
      )
    }
    Column(verticalArrangement = Arrangement.spacedBy(2.dp)) {
      Text(name, fontSize = 13.sp, fontWeight = FontWeight.SemiBold, maxLines = 1)
      Row(
        verticalAlignment = Alignment.CenterVertically,
        horizontalArrangement = Arrangement.spacedBy(5.dp),
      ) {
        Box(Modifier.size(6.dp).background(Color(TerminalPalette.GREEN), CircleShape))
        val muted = white.copy(alpha = 0.5f)
        Text("Connected", fontSize = 11.sp, color = muted)
        Text("·", fontSize = 11.sp, color = muted)
        Text(key.take(8), fontSize = 11.sp, fontFamily = FontFamily.Monospace, color = muted)
      }
    }
  }
}

// Connecting

/** The handshake with the computer: under way, done, or failed. */
class Pairing(
  private val scope: kotlinx.coroutines.CoroutineScope,
  private val session: Session,
  private val preferences: Preferences,
  private val link: String,
  private val name: String,
) {
  sealed interface Phase {
    data object Pairing : Phase

    data class Paired(val device: SavedDevice) : Phase

    data class Failed(val reason: String) : Phase
  }

  var phase: Phase by mutableStateOf(Phase.Pairing)
    private set

  private var job: Job? = null

  init {
    start()
  }

  val paired: SavedDevice?
    get() = (phase as? Phase.Paired)?.device

  fun retry() {
    if (phase is Phase.Failed) start()
  }

  private fun start() {
    phase = Phase.Pairing
    job?.cancel()
    job = scope.launch {
      try {
        val device = session.pair(link, name)
        preferences.phoneName = name
        phase = Phase.Paired(device)
      } catch (e: CancellationException) {
        throw e
      } catch (e: Exception) {
        phase = Phase.Failed(e.message ?: "Couldn't pair.")
      }
    }
  }
}

/** The handshake, then the computer's score and Agents arriving. */
@Composable
private fun ConnectingStep(
  pairing: Pairing,
  firstLaunch: Boolean,
  finish: () -> Unit,
  back: () -> Unit,
) {
  val session = LocalStores.current.session
  val paired = pairing.paired
  /** The new computer's Agents, as they arrive with its Bootstrap. */
  val found: List<AgentItem> =
    paired?.let { session.device(it.address) }?.let { session.agents(it).byUrgency() }
      ?: emptyList()
  // Back only from a failure: once paired, the button is the way on, as on iOS.
  BackHandler(enabled = pairing.phase !is Pairing.Phase.Failed) {}
  val haptics = LocalHapticFeedback.current
  LaunchedEffect(paired) {
    if (paired != null) haptics.performHapticFeedback(HapticFeedbackType.Confirm)
  }
  Scaffold(
    bottomBar = {
      when (pairing.phase) {
        is Pairing.Phase.Paired ->
          StepFooter(
            primary = {
              Button(onClick = finish, modifier = Modifier.fillMaxWidth().height(56.dp)) {
                Text(
                  if (firstLaunch) "Show my agents" else "Done",
                  style = MaterialTheme.typography.titleMedium,
                )
              }
            }
          )
        is Pairing.Phase.Failed ->
          StepFooter(
            primary = {
              Button(
                onClick = { pairing.retry() },
                modifier = Modifier.fillMaxWidth().height(56.dp),
              ) {
                Text("Try again", style = MaterialTheme.typography.titleMedium)
              }
            },
            secondary = { TextButton(onClick = back) { Text("Back") } },
          )
        Pairing.Phase.Pairing -> {}
      }
    }
  ) { insets ->
    Column(
      Modifier.fillMaxSize()
        .padding(insets)
        .verticalScroll(rememberScrollState())
        .padding(horizontal = 28.dp),
      horizontalAlignment = Alignment.CenterHorizontally,
    ) {
      Spacer(Modifier.height(40.dp))
      var rose by remember { mutableStateOf(0) }
      var shown by remember { mutableStateOf(0) }
      LaunchedEffect(found.size, paired) {
        if (paired == null) return@LaunchedEffect
        // Each Agent's bar rises, then its row, as they come.
        for (index in rose until found.size) {
          kotlinx.coroutines.delay(if (index == 0) 250L else 110L)
          rose = index + 1
        }
        for (index in shown until found.size) {
          kotlinx.coroutines.delay(if (index == 0) 300L else 140L)
          shown = index + 1
        }
      }
      Crossfade(paired, Modifier.height(230.dp), label = "picture") { device ->
        Box(Modifier.fillMaxSize(), contentAlignment = Alignment.Center) {
          if (device != null) Rising(found.map { it.agent.status }, rose, device.label)
          else Nearby()
        }
      }
      val (title, detail) =
        when (val phase = pairing.phase) {
          Pairing.Phase.Pairing -> "Pairing…" to "Exchanging keys with your computer."
          is Pairing.Phase.Paired ->
            "Paired with ${phase.device.label}" to "Its agents show up here now."
          is Pairing.Phase.Failed -> "Couldn't pair" to phase.reason
        }
      Spacer(Modifier.height(18.dp))
      Text(
        title,
        style = MaterialTheme.typography.headlineMedium,
        fontWeight = FontWeight.Bold,
        textAlign = TextAlign.Center,
      )
      Spacer(Modifier.height(10.dp))
      Text(detail, color = MaterialTheme.colorScheme.onSurfaceVariant, textAlign = TextAlign.Center)
      AnimatedVisibility(visible = paired != null && found.isNotEmpty()) {
        Surface(
          shape = RoundedCornerShape(22.dp),
          color = MaterialTheme.colorScheme.surfaceContainerHigh,
          modifier = Modifier.fillMaxWidth().padding(top = 32.dp),
        ) {
          Column {
            found.forEachIndexed { index, item ->
              // Each row arrives after its bar, rising into place.
              val arrived by
                animateFloatAsState(
                  if (shown > index) 1f else 0f,
                  spring(dampingRatio = 0.7f, stiffness = 300f),
                  label = "row",
                )
              ListItem(
                modifier = Modifier.alpha(arrived).offset(y = 12.dp * (1 - arrived)),
                headlineContent = {
                  Text(item.pane.name, fontWeight = FontWeight.SemiBold, maxLines = 1)
                },
                supportingContent = {
                  Text(
                    agentStatusLine(
                      item.agent.label,
                      item.agent.status,
                      item.agent.detail,
                      kind = true,
                    ),
                    maxLines = 1,
                  )
                },
                leadingContent = {
                  AgentAvatar(
                    kind = item.agent.kind,
                    status = item.agent.status,
                    side = 34.dp,
                    surface = MaterialTheme.colorScheme.surfaceContainerHigh,
                  )
                },
                colors = ListItemDefaults.colors(containerColor = Color.Transparent),
              )
              if (index < found.lastIndex) HorizontalDivider(Modifier.padding(start = 66.dp))
            }
          }
        }
      }
      Spacer(Modifier.height(40.dp))
    }
  }
}

/**
 * A step's buttons at the bottom: the one that moves on, then a slot of fixed height for a quieter
 * alternative, so the button sits at the same place on every step; `below` goes under the slot.
 */
@Composable
private fun StepFooter(
  primary: @Composable () -> Unit,
  secondary: @Composable () -> Unit = {},
  below: @Composable () -> Unit = {},
) {
  Column(
    Modifier.fillMaxWidth()
      .navigationBarsPadding()
      .imePadding()
      .padding(horizontal = 24.dp)
      .padding(top = 12.dp),
    horizontalAlignment = Alignment.CenterHorizontally,
    verticalArrangement = Arrangement.spacedBy(4.dp),
  ) {
    primary()
    Box(Modifier.fillMaxWidth().height(48.dp), contentAlignment = Alignment.Center) { secondary() }
    below()
  }
}

/** The phone and the computer on one network: Wi-Fi rings spreading between them. */
@Composable
private fun Nearby() {
  val still = reducedMotion()
  var time by remember { mutableFloatStateOf(0f) }
  LaunchedEffect(still) {
    if (still) return@LaunchedEffect
    while (true) withInfiniteAnimationFrameMillis { time = it / 1000f }
  }
  val ink = MaterialTheme.colorScheme.onSurface
  val surface = MaterialTheme.colorScheme.surface
  Box(
    Modifier.fillMaxWidth().height(230.dp).clipToBounds().clearAndSetSemantics {},
    contentAlignment = Alignment.Center,
  ) {
    Canvas(Modifier.fillMaxSize()) {
      for (ring in 0 until 4) {
        val progress = (time / 3.2f + ring / 4f) % 1f
        drawCircle(
          ink.copy(alpha = (1 - progress) * 0.28f),
          radius = (70 + 220 * progress).dp.toPx() / 2,
          style = Stroke(1.2.dp.toPx()),
        )
      }
    }
    Box(
      Modifier.size(64.dp)
        .background(surface, CircleShape)
        .border(1.dp, ink.copy(alpha = 0.12f), CircleShape),
      contentAlignment = Alignment.Center,
    ) {
      Icon(painterResource(R.drawable.ic_wifi), contentDescription = null, Modifier.size(28.dp))
    }
    Row(Modifier.fillMaxWidth(), horizontalArrangement = Arrangement.SpaceBetween) {
      for (glyph in listOf(R.drawable.ic_phone, R.drawable.ic_tab_devices)) {
        Box(Modifier.size(92.dp).background(surface, CircleShape), Alignment.Center) {
          Icon(
            painterResource(glyph),
            contentDescription = null,
            tint = ink.copy(alpha = 0.8f),
            modifier = Modifier.size(44.dp),
          )
        }
      }
    }
  }
}

/** The new computer's bars rising one by one, then its name. */
@Composable
private fun Rising(statuses: List<uniffi.condr_mobile.AgentStatus>, shown: Int, label: String) {
  Column(verticalArrangement = Arrangement.spacedBy(12.dp)) {
    Row(
      verticalAlignment = Alignment.Bottom,
      horizontalArrangement = Arrangement.spacedBy(9.dp),
      modifier = Modifier.height(120.dp),
    ) {
      statuses.forEachIndexed { index, status ->
        val target =
          if (index < shown) dev.condr.companion.ui.theme.barHeight(status, 120f, 0.5f) else 0f
        val height by animateFloatAsState(target, tween(550), label = "bar")
        val color =
          if (
            status == uniffi.condr_mobile.AgentStatus.IDLE ||
              status == uniffi.condr_mobile.AgentStatus.UNKNOWN
          )
            MaterialTheme.colorScheme.outlineVariant
          else dev.condr.companion.ui.theme.tintOf(status)
        Box(
          Modifier.size(width = 24.dp, height = height.dp)
            .background(color, RoundedCornerShape(5.dp))
        )
      }
    }
    Text(
      label,
      style = MaterialTheme.typography.titleSmall,
      color = MaterialTheme.colorScheme.onSurfaceVariant,
    )
  }
}
