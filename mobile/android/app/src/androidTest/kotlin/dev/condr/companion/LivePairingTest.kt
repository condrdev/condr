package dev.condr.companion

import androidx.compose.ui.test.assertIsDisplayed
import androidx.compose.ui.test.hasText
import androidx.compose.ui.test.junit4.createAndroidComposeRule
import androidx.compose.ui.test.onFirst
import androidx.compose.ui.test.onNodeWithText
import androidx.compose.ui.test.performClick
import androidx.compose.ui.test.performTextInput
import androidx.test.platform.app.InstrumentationRegistry
import org.junit.Assume.assumeTrue
import org.junit.Rule
import org.junit.Test

/**
 * Pairing with a real Server, end to end (ADR 0048): skipped unless an Invite is given.
 *
 *     ./gradlew :app:connectedDebugAndroidTest \
 *       -Pandroid.testInstrumentationRunnerArguments.invite='tcp://…@10.0.2.2:7419'
 *
 * The app must not be paired yet (`adb uninstall dev.condr.companion` first). Without the argument
 * the build leaves this class out of the run.
 */
class LivePairingTest {
  @get:Rule val compose = createAndroidComposeRule<MainActivity>()

  private val invite: String? = InstrumentationRegistry.getArguments().getString("invite")

  @Test
  fun pairsOverAPastedLinkAndShowsTheComputersAgents() {
    assumeTrue(
      "Pass an Invite with -Pandroid.testInstrumentationRunnerArguments.invite=…",
      !invite.isNullOrBlank(),
    )
    compose.onNodeWithText("Pair with your computer").performClick()
    compose.waitUntil(5_000) {
      compose
        .onAllNodes(hasText("Paste a link", substring = true))
        .fetchSemanticsNodes()
        .isNotEmpty()
    }
    // Either the only button, without Google Play services, or the alternative under Scan.
    compose.onAllNodes(hasText("Paste a link", substring = true)).onFirst().performClick()
    compose.onNodeWithText("p2p://… or tcp://…").performTextInput(invite!!)
    compose.onNodeWithText("Continue").performClick()
    // The first pairing asks for the phone's name, offered as the system's.
    compose.waitUntil(5_000) {
      compose.onAllNodes(hasText("Name this phone")).fetchSemanticsNodes().isNotEmpty()
    }
    compose.onNodeWithText("Continue").performClick()
    compose.waitUntil(30_000) {
      compose
        .onAllNodes(hasText("Paired with", substring = true))
        .fetchSemanticsNodes()
        .isNotEmpty()
    }
    compose.onNodeWithText("Show my agents").performClick()
    compose.waitUntil(5_000) {
      compose.onAllNodes(hasText("Needs you")).fetchSemanticsNodes().isNotEmpty()
    }
    compose.onNodeWithText("Needs you").assertIsDisplayed()
  }
}
