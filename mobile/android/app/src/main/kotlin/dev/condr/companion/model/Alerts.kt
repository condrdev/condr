package dev.condr.companion.model

import androidx.compose.runtime.getValue
import androidx.compose.runtime.mutableStateOf
import androidx.compose.runtime.setValue
import java.util.UUID
import kotlinx.coroutines.CoroutineScope
import kotlinx.coroutines.Dispatchers
import kotlinx.coroutines.SupervisorJob
import kotlinx.coroutines.delay
import kotlinx.coroutines.launch
import uniffi.condr_mobile.AgentStatus

/** An Agent elsewhere started to need you or finished, while you look at something else. */
data class Banner(
  val ref: PaneRef,
  val status: AgentStatus,
  /** The Agent's kind id, for its mark. */
  val kind: String?,
  val title: String,
  val detail: String,
  /** Raised while the first screen is showing, which shows the change itself: felt, not shown. */
  val quiet: Boolean = false,
  val id: UUID = UUID.randomUUID(),
)

/**
 * What the app says over its screens: one banner, one error to read, copy and dismiss (ADR 0020),
 * and a passing notice such as "Copied to clipboard". Lifetimes of null keep them, as a demo
 * scene's do.
 */
class Alerts(
  private val bannerLifetimeMillis: Long? = BANNER_LIFETIME_MILLIS,
  private val noticeLifetimeMillis: Long? = NOTICE_LIFETIME_MILLIS,
) {
  private val scope = CoroutineScope(SupervisorJob() + Dispatchers.Main.immediate)

  var banner: Banner? by mutableStateOf(null)
    private set

  var error: String? by mutableStateOf(null)
    private set

  var notice: String? by mutableStateOf(null)
    private set

  fun raise(banner: Banner) {
    this.banner = banner
    val lifetime = bannerLifetimeMillis ?: return
    scope.launch {
      delay(lifetime)
      if (this@Alerts.banner?.id == banner.id) this@Alerts.banner = null
    }
  }

  fun dismissBanner() {
    banner = null
  }

  /** Opening the Pane a banner points at answers it. */
  fun dismissBanner(ref: PaneRef) {
    if (banner?.ref == ref) banner = null
  }

  fun report(message: String) {
    error = message
  }

  fun dismissError() {
    error = null
  }

  fun notify(text: String) {
    notice = text
    val lifetime = noticeLifetimeMillis ?: return
    scope.launch {
      delay(lifetime)
      if (notice == text) notice = null
    }
  }

  companion object {
    const val BANNER_LIFETIME_MILLIS = 6_000L
    const val NOTICE_LIFETIME_MILLIS = 2_000L
  }
}
