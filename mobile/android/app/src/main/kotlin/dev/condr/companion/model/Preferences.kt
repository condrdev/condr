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
  private val lockOnState = mutableStateOf(initial[LOCK_ON] ?: true)

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

  /** The lock is on until the owner turns it off (ADR 0041). */
  var lockOn: Boolean
    get() = lockOnState.value
    set(value) {
      lockOnState.value = value
      write { it[LOCK_ON] = value }
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

    /** The app's preferences, read now: a few bytes, before anything is drawn. */
    fun load(context: Context): Preferences {
      val store = context.preferencesStore
      return Preferences(store, runBlocking { store.data.first() })
    }

    /** The name the person gave this phone in its settings, to offer at pairing. */
    fun systemDeviceName(context: Context): String =
      Settings.Global.getString(context.contentResolver, Settings.Global.DEVICE_NAME)?.takeIf {
        it.isNotBlank()
      } ?: "Android"
  }
}
