package dev.condr.companion.ui.settings

import androidx.compose.foundation.clickable
import androidx.compose.foundation.layout.Column
import androidx.compose.foundation.layout.fillMaxSize
import androidx.compose.foundation.layout.padding
import androidx.compose.material3.ExperimentalMaterial3Api
import androidx.compose.material3.LargeTopAppBar
import androidx.compose.material3.ListItem
import androidx.compose.material3.Scaffold
import androidx.compose.material3.Switch
import androidx.compose.material3.Text
import androidx.compose.runtime.Composable
import androidx.compose.ui.Modifier
import androidx.compose.ui.unit.dp
import dev.condr.companion.model.SettingsRoute
import dev.condr.companion.ui.LocalStores

@OptIn(ExperimentalMaterial3Api::class)
@Composable
fun SettingsScreen() {
  val stores = LocalStores.current
  val preferences = stores.preferences
  val router = stores.router
  Scaffold(topBar = { LargeTopAppBar(title = { Text("Settings") }) }) { insets ->
    Column(Modifier.fillMaxSize().padding(insets)) {
      ListItem(
        headlineContent = { Text("This phone's name") },
        supportingContent = { Text(preferences.phoneName.ifBlank { "Android" }) },
        modifier = Modifier.clickable { router.settingsPath.add(SettingsRoute.PhoneName) },
      )
      ListItem(
        headlineContent = { Text("Lock") },
        supportingContent = { Text("Ask for your fingerprint, face or PIN when the app opens") },
        trailingContent = {
          Switch(checked = preferences.lockOn, onCheckedChange = { preferences.lockOn = it })
        },
      )
      ListItem(
        headlineContent = { Text("This phone's key") },
        supportingContent = { Text(stores.session.fingerprint) },
      )
      ListItem(
        headlineContent = { Text("If you lose this phone") },
        modifier = Modifier.clickable { router.settingsPath.add(SettingsRoute.LostPhone) },
      )
    }
  }
}

@OptIn(ExperimentalMaterial3Api::class)
@Composable
fun PhoneNameScreen() {
  Scaffold(topBar = { LargeTopAppBar(title = { Text("This phone's name") }) }) { insets ->
    Text("Coming next", Modifier.padding(insets).padding(16.dp))
  }
}

@OptIn(ExperimentalMaterial3Api::class)
@Composable
fun LostPhoneScreen() {
  Scaffold(topBar = { LargeTopAppBar(title = { Text("If you lose this phone") }) }) { insets ->
    Text("Coming next", Modifier.padding(insets).padding(16.dp))
  }
}
