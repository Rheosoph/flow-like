@file:Suppress("NOTHING_TO_INLINE")
@file:OptIn(UnsafeWasmMemoryApi::class)

package sdk

import kotlin.wasm.unsafe.*

// Persistent bump allocator state (survives across function calls)
private var bumpPtr: Int = 8   // Start at 8 to reserve address 0 as null sentinel
private var committedBytes: Int = 0  // How many bytes of linear memory we know exist

private fun ensureCapacity(needed: Int) {
    if (needed <= committedBytes) return
    // Use withScopedMemoryAllocator to trigger memory.grow as a side effect.
    // The scoped allocator always starts from its reset base (address 0),
    // so we request 'needed' bytes — this ensures at least 'needed' bytes
    // of linear memory exist, even though we track our own pointer.
    withScopedMemoryAllocator { allocator ->
        allocator.allocate(needed)
    }
    committedBytes = needed
}

@WasmExport
fun alloc(size: Int): Int {
    if (size <= 0) return 0
    val aligned = (size + 7) and 7.inv()
    val ptr = bumpPtr
    bumpPtr += aligned
    ensureCapacity(bumpPtr)
    return ptr
}

@WasmExport
fun dealloc(ptr: Int, size: Int) {
    // Bump allocator: memory is reclaimed when the WASM instance is destroyed.
}

fun packI64(ptr: Int, len: Int): Long =
    (ptr.toLong() shl 32) or (len.toLong() and 0xFFFFFFFFL)

fun packResult(value: String): Long {
    val bytes = value.encodeToByteArray()
    if (bytes.isEmpty()) return 0L
    val ptr = alloc(bytes.size)
    if (ptr == 0) return 0L
    writeBytes(ptr, bytes)
    return packI64(ptr, bytes.size)
}

fun stringToPtr(value: String): Pair<Int, Int> {
    val bytes = value.encodeToByteArray()
    val ptr = alloc(bytes.size)
    writeBytes(ptr, bytes)
    return ptr to bytes.size
}

fun ptrToString(ptr: Int, len: Int): String {
    if (ptr == 0 || len == 0) return ""
    val bytes = readBytes(ptr, len)
    return bytes.decodeToString()
}

fun unpackString(packed: Long): String? {
    if (packed == 0L) return null
    val ptr = (packed ushr 32).toInt()
    val len = (packed and 0xFFFFFFFFL).toInt()
    if (ptr == 0 || len == 0) return null
    return ptrToString(ptr, len)
}

private fun writeBytes(ptr: Int, bytes: ByteArray) {
    val base = Pointer(ptr.toUInt())
    for (i in bytes.indices) {
        (base + i).storeByte(bytes[i])
    }
}

private fun readBytes(ptr: Int, len: Int): ByteArray {
    val base = Pointer(ptr.toUInt())
    val bytes = ByteArray(len)
    for (i in 0 until len) {
        bytes[i] = (base + i).loadByte()
    }
    return bytes
}
