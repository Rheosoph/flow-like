// Memory.swift — WASM memory management and ABI exports.

// MARK: - Pack / Unpack

/// Packs a pointer and length into a single i64: (ptr << 32) | len.
public func packI64(ptr: UInt32, len: UInt32) -> Int64 {
    (Int64(ptr) << 32) | Int64(len)
}

/// Unpacks a packed i64 into (ptr, len).
public func unpackI64(_ packed: Int64) -> (ptr: UInt32, len: UInt32) {
    let ptr = UInt32(truncatingIfNeeded: packed >> 32)
    let len = UInt32(truncatingIfNeeded: packed & 0xFFFF_FFFF)
    return (ptr, len)
}

// MARK: - String ↔ Pointer

/// Copies a Swift string into WASM linear memory and returns (ptr, len).
/// The caller is responsible for the allocated memory.
public func stringToPtr(_ s: String) -> (UInt32, UInt32) {
    let utf8 = Array(s.utf8)
    if utf8.isEmpty { return (0, 0) }
    let size = UInt32(utf8.count)
    let ptr = wasmAlloc(size)
    let dest = UnsafeMutablePointer<UInt8>(bitPattern: UInt(ptr))!
    for i in 0..<utf8.count {
        dest[i] = utf8[i]
    }
    return (ptr, size)
}

/// Reads a string from WASM linear memory at (ptr, len).
public func ptrToString(ptr: UInt32, len: UInt32) -> String {
    if ptr == 0 || len == 0 { return "" }
    let src = UnsafePointer<UInt8>(bitPattern: UInt(ptr))!
    let buf = UnsafeBufferPointer(start: src, count: Int(len))
    return String(decoding: buf, as: UTF8.self)
}

/// Reads a string from a packed i64.
public func unpackString(_ packed: Int64) -> String {
    if packed == 0 { return "" }
    let (ptr, len) = unpackI64(packed)
    return ptrToString(ptr: ptr, len: len)
}

// MARK: - Result packing

/// Serializes a string into WASM memory and returns a packed i64.
public func packResult(_ s: String) -> Int64 {
    let (ptr, len) = stringToPtr(s)
    return packI64(ptr: ptr, len: len)
}

/// Serializes a NodeDefinition to JSON and returns a packed i64.
public func serializeDefinition(_ def: NodeDefinition) -> Int64 {
    packResult(def.toJSON())
}

/// Serializes an ExecutionResult to JSON and returns a packed i64.
public func serializeResult(_ result: ExecutionResult) -> Int64 {
    packResult(result.toJSON())
}

// MARK: - ParseInput

/// Deserializes an ExecutionInput from WASM memory at (ptr, len).
public func parseInput(ptr: UInt32, length: UInt32) -> ExecutionInput {
    let json = ptrToString(ptr: ptr, len: length)
    return parseExecutionInput(json)
}

// MARK: - WASM ABI Exports

/// Allocates a block of memory in WASM linear memory.
@_cdecl("alloc")
public func wasmAlloc(_ size: UInt32) -> UInt32 {
    if size == 0 { return 0 }
    let ptr = UnsafeMutablePointer<UInt8>.allocate(capacity: Int(size))
    return UInt32(UInt(bitPattern: ptr))
}

/// Deallocates a block of memory in WASM linear memory.
@_cdecl("dealloc")
public func wasmDealloc(_ ptr: UInt32, _ size: UInt32) {
    guard ptr != 0, size != 0 else { return }
    let p = UnsafeMutablePointer<UInt8>(bitPattern: UInt(ptr))
    p?.deallocate()
}

/// Returns the ABI version supported by this SDK.
@_cdecl("get_abi_version")
public func getABIVersion() -> UInt32 {
    ABI_VERSION
}
