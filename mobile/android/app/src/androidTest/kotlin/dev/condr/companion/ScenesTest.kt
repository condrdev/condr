package dev.condr.companion

import android.content.Intent
import androidx.compose.ui.test.assertIsDisplayed
import androidx.compose.ui.test.hasText
import androidx.compose.ui.test.junit4.createEmptyComposeRule
import androidx.compose.ui.test.onNodeWithText
import androidx.test.core.app.ActivityScenario
import androidx.test.core.app.ApplicationProvider
import org.junit.Rule
import org.junit.Test

/** Every demo scene opens on what it is for, as the iOS app's UI tests check (ADR 0048). */
class ScenesTest {
  @get:Rule val compose = createEmptyComposeRule()

  private fun open(
    scene: String,
    vararg extras: Pair<String, String>,
  ): ActivityScenario<MainActivity> {
    val intent =
      Intent(ApplicationProvider.getApplicationContext(), MainActivity::class.java)
        .putExtra("scene", scene)
    for ((key, value) in extras) intent.putExtra(key, value)
    return ActivityScenario.launch(intent)
  }

  private fun shows(text: String, substring: Boolean = false) {
    compose.waitUntil(5_000) {
      compose.onAllNodes(hasText(text, substring = substring)).fetchSemanticsNodes().isNotEmpty()
    }
  }

  @Test
  fun welcomeOffersPairing() {
    open("welcome").use {
      shows("Pair with your computer")
      compose.onNodeWithText("Pair with your computer").assertIsDisplayed()
    }
  }

  @Test
  fun agentsListsWhatNeedsYou() {
    open("agents").use {
      shows("Needs you")
      shows("Implement statistics page")
      shows("Add usage chart")
    }
  }

  @Test
  fun emptyHasNothingNeedingYou() {
    open("empty").use { shows("Nothing needs you") }
  }

  @Test
  fun devicesListsEveryComputer() {
    open("devices").use {
      shows("macmini")
      shows("studio")
      shows("Pair a computer")
    }
  }

  @Test
  fun offlineSaysWhyTheComputerIsOutOfReach() {
    open("offline").use {
      shows("Can't reach studio")
      shows("Try again")
    }
  }

  @Test
  fun terminalShowsThePaneAndItsKeys() {
    open("terminal").use {
      shows("Implement statistics page")
      shows("Reply, or pick with", substring = true)
      shows("esc")
    }
  }

  @Test
  fun terminalErrorShowsTheErrorToast() {
    open("terminal-error").use { shows("Couldn't send to macmini", substring = true) }
  }

  @Test
  fun bannerOffersToOpenThePane() {
    open("banner").use {
      shows("Claude needs you")
      shows("Open")
    }
  }

  @Test
  fun lockAsksToUnlock() {
    open("lock").use { shows("Unlock") }
  }

  @Test
  fun settingsNamesThePhone() {
    open("settings").use {
      shows("Text size")
      shows("If you lose this phone")
    }
  }

  @Test
  fun pairedShowsTheNewComputer() {
    open("paired").use {
      shows("Paired with macmini")
      shows("Show my agents")
    }
  }
}
