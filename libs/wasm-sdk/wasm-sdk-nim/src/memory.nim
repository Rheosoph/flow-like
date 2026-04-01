import types

var resultBuffer: string

proc packI64*(p: uint32; l: uint32): int64 =
  (int64(p) shl 32) or int64(l)

proc unpackI64*(packed: int64): tuple[p: uint32, l: uint32] =
  result.p = uint32(packed shr 32)
  result.l = uint32(packed and 0xFFFFFFFF'i64)

proc unpackString*(packed: int64): string =
  if packed == 0: return ""
  let (p, l) = unpackI64(packed)
  if p == 0 or l == 0: return ""
  result = newString(l)
  copyMem(addr result[0], cast[pointer](p), l)

proc packResult*(json: string): int64 =
  resultBuffer = json
  let p = cast[uint32](addr resultBuffer[0])
  let l = uint32(resultBuffer.len)
  packI64(p, l)

proc c_malloc(size: csize_t): pointer {.importc: "malloc", header: "<stdlib.h>".}
proc c_free(p: pointer) {.importc: "free", header: "<stdlib.h>".}

proc wasmAlloc*(size: uint32): uint32 {.exportc: "alloc".} =
  let p = c_malloc(csize_t(size))
  cast[uint32](p)

proc wasmDealloc*(p: uint32; size: uint32) {.exportc: "dealloc".} =
  c_free(cast[pointer](p))

proc getAbiVersionExport*(): uint32 {.exportc: "get_abi_version".} =
  ABI_VERSION
