package com.flow_like.app

import android.content.Context
import android.security.keystore.KeyGenParameterSpec
import android.security.keystore.KeyProperties
import android.util.AtomicFile
import java.io.File
import java.security.KeyStore
import java.security.MessageDigest
import javax.crypto.Cipher
import javax.crypto.KeyGenerator
import javax.crypto.SecretKey
import javax.crypto.spec.GCMParameterSpec

/** Ciphertext stays out of backup; the wrapping key never leaves Android Keystore. */
internal interface BotRecordStore {
    fun read(key: String): ByteArray?
    fun write(key: String, bytes: ByteArray)
    fun delete(key: String)
}

internal class BotSecureStore(context: Context) : BotRecordStore {
    private val directory = File(context.noBackupFilesDir, "bot-secure-v1").apply { mkdirs() }

    companion object {
        private const val ALIAS = "com.flow-like.bot-secure.v1"
        private const val MAX_BYTES = 4 * 1024 * 1024
        private val lock = Any()
    }

    private fun file(key: String): AtomicFile {
        require(key.isNotEmpty() && key.length <= 2048) { "Invalid secure storage key" }
        val digest = MessageDigest.getInstance("SHA-256").digest(key.toByteArray())
        return AtomicFile(File(directory, digest.joinToString("") { "%02x".format(it) }))
    }

    private fun encryptionKey(create: Boolean): SecretKey {
        val store = KeyStore.getInstance("AndroidKeyStore").apply { load(null) }
        (store.getKey(ALIAS, null) as? SecretKey)?.let { return it }
        // A missing key must never silently replace the key for existing ciphertext.
        check(create && directory.listFiles().orEmpty().none { it.isFile }) {
            "Android Keystore key is unavailable; encrypted data was not changed"
        }
        return KeyGenerator.getInstance(KeyProperties.KEY_ALGORITHM_AES, "AndroidKeyStore").run {
            init(KeyGenParameterSpec.Builder(ALIAS, KeyProperties.PURPOSE_ENCRYPT or KeyProperties.PURPOSE_DECRYPT)
                .setBlockModes(KeyProperties.BLOCK_MODE_GCM)
                .setEncryptionPaddings(KeyProperties.ENCRYPTION_PADDING_NONE)
                .setKeySize(256)
                .setRandomizedEncryptionRequired(true)
                .build())
            generateKey()
        }
    }

    override fun read(key: String): ByteArray? = synchronized(lock) {
        val file = file(key)
        if (!file.baseFile.exists() && !File(file.baseFile.path + ".bak").exists()) return@synchronized null
        require(file.baseFile.length() <= MAX_BYTES + 28) { "Encrypted record is too large" }
        val bytes = file.readFully()
        require(bytes.size in 28..MAX_BYTES + 28) { "Invalid encrypted record" }
        Cipher.getInstance("AES/GCM/NoPadding").run {
            init(Cipher.DECRYPT_MODE, encryptionKey(false), GCMParameterSpec(128, bytes.copyOfRange(0, 12)))
            updateAAD(key.toByteArray(Charsets.UTF_8))
            doFinal(bytes, 12, bytes.size - 12)
        }
    }

    override fun write(key: String, bytes: ByteArray) = synchronized(lock) {
        require(bytes.size <= MAX_BYTES) { "Secure record is too large" }
        val cipher = Cipher.getInstance("AES/GCM/NoPadding")
        cipher.init(Cipher.ENCRYPT_MODE, encryptionKey(true))
        cipher.updateAAD(key.toByteArray(Charsets.UTF_8))
        val encrypted = cipher.iv + cipher.doFinal(bytes)
        val file = file(key)
        val stream = file.startWrite()
        try {
            stream.write(encrypted)
            file.finishWrite(stream)
        } catch (error: Exception) {
            file.failWrite(stream)
            throw error
        }
    }

    override fun delete(key: String) = synchronized(lock) { file(key).delete() }
}
