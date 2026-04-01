package sdk

import "unsafe"

// packI64 packs a pointer and length into a single i64 value.
// Upper 32 bits = pointer, lower 32 bits = length.
func packI64(ptr uint32, length uint32) int64 {
	return int64(ptr)<<32 | int64(length)
}

// unpackI64 extracts pointer and length from a packed i64.
func unpackI64(packed int64) (ptr uint32, length uint32) {
	ptr = uint32(packed >> 32)
	length = uint32(packed & 0xFFFFFFFF)
	return
}

// stringToPtr returns the pointer and length for a Go string's underlying bytes.
func stringToPtr(s string) (uint32, uint32) {
	if len(s) == 0 {
		return 0, 0
	}
	b := []byte(s)
	return uint32(uintptr(unsafe.Pointer(&b[0]))), uint32(len(b))
}

// ptrToString reads a string from a wasm pointer and length.
func ptrToString(ptr uint32, length uint32) string {
	if ptr == 0 || length == 0 {
		return ""
	}
	b := make([]byte, length)
	src := unsafe.Pointer(uintptr(ptr))
	for i := uint32(0); i < length; i++ {
		b[i] = *(*byte)(unsafe.Pointer(uintptr(src) + uintptr(i)))
	}
	return string(b)
}

// unpackString reads a string from a packed i64 (ptr<<32|len).
func unpackString(packed int64) string {
	if packed == 0 {
		return ""
	}
	ptr, length := unpackI64(packed)
	return ptrToString(ptr, length)
}

// PackResult serializes a string to wasm memory and returns a packed i64.
// It keeps a reference to prevent GC.
var resultKeepAlive []byte

func PackResult(s string) int64 {
	b := []byte(s)
	resultKeepAlive = b
	if len(b) == 0 {
		return 0
	}
	ptr := uint32(uintptr(unsafe.Pointer(&b[0])))
	return packI64(ptr, uint32(len(b)))
}

// Alloc allocates a block of memory of the given size and returns a pointer.
//
//export alloc
func Alloc(size uint32) uint32 {
	buf := make([]byte, size)
	if size == 0 {
		return 0
	}
	return uint32(uintptr(unsafe.Pointer(&buf[0])))
}

// Dealloc is a no-op in Go (GC handles memory), but required by the host ABI.
//
//export dealloc
func Dealloc(ptr uint32, size uint32) {
}

// GetABIVersion returns the ABI version supported by this SDK.
//
//export get_abi_version
func GetABIVersion() int32 {
	return ABIVersion
}
