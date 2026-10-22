package dev.condr.companion

import android.content.Context
import dev.condr.companion.demo.DemoClient
import dev.condr.companion.demo.DemoScene
import dev.condr.companion.demo.DemoWorld
import dev.condr.companion.model.Alerts
import dev.condr.companion.model.AppLock
import dev.condr.companion.model.AppTab
import dev.condr.companion.model.Banner
import dev.condr.companion.model.DemoAuthenticator
import dev.condr.companion.model.DeviceAuthenticator
import dev.condr.companion.model.DeviceRoute
import dev.condr.companion.model.PaneRef
import dev.condr.companion.model.Preferences
import dev.condr.companion.model.Router
import dev.condr.companion.model.Session
import dev.condr.companion.model.SessionListener
import dev.condr.companion.persistence.DeviceKey
import dev.condr.companion.persistence.SavedDeviceStore
import uniffi.condr_mobile.AgentStatus
import uniffi.condr_mobile.Companion
import uniffi.condr_mobile.CompanionInterface
import uniffi.condr_mobile.fingerprint

/** The app's objects, built once at launch: over the library, or over a demo scene. */
class Stores(
  val session: Session,
  val router: Router,
  val preferences: Preferences,
  val alerts: Alerts,
  val lock: AppLock,
  val client: CompanionInterface,
  /** A demo scene connects nowhere and registers for nothing. */
  val scene: DemoScene?,
) {
  /** `-e scrub "<device index>:<bar index>"` holds a scrub of the score for review. */
  var scrub: String? = null
  /** `-e focus true` opens a terminal with its field focused. */
  var focus: Boolean = false

  companion object {
    fun make(context: Context): Stores {
      val preferences = Preferences.load(context)
      val router = Router()
      val alerts = Alerts()
      val saved = SavedDeviceStore.load(context)
      val seed = DeviceKey.load(context)
      val listener = SessionListener()
      val client = Companion(seed, preferences.phoneName, saved.devices, listener)
      val session = Session(client, fingerprint(seed), saved, alerts, router, context)
      listener.bind { session.refresh() }
      val lock = AppLock(DeviceAuthenticator(context), session.paired, preferences)
      return Stores(session, router, preferences, alerts, lock, client, scene = null)
    }

    /** A scene for review or a UI test: the demo world, stores that forget between launches. */
    fun demo(context: Context, scene: DemoScene): Stores {
      val preferences = Preferences.ephemeral(context)
      val router = Router()
      // The demo world holds still, its banner and notices too, however slowly a test finds them.
      val alerts = Alerts(bannerLifetimeMillis = null, noticeLifetimeMillis = null)
      val saved = SavedDeviceStore.ephemeral(context)
      val client = DemoClient(scene)
      val session = Session(client, DemoWorld.FINGERPRINT, saved, alerts, router, context)
      val macmini1 = PaneRef(DemoWorld.MACMINI, 1u)
      when (scene) {
        DemoScene.DEVICES -> router.tab = AppTab.DEVICES
        DemoScene.DEVICE -> {
          router.tab = AppTab.DEVICES
          router.devicesPath.add(DeviceRoute.Device(DemoWorld.MACMINI))
        }
        DemoScene.OFFLINE -> {
          router.tab = AppTab.DEVICES
          router.devicesPath.add(DeviceRoute.Device(DemoWorld.STUDIO))
        }
        DemoScene.SETTINGS -> router.tab = AppTab.SETTINGS
        DemoScene.ERROR ->
          alerts.report("Couldn't reach macmini: the connection closed while opening the Pane.")
        DemoScene.TERMINAL -> router.agentsPath.add(DeviceRoute.Pane(macmini1))
        DemoScene.TERMINAL_ERROR -> {
          router.agentsPath.add(DeviceRoute.Pane(macmini1))
          alerts.report(
            "Couldn't send to macmini: the connection closed. Your reply is still in the field."
          )
        }
        DemoScene.QUESTION -> router.agentsPath.add(DeviceRoute.Pane(PaneRef(DemoWorld.JPDEV, 11u)))
        DemoScene.CODEX -> router.agentsPath.add(DeviceRoute.Pane(PaneRef(DemoWorld.MACMINI, 2u)))
        DemoScene.COMPUTER -> router.agentsPath.add(DeviceRoute.Device(DemoWorld.MACMINI))
        DemoScene.CROWD_DEVICE -> router.agentsPath.add(DeviceRoute.Device(DemoWorld.BUILDBOX))
        DemoScene.SHELL -> {
          router.tab = AppTab.DEVICES
          router.devicesPath.add(DeviceRoute.Device(DemoWorld.MACMINI))
          router.devicesPath.add(DeviceRoute.Pane(PaneRef(DemoWorld.MACMINI, 3u)))
        }
        DemoScene.BANNER -> {
          router.tab = AppTab.DEVICES
          alerts.raise(
            Banner(
              ref = PaneRef(DemoWorld.JPDEV, 12u),
              status = AgentStatus.BLOCKED,
              kind = "claude",
              title = "Claude needs you",
              detail = "Migrate auth to passkeys · jpdev",
            )
          )
        }
        else -> {}
      }
      val lock =
        AppLock(DemoAuthenticator, paired = scene == DemoScene.LOCK, preferences = preferences)
      return Stores(session, router, preferences, alerts, lock, client, scene)
    }
  }
}
