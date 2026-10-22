package dev.condr.companion.ui.pairing

import androidx.compose.foundation.layout.Box
import androidx.compose.foundation.layout.fillMaxSize
import androidx.compose.material3.Text
import androidx.compose.runtime.Composable
import androidx.compose.ui.Alignment
import androidx.compose.ui.Modifier

/** Nothing is paired yet: the way in (ADR 0040). */
@Composable
fun WelcomeScreen() {
  Box(Modifier.fillMaxSize(), contentAlignment = Alignment.Center) { Text("Pair a computer") }
}
