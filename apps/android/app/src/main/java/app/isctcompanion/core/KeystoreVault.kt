package app.isctcompanion.core

import android.content.Context
import android.security.keystore.KeyGenParameterSpec
import android.security.keystore.KeyProperties
import android.util.Base64
import java.security.KeyStore
import javax.crypto.Cipher
import javax.crypto.KeyGenerator
import javax.crypto.SecretKey
import javax.crypto.spec.GCMParameterSpec

/** `NativeVault` over the Android Keystore: each secret is sealed with a
 * non-exportable AES-GCM key and the ciphertext kept in app-private
 * preferences, so it never leaves this device. Written by hand because
 * androidx.security-crypto, the library that did this, is deprecated. Called on
 * Core worker threads, never the main thread. */
class KeystoreVault(context: Context) : NativeVault {
    private val preferences = context.getSharedPreferences("vault", Context.MODE_PRIVATE)

    override fun load(account: String): String? {
        val sealed = preferences.getString(account, null) ?: return null
        return try {
            val bytes = Base64.decode(sealed, Base64.NO_WRAP)
            val cipher = Cipher.getInstance(TRANSFORMATION)
            cipher.init(Cipher.DECRYPT_MODE, key(), GCMParameterSpec(128, bytes, 0, IV_BYTES))
            String(cipher.doFinal(bytes, IV_BYTES, bytes.size - IV_BYTES), Charsets.UTF_8)
        } catch (error: Exception) {
            throw SdkException.Unavailable("Keystore read failed: ${error.javaClass.simpleName}")
        }
    }

    override fun store(account: String, secret: String) {
        val sealed = try {
            val cipher = Cipher.getInstance(TRANSFORMATION)
            cipher.init(Cipher.ENCRYPT_MODE, key())
            Base64.encodeToString(cipher.iv + cipher.doFinal(secret.toByteArray(Charsets.UTF_8)), Base64.NO_WRAP)
        } catch (error: Exception) {
            throw SdkException.Unavailable("Keystore write failed: ${error.javaClass.simpleName}")
        }
        if (!preferences.edit().putString(account, sealed).commit()) {
            throw SdkException.Unavailable("Keystore write failed")
        }
    }

    override fun delete(account: String) {
        preferences.edit().remove(account).commit()
    }

    @Synchronized
    private fun key(): SecretKey {
        val store = KeyStore.getInstance(KEYSTORE).apply { load(null) }
        (store.getEntry(ALIAS, null) as? KeyStore.SecretKeyEntry)?.let { return it.secretKey }
        val generator = KeyGenerator.getInstance(KeyProperties.KEY_ALGORITHM_AES, KEYSTORE)
        generator.init(
            KeyGenParameterSpec.Builder(ALIAS, KeyProperties.PURPOSE_ENCRYPT or KeyProperties.PURPOSE_DECRYPT)
                .setBlockModes(KeyProperties.BLOCK_MODE_GCM)
                .setEncryptionPaddings(KeyProperties.ENCRYPTION_PADDING_NONE)
                .setKeySize(256)
                .build(),
        )
        return generator.generateKey()
    }

    private companion object {
        const val KEYSTORE = "AndroidKeyStore"
        const val ALIAS = "app.isctcompanion.vault"
        const val TRANSFORMATION = "AES/GCM/NoPadding"
        const val IV_BYTES = 12
    }
}
