package dev.condr.companion.ui.terminal

import androidx.compose.foundation.layout.padding
import androidx.compose.material3.ExperimentalMaterial3Api
import androidx.compose.material3.Scaffold
import androidx.compose.material3.Text
import androidx.compose.material3.TopAppBar
import androidx.compose.runtime.Composable
import androidx.compose.ui.Modifier
import androidx.compose.ui.unit.dp
import dev.condr.companion.model.PaneRef
import dev.condr.companion.model.name
import dev.condr.companion.ui.LocalStores

/** The Pane's terminal (ADR 0041); the grid comes with the renderer. */
@OptIn(ExperimentalMaterial3Api::class)
@Composable
fun TerminalPage(ref: PaneRef) {
  val session = LocalStores.current.session
  val pane = session.pane(ref)
  Scaffold(topBar = { TopAppBar(title = { Text(pane?.name ?: "Terminal") }) }) { insets ->
    Text("Terminal for ${ref.address} #${ref.pane}", Modifier.padding(insets).padding(16.dp))
  }
}
