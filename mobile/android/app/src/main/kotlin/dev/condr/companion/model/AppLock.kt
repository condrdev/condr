package dev.condr.companion.model

import android.content.Context
import android.hardware.biometrics.BiometricManager
import android.hardware.biometrics.BiometricPrompt
import android.os.Build
import android.os.CancellationSignal
import androidx.compose.runtime.getValue
import androidx.compose.runtime.mutableStateOf
import androidx.compose.runtime.setValue
import kotlin.coroutines.resume
import kotlinx.coroutines.suspendCancellableCoroutine

/** Who confirms it is the owner: a fingerprint, a face or the screen lock. */
interface Authenticator {
  /** Whether the phone can ask at all; one without a screen lock cannot. */
  val available: Boolean
  /** What it asks for, for the setting's name. */
  val method: String
  /** Whether the lock asks as soon as it shows, or waits for the button. */
  val promptsOnAppear: Boolean

  suspend fun authenticate(reason: String): Boolean
}

/** Android's own prompt (ADR 0048): a weak biometric or the device credential. */
class DeviceAuthenticator(private val context: Context) : Authenticator {
  private val manager: BiometricManager?
    get() = context.getSystemService(BiometricManager::class.java)

  private val authenticators =
    BiometricManager.Authenticators.BIOMETRIC_WEAK or
      BiometricManager.Authenticators.DEVICE_CREDENTIAL

  override val available: Boolean
    get() =
      if (Build.VERSION.SDK_INT >= Build.VERSION_CODES.R) {
        manager?.canAuthenticate(authenticators) == BiometricManager.BIOMETRIC_SUCCESS
      } else {
        context.getSystemService(android.app.KeyguardManager::class.java)?.isDeviceSecure == true
      }

  override val method: String
    get() {
      val biometric =
        if (Build.VERSION.SDK_INT >= Build.VERSION_CODES.R)
          manager?.canAuthenticate(BiometricManager.Authenticators.BIOMETRIC_WEAK) ==
            BiometricManager.BIOMETRIC_SUCCESS
        else manager?.canAuthenticate() == BiometricManager.BIOMETRIC_SUCCESS
      return if (biometric) "biometrics" else "screen lock"
    }

  override val promptsOnAppear: Boolean = true

  override suspend fun authenticate(reason: String): Boolean =
    suspendCancellableCoroutine { continuation ->
      val builder =
        BiometricPrompt.Builder(context)
          .setTitle("Condr")
          .setDescription(reason)
          .setConfirmationRequired(false)
      if (Build.VERSION.SDK_INT >= Build.VERSION_CODES.R)
        builder.setAllowedAuthenticators(authenticators)
      else @Suppress("DEPRECATION") builder.setDeviceCredentialAllowed(true)
      val signal = CancellationSignal()
      continuation.invokeOnCancellation { signal.cancel() }
      builder
        .build()
        .authenticate(
          signal,
          context.mainExecutor,
          object : BiometricPrompt.AuthenticationCallback() {
            override fun onAuthenticationSucceeded(result: BiometricPrompt.AuthenticationResult) {
              if (continuation.isActive) continuation.resume(true)
            }

            override fun onAuthenticationError(errorCode: Int, errString: CharSequence) {
              if (continuation.isActive) continuation.resume(false)
            }
          },
        )
    }
}

/** A demo scene's lock opens on request, without the phone's owner. */
object DemoAuthenticator : Authenticator {
  override val available = true
  override val method = "biometrics"
  override val promptsOnAppear = false

  override suspend fun authenticate(reason: String) = true
}

/**
 * The app lock (ADR 0041): the owner's finger, face or screen lock when the app starts and when it
 * comes back after more than five minutes away. It is on unless the owner turned it off, which asks
 * them first; before anything is paired there is nothing to protect.
 */
class AppLock(
  private val authenticator: Authenticator,
  paired: Boolean,
  private val preferences: Preferences,
  private val now: () -> Long = System::currentTimeMillis,
) {
  var locked by mutableStateOf(paired && preferences.lockOn && authenticator.available)
    private set

  var authenticating by mutableStateOf(false)
    private set

  private var leftAt: Long? = null

  /** Whether the lock is on; it is until the owner turns it off. */
  val enabled: Boolean
    get() = preferences.lockOn

  val available: Boolean
    get() = authenticator.available

  val method: String
    get() = authenticator.method

  val promptsOnAppear: Boolean
    get() = authenticator.promptsOnAppear

  fun lock() {
    if (!enabled || !authenticator.available) return
    locked = true
  }

  /** The app left the screen; the clock on the grace period starts. */
  fun sceneLeft() {
    if (leftAt == null) leftAt = now()
  }

  /** The app is back: locked if it was away past the grace period. */
  fun sceneBecameActive(paired: Boolean) {
    val left = leftAt
    if (left != null && paired && now() - left > GRACE_MILLIS) lock()
    leftAt = null
  }

  suspend fun unlock() {
    if (!locked || authenticating) return
    if (ask("Unlock to reach your computers.")) locked = false
  }

  /**
   * Turns the lock on, or off once the owner confirms it is them: whoever holds the phone unlocked
   * must not be able to leave it open for the next person.
   */
  suspend fun setEnabled(on: Boolean) {
    if (on == enabled) return
    if (!on && !ask("Confirm it is you to turn off the app lock.")) return
    preferences.lockOn = on
  }

  private suspend fun ask(reason: String): Boolean {
    if (authenticating) return false
    authenticating = true
    try {
      return authenticator.authenticate(reason)
    } finally {
      authenticating = false
    }
  }

  companion object {
    const val GRACE_MILLIS = 300_000L
  }
}
