package dev.condr.companion

import dev.condr.companion.model.Preferences
import dev.condr.companion.model.Push
import kotlinx.coroutines.Dispatchers
import kotlinx.coroutines.test.StandardTestDispatcher
import kotlinx.coroutines.test.resetMain
import kotlinx.coroutines.test.runTest
import kotlinx.coroutines.test.setMain
import org.junit.After
import org.junit.Assert.assertEquals
import org.junit.Assert.assertFalse
import org.junit.Assert.assertTrue
import org.junit.Before
import org.junit.Test
import uniffi.condr_mobile.CompanionInterface

/** The registration with Condr Cloud against a fake sender (ADR 0047, ADR 0048). */
@OptIn(kotlinx.coroutines.ExperimentalCoroutinesApi::class)
class PushTest {
  private val dispatcher = StandardTestDispatcher()
  private val requests = ArrayList<Triple<String, String, String?>>()
  private var answer: Pair<Int, String>? = 200 to """{"code":"200"}"""
  private val flags = ArrayList<Boolean>()
  private val client: CompanionInterface = FakeClient { flags.add(it) }
  private val preferences = Preferences.inMemory()

  @Before fun main() = Dispatchers.setMain(dispatcher)

  @After fun reset() = Dispatchers.resetMain()

  private fun push() =
    Push(client, preferences, cloud = "https://cloud.test") { method, url, _, body ->
      requests.add(Triple(method, url, body))
      answer
    }

  @Test
  fun registersOncePermissionServersAndTokenAreAllThere() = runTest {
    val push = push()
    push.update(granted = true, servers = listOf("mac"))
    assertTrue(requests.isEmpty())
    push.received("tok")
    assertEquals(1, requests.size)
    val (method, url, body) = requests[0]
    assertEquals("PUT", method)
    assertEquals("https://cloud.test/api/push/device", url)
    assertEquals("""{"platform":"FCM","token":"tok","servers":["mac"]}""", body)
    assertTrue(push.on)
    assertEquals(true, flags.last())
  }

  @Test
  fun losingThePermissionRemovesARegistrationOnceMade() = runTest {
    val push = push()
    push.received("tok")
    push.update(granted = true, servers = listOf("mac"))
    push.update(granted = false, servers = listOf("mac"))
    assertEquals(listOf("PUT", "DELETE"), requests.map { it.first })
    assertFalse(push.on)
    assertFalse(preferences.pushRegistered)
  }

  @Test
  fun aRefusalLeavesTheFlagOffAndTriesAgainNextTime() = runTest {
    answer = 200 to """{"code":"401"}"""
    val push = push()
    push.received("tok")
    push.update(granted = true, servers = listOf("mac"))
    assertFalse(push.on)
    answer = 200 to """{"code":"200"}"""
    push.update(granted = true, servers = listOf("mac"))
    assertEquals(2, requests.size)
    assertTrue(push.on)
  }

  @Test
  fun noTokenMeansNothingReachesThePhone() = runTest {
    val push = push()
    push.update(granted = true, servers = listOf("mac"))
    push.failed()
    assertFalse(push.available)
    assertFalse(push.on)
    assertTrue(requests.isEmpty())
  }
}
