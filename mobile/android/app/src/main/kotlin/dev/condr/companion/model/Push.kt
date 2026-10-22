package dev.condr.companion.model

import androidx.compose.runtime.getValue
import androidx.compose.runtime.mutableStateOf
import androidx.compose.runtime.setValue
import java.net.HttpURLConnection
import java.net.URL
import kotlinx.coroutines.CoroutineScope
import kotlinx.coroutines.Deferred
import kotlinx.coroutines.Dispatchers
import kotlinx.coroutines.SupervisorJob
import kotlinx.coroutines.async
import kotlinx.coroutines.withContext
import kotlinx.serialization.Serializable
import kotlinx.serialization.json.Json
import uniffi.condr_mobile.CompanionInterface
import uniffi.condr_mobile.cloudUrl

/** The Pane a notification names: its Server's Device id and the Pane id (ADR 0047). */
data class PushedPane(val device: String, val pane: ULong) {
  companion object {
    fun from(device: String?, pane: String?): PushedPane? {
      val id = pane?.toULongOrNull() ?: return null
      return PushedPane(device ?: return null, id)
    }
  }
}

/** The answer to a request: its HTTP status and body, or null when it never came. */
typealias Send =
  suspend (method: String, url: String, headers: Map<String, String>, body: String?) -> Pair<
      Int,
      String,
    >?

/**
 * This phone's registration with Condr Cloud, and the one `push` flag every `Hello` says (ADR
 * 0047): on while notifications are allowed and the last registration succeeded. Each launch, a new
 * FCM token and a Device paired or forgotten register again; losing permission or the last Device
 * removes a registration once made. A phone that never registered sends nothing. A request that
 * fails waits for the next of those; requests go out one at a time, in the order they were made.
 * Every member runs on the main thread.
 */
class Push(
  private val client: CompanionInterface,
  private val preferences: Preferences,
  private val cloud: String = cloudUrl(),
  private val send: Send = ::sendOverHttp,
) {
  private val scope = CoroutineScope(SupervisorJob() + Dispatchers.Main.immediate)

  /** What every `Hello` says, kept for the next launch's first connections. */
  var on: Boolean by mutableStateOf(preferences.pushOn)
    private set

  /** Whether this phone can be reached at all: Google Play services with Firebase behind them. */
  var available: Boolean by mutableStateOf(true)
    private set

  private var granted: Boolean? = null
  private var servers: List<String> = emptyList()
  private var token: String? = null
  /** Bumped by every registration, so only the latest one's answer sets the flag. */
  private var attempt = 0
  /** The last request, which the next one waits for. */
  private var last: Deferred<Boolean>? = null

  init {
    client.setPush(on)
  }

  /**
   * The notification permission and the Devices paired now, as the app last found them. A
   * registration that failed is tried again on the next call even when nothing changed.
   */
  suspend fun update(granted: Boolean, servers: List<String>) {
    if (granted == this.granted && servers == this.servers && on) return
    this.granted = granted
    this.servers = servers
    register()
  }

  /** FCM gave this launch's token, or a new one. */
  suspend fun received(token: String) {
    available = true
    if (token == this.token) return
    this.token = token
    register()
  }

  /** FCM gave no token: nothing can reach this phone. */
  fun failed() {
    available = false
    token = null
    attempt += 1
    set(false)
  }

  private suspend fun register() {
    attempt += 1
    val current = attempt
    val granted = granted ?: return
    if (granted && servers.isNotEmpty()) {
      // Without a token yet, its arrival registers.
      val token = token ?: return
      val body =
        Json.encodeToString(
          Registration.serializer(),
          Registration(platform = "FCM", token = token, servers = servers),
        )
      val registered = inOrder {
        // A `PUT` whose answer is lost may still have registered.
        preferences.pushRegistered = true
        succeeds("PUT", body)
      }
      if (current == attempt) set(registered)
    } else {
      set(false)
      inOrder {
        if (!preferences.pushRegistered) return@inOrder true
        val removed = succeeds("DELETE", null)
        if (removed) preferences.pushRegistered = false
        removed
      }
    }
  }

  /** Runs `request` once every request made before it has its answer, in order. */
  private suspend fun inOrder(request: suspend () -> Boolean): Boolean {
    val previous = last
    val next = scope.async {
      previous?.await()
      request()
    }
    last = next
    return next.await()
  }

  /** Signed as it goes out, since a token is good for five minutes. */
  private suspend fun succeeds(method: String, body: String?): Boolean {
    val headers =
      mutableMapOf("API-Version" to "1", "Authorization" to "Condr ${client.cloudToken()}")
    if (body != null) headers["Content-Type"] = "application/json"
    val (status, answer) = send(method, "$cloud/api/push/device", headers, body) ?: return false
    if (status !in 200..299) return false
    return runCatching {
        Json { ignoreUnknownKeys = true }.decodeFromString(Reply.serializer(), answer).code == "200"
      }
      .getOrDefault(false)
  }

  private fun set(on: Boolean) {
    this.on = on
    preferences.pushOn = on
    client.setPush(on)
  }

  /** Condr Cloud answers HTTP 200 with the outcome in `code`. */
  @Serializable data class Reply(val code: String)

  /** What `PUT /api/push/device` registers: the FCM token and the Devices that may push to it. */
  @Serializable
  data class Registration(val platform: String, val token: String, val servers: List<String>)

  companion object {
    /**
     * `PUT` or `DELETE /api/push/device` over the platform's client; a lost answer holds up the
     * queue.
     */
    suspend fun sendOverHttp(
      method: String,
      url: String,
      headers: Map<String, String>,
      body: String?,
    ): Pair<Int, String>? =
      withContext(Dispatchers.IO) {
        runCatching {
          val connection = URL(url).openConnection() as HttpURLConnection
          connection.requestMethod = method
          connection.connectTimeout = 15_000
          connection.readTimeout = 15_000
          headers.forEach { (name, value) -> connection.setRequestProperty(name, value) }
          if (body != null) {
            connection.doOutput = true
            connection.outputStream.use { it.write(body.encodeToByteArray()) }
          }
          val status = connection.responseCode
          val stream = if (status in 200..299) connection.inputStream else connection.errorStream
          status to (stream?.use { it.readBytes().decodeToString() } ?: "")
        }
          .getOrNull()
      }
  }
}
