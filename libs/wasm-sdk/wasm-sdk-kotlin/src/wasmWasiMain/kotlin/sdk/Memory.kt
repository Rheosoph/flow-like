@file:Suppress("NOTHING_TO_INLINE")
@file:OptIn(UnsafeWasmMemoryApi::class, kotlin.wasm.ExperimentalWasmInterop::class)

package sdk

import kotlin.wasm.unsafe.*

// ABI buffers live until the host resets scratch memory before the next invocation.
private const val SCRATCH_START: Int = 8 // Reserve address 0 as the null sentinel.
private var bumpPtr: Int = SCRATCH_START
private var committedBytes: Int = 0  // How many bytes of linear memory we know exist

private fun ensureCapacity(needed: Int) {
    if (needed <= committedBytes) return
    // Use withScopedMemoryAllocator to trigger memory.grow as a side effect.
    // The scoped allocator always starts from its reset base (address 0),
    // so we request 'needed' bytes to ensure at least 'needed' bytes
    // of linear memory exist, even though we track our own pointer.
    withScopedMemoryAllocator { allocator ->
        allocator.allocate(needed)
    }
    committedBytes = needed
}

@WasmExport
fun alloc(size: Int): Int {
    if (size <= 0 || size > Int.MAX_VALUE - 7) return 0
    val aligned = (size + 7) and 7.inv()
    val ptr = bumpPtr
    if (aligned > Int.MAX_VALUE - ptr) return 0
    val end = ptr + aligned
    ensureCapacity(end)
    bumpPtr = end
    return ptr
}

@WasmExport
fun dealloc(ptr: Int, size: Int) {
    // Individual buffers are reclaimed together by reset_scratch.
}

@WasmExport
fun reset_scratch() {
    // Reuse committed linear memory without clearing Kotlin objects or package globals.
    bumpPtr = SCRATCH_START
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
