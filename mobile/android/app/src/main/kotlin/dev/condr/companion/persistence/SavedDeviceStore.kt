package dev.condr.companion.persistence

import android.content.Context
import androidx.datastore.core.CorruptionException
import androidx.datastore.core.DataStore
import androidx.datastore.core.DataStoreFactory
import androidx.datastore.core.Serializer
import androidx.datastore.dataStoreFile
import java.io.InputStream
import java.io.OutputStream
import kotlinx.coroutines.CoroutineScope
import kotlinx.coroutines.Dispatchers
import kotlinx.coroutines.SupervisorJob
import kotlinx.coroutines.flow.first
import kotlinx.coroutines.launch
import kotlinx.coroutines.runBlocking
import kotlinx.serialization.Serializable
import kotlinx.serialization.SerializationException
import kotlinx.serialization.json.Json
import uniffi.condr_mobile.SavedDevice

@Serializable private data class SavedDeviceRecord(val address: String, val label: String)

@Serializable private data class SavedDevices(val devices: List<SavedDeviceRecord> = emptyList())

private object SavedDevicesSerializer : Serializer<SavedDevices> {
  override val defaultValue = SavedDevices()

  override suspend fun readFrom(input: InputStream): SavedDevices =
    try {
      Json.decodeFromString(SavedDevices.serializer(), input.readBytes().decodeToString())
    } catch (e: SerializationException) {
      throw CorruptionException("The paired devices could not be read", e)
    }

  override suspend fun writeTo(t: SavedDevices, output: OutputStream) {
    output.write(Json.encodeToString(SavedDevices.serializer(), t).encodeToByteArray())
  }
}

/**
 * The paired Devices as the app keeps them, beside the key (ADR 0040): each an address without its
 * invite and the name the Server goes by. Read once at launch; every change is written through as
 * one atomic file.
 */
class SavedDeviceStore
private constructor(private val store: DataStore<SavedDevices>, initial: SavedDevices) {
  private val scope = CoroutineScope(SupervisorJob() + Dispatchers.IO)

  var devices: List<SavedDevice> = initial.devices.map { SavedDevice(it.address, it.label) }
    private set

  fun add(device: SavedDevice) {
    devices = devices.filter { it.address != device.address } + device
    save()
  }

  fun remove(address: String) {
    devices = devices.filter { it.address != address }
    save()
  }

  fun rename(address: String, label: String) {
    devices = devices.map { if (it.address == address) SavedDevice(address, label) else it }
    save()
  }

  private fun save() {
    val snapshot = SavedDevices(devices.map { SavedDeviceRecord(it.address, it.label) })
    scope.launch { store.updateData { snapshot } }
  }

  companion object {
    fun load(context: Context): SavedDeviceStore {
      val store =
        DataStoreFactory.create(serializer = SavedDevicesSerializer) {
          context.dataStoreFile("devices.json")
        }
      return SavedDeviceStore(store, runBlocking { store.data.first() })
    }

    /** A store that forgets: a demo scene's. */
    fun ephemeral(context: Context): SavedDeviceStore {
      val store =
        DataStoreFactory.create(serializer = SavedDevicesSerializer) {
          context.cacheDir.resolve("demo-devices.json").also { it.delete() }
        }
      return SavedDeviceStore(store, SavedDevices())
    }
  }
}
