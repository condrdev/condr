package dev.condr.companion.model

import android.content.Context
import android.provider.Settings
import androidx.compose.runtime.mutableStateOf
import androidx.datastore.core.DataStore
import androidx.datastore.preferences.core.Preferences as Prefs
import androidx.datastore.preferences.core.booleanPreferencesKey
import androidx.datastore.preferences.core.edit
import androidx.datastore.preferences.core.floatPreferencesKey
import androidx.datastore.preferences.core.stringPreferencesKey
import androidx.datastore.preferences.preferencesDataStore
import kotlinx.coroutines.CoroutineScope
import kotlinx.coroutines.Dispatchers
import kotlinx.coroutines.SupervisorJob
import kotlinx.coroutines.flow.first
import kotlinx.coroutines.launch
import kotlinx.coroutines.runBlocking

private val Context.preferencesStore: DataStore<Prefs> by preferencesDataStore("preferences")

/**
 * What the person set and the app keeps: the terminal's text size, this phone's name and whether
 * the lock is on. Read once at launch, written through as they change.
 */
class Preferences private constructor(private val store: DataStore<Prefs>, initial: Prefs) {
  private val scope = CoroutineScope(SupervisorJob() + Dispatchers.IO)

  private val fontSizeState = mutableStateOf(initial[FONT_SIZE] ?: DEFAULT_FONT)
  private val phoneNameState = mutableStateOf(initial[PHONE_NAME] ?: "")
  private val namedState = mutableStateOf(initial[PHONE_NAME] != null)
  private val lockOnState = mutableStateOf(initial[LOCK_ON] ?: false)
  private val pushOnState = mutableStateOf(initial[PUSH_ON] ?: false)
  private var pushRegisteredValue = initial[PUSH_REGISTERED] ?: false
  private var pushAskedValue = initial[PUSH_ASKED] ?: false

  var fontSize: Float
    get() = fontSizeState.value
    set(value) {
      fontSizeState.value = value.coerceIn(SMALLEST_FONT, LARGEST_FONT)
      write { it[FONT_SIZE] = fontSizeState.value }
    }

  /** The name the phone pairs under; a Server lists it by it (ADR 0040). */
  var phoneName: String
    get() = phoneNameState.value
    set(value) {
      phoneNameState.value = value
      namedState.value = true
      write { it[PHONE_NAME] = value }
    }

  /** Whether the phone has a name of its own yet: only the first pairing asks for one. */
  val named: Boolean
    get() = namedState.value

  /** The lock is off until the owner turns it on (ADR 0048). */
  var lockOn: Boolean
    get() = lockOnState.value
    set(value) {
      lockOnState.value = value
      write { it[LOCK_ON] = value }
    }

  /** What every `Hello` says about pushes, as the last registration left it (ADR 0047). */
  var pushOn: Boolean
    get() = pushOnState.value
    set(value) {
      pushOnState.value = value
      write { it[PUSH_ON] = value }
    }

  /** Whether Condr Cloud may hold a registration to remove. */
  var pushRegistered: Boolean
    get() = pushRegisteredValue
    set(value) {
      pushRegisteredValue = value
      write { it[PUSH_REGISTERED] = value }
    }

  /**
   * Whether the system's notification prompt was shown: like iOS's, it is asked once, and Android
   * shows it twice at most.
   */
  var pushAsked: Boolean
    get() = pushAskedValue
    set(value) {
      pushAskedValue = value
      write { it[PUSH_ASKED] = value }
    }

  private fun write(edit: (androidx.datastore.preferences.core.MutablePreferences) -> Unit) {
    scope.launch { store.edit(edit) }
  }

  companion object {
    const val SMALLEST_FONT = 8f
    const val LARGEST_FONT = 20f
    const val DEFAULT_FONT = 12.5f
    private val FONT_SIZE = floatPreferencesKey("terminal.fontSize")
    private val PHONE_NAME = stringPreferencesKey("phone.name")
    private val LOCK_ON = booleanPreferencesKey("lock.on")
    private val PUSH_ON = booleanPreferencesKey("push.on")
    private val PUSH_REGISTERED = booleanPreferencesKey("push.registered")
    private val PUSH_ASKED = booleanPreferencesKey("push.asked")

    /** The app's preferences, read now: a few bytes, before anything is drawn. */
    fun load(context: Context): Preferences {
      val store = context.preferencesStore
      return Preferences(store, runBlocking { store.data.first() })
    }

    /** Preferences that forget between launches: a demo scene's. */
    fun ephemeral(context: Context): Preferences {
      // Its own file each time: DataStore refuses two instances over one file, and a demo
      // scene may follow another in the same process.
      val file = context.cacheDir.resolve("demo-${java.util.UUID.randomUUID()}.preferences_pb")
      val store = androidx.datastore.preferences.core.PreferenceDataStoreFactory.create { file }
      return Preferences(store, androidx.datastore.preferences.core.emptyPreferences())
    }

    /** Preferences over a store that never touches a file, for tests. */
    fun inMemory(): Preferences {
      val store =
        object : DataStore<Prefs> {
          private val state =
            kotlinx.coroutines.flow.MutableStateFlow(
              androidx.datastore.preferences.core.emptyPreferences()
            )
          override val data: kotlinx.coroutines.flow.Flow<Prefs> = state

          override suspend fun updateData(transform: suspend (t: Prefs) -> Prefs): Prefs {
            val next = transform(state.value)
            state.value = next
            return next
          }
        }
      return Preferences(store, androidx.datastore.preferences.core.emptyPreferences())
    }

    /** The name the person gave this phone in its settings, to offer at pairing. */
    fun systemDeviceName(context: Context): String =
      Settings.Global.getString(context.contentResolver, Settings.Global.DEVICE_NAME)?.takeIf {
        it.isNotBlank()
      } ?: "Android"
  }
}
