package dev.condr.companion.persistence

import android.content.Context
import android.security.keystore.KeyGenParameterSpec
import android.security.keystore.KeyProperties
import java.io.File
import java.security.KeyStore
import javax.crypto.Cipher
import javax.crypto.KeyGenerator
import javax.crypto.SecretKey
import javax.crypto.spec.GCMParameterSpec
import uniffi.condr_mobile.generateSeed

/**
 * This phone's Device key seed (ADR 0040): an AES-GCM key the Keystore holds and never exports
 * encrypts it, and the ciphertext lives in the app's no-backup directory, so the seed never leaves
 * this phone. The library generates it once; the app hands it back at every start.
 */
object DeviceKey {
  private const val ALIAS = "device-key"
  private const val FILE = "device-key"
  private const val IV_BYTES = 12
  private const val TAG_BITS = 128

  /** The stored seed, or a fresh one stored now. */
  fun load(context: Context): ByteArray {
    val file = File(context.noBackupFilesDir, FILE)
    if (file.exists()) return decrypt(file.readBytes())
    val seed = generateSeed()
    val sealed = encrypt(seed)
    val staging = File(context.noBackupFilesDir, "$FILE.tmp")
    staging.writeBytes(sealed)
    if (!staging.renameTo(file)) error("Could not keep the device key")
    return seed
  }

  private fun key(): SecretKey {
    val store = KeyStore.getInstance("AndroidKeyStore").apply { load(null) }
    (store.getEntry(ALIAS, null) as? KeyStore.SecretKeyEntry)?.let {
      return it.secretKey
    }
    val generator = KeyGenerator.getInstance(KeyProperties.KEY_ALGORITHM_AES, "AndroidKeyStore")
    generator.init(
      KeyGenParameterSpec.Builder(
          ALIAS,
          KeyProperties.PURPOSE_ENCRYPT or KeyProperties.PURPOSE_DECRYPT,
        )
        .setBlockModes(KeyProperties.BLOCK_MODE_GCM)
        .setEncryptionPaddings(KeyProperties.ENCRYPTION_PADDING_NONE)
        .setKeySize(256)
        .build()
    )
    return generator.generateKey()
  }

  private fun encrypt(plain: ByteArray): ByteArray {
    val cipher = Cipher.getInstance("AES/GCM/NoPadding")
    cipher.init(Cipher.ENCRYPT_MODE, key())
    val iv = cipher.iv
    check(iv.size == IV_BYTES)
    return iv + cipher.doFinal(plain)
  }

  private fun decrypt(sealed: ByteArray): ByteArray {
    val cipher = Cipher.getInstance("AES/GCM/NoPadding")
    cipher.init(
      Cipher.DECRYPT_MODE,
      key(),
      GCMParameterSpec(TAG_BITS, sealed.copyOfRange(0, IV_BYTES)),
    )
    return cipher.doFinal(sealed.copyOfRange(IV_BYTES, sealed.size))
  }
}
