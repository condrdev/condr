package dev.condr.companion

import android.app.Application
import android.net.ConnectivityManager
import android.net.Network
import androidx.lifecycle.DefaultLifecycleObserver
import androidx.lifecycle.LifecycleOwner
import androidx.lifecycle.ProcessLifecycleOwner
import dev.condr.companion.demo.DemoScene
import kotlinx.coroutines.CoroutineScope
import kotlinx.coroutines.Dispatchers
import kotlinx.coroutines.SupervisorJob
import kotlinx.coroutines.launch

/**
 * Holds the app's objects for the process (ADR 0048) and drives the library's foreground life (ADR
 * 0040): connected while any Activity is started, closed when none is.
 */
class CompanionApplication : Application() {
  private var stores: Stores? = null
  private val scope = CoroutineScope(SupervisorJob() + Dispatchers.Main.immediate)

  /** The stores for the scene the Activity was launched into, or the real ones. */
  fun stores(scene: DemoScene?): Stores {
    val current = stores
    if (current != null && current.scene == scene) return current
    current?.client?.shutdown()
    val built = if (scene == null) Stores.make(this) else Stores.demo(this, scene)
    stores = built
    return built
  }

  /** The stores built so far, for the messaging service. */
  fun current(): Stores? = stores

  override fun onCreate() {
    super.onCreate()
    dev.condr.companion.push.Notifications.createChannel(this)
    ProcessLifecycleOwner.get()
      .lifecycle
      .addObserver(
        object : DefaultLifecycleObserver {
          override fun onStart(owner: LifecycleOwner) {
            stores?.takeIf { it.scene == null }?.session?.resume()
          }

          override fun onStop(owner: LifecycleOwner) {
            val session = stores?.takeIf { it.scene == null }?.session ?: return
            scope.launch { session.suspend() }
          }
        }
      )
    getSystemService(ConnectivityManager::class.java)
      .registerDefaultNetworkCallback(
        object : ConnectivityManager.NetworkCallback() {
          override fun onAvailable(network: Network) = changed()

          override fun onLost(network: Network) = changed()

          private fun changed() {
            scope.launch { stores?.takeIf { it.scene == null }?.session?.networkChanged() }
          }
        }
      )
  }
}
