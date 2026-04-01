///| Inline WASM helpers for linear memory access
extern "wasm" fn wasm_memory_grow(pages : Int) -> Int =
  #|(func (param i32) (result i32) (memory.grow (local.get 0)))

extern "wasm" fn wasm_store8(addr : Int, val : Int) =
  #|(func (param i32 i32) (i32.store8 (local.get 0) (local.get 1)))

extern "wasm" fn wasm_load8(addr : Int) -> Int =
  #|(func (param i32) (result i32) (i32.load8_u (local.get 0)))

// ---------------------------------------------------------------------------
// Bump allocator — grows memory independently of MoonBit's own heap
// ---------------------------------------------------------------------------

let arena_next : Ref[Int] = Ref::new(0)
let arena_end : Ref[Int] = Ref::new(0)

///| Allocate `size` bytes of linear memory (exported to the host).
pub fn alloc(size : Int) -> Int {
  let aligned = (size + 7) / 8 * 8
  if arena_next.val + aligned > arena_end.val {
    let needed = if aligned > 65536 { aligned } else { 65536 }
    let pages = (needed + 65535) / 65536
    let prev = wasm_memory_grow(pages)
    if prev < 0 {
      return 0
    }
    let base = prev * 65536
    if arena_next.val == 0 {
      arena_next.val = base
    }
    arena_end.val = base + pages * 65536
  }
  let ptr = arena_next.val
  arena_next.val = arena_next.val + aligned
  ptr
}

///| Free memory (no-op for bump allocator).
pub fn dealloc(_ptr : Int, _size : Int) -> Unit {
  // no-op
}

// ---------------------------------------------------------------------------
// Pack / unpack i64 (pointer in high 32 bits, length in low 32 bits)
// ---------------------------------------------------------------------------

///| Pack a (ptr, len) pair into a single Int64.
pub fn pack_result(ptr : Int, len : Int) -> Int64 {
  (ptr.to_uint64().reinterpret_as_int64() << 32) | len.to_uint64().reinterpret_as_int64()
}

///| Extract the pointer from a packed i64.
pub fn unpack_ptr(packed : Int64) -> Int {
  (packed.reinterpret_as_uint64() >> 32).to_int()
}

///| Extract the length from a packed i64.
pub fn unpack_len(packed : Int64) -> Int {
  packed.to_int()
}

// ---------------------------------------------------------------------------
// UTF-8 ↔ String encoding
// ---------------------------------------------------------------------------

///| Write a MoonBit String as UTF-8 into linear memory at `ptr`.
///  Returns the number of bytes written.
fn write_utf8(s : String, ptr : Int) -> Int {
  let mut offset = 0
  for c in s {
    let code = c.to_int()
    if code < 0x80 {
      wasm_store8(ptr + offset, code)
      offset = offset + 1
    } else if code < 0x800 {
      wasm_store8(ptr + offset, 0xC0 | (code >> 6))
      wasm_store8(ptr + offset + 1, 0x80 | (code & 0x3F))
      offset = offset + 2
    } else if code < 0x10000 {
      wasm_store8(ptr + offset, 0xE0 | (code >> 12))
      wasm_store8(ptr + offset + 1, 0x80 | ((code >> 6) & 0x3F))
      wasm_store8(ptr + offset + 2, 0x80 | (code & 0x3F))
      offset = offset + 3
    } else {
      wasm_store8(ptr + offset, 0xF0 | (code >> 18))
      wasm_store8(ptr + offset + 1, 0x80 | ((code >> 12) & 0x3F))
      wasm_store8(ptr + offset + 2, 0x80 | ((code >> 6) & 0x3F))
      wasm_store8(ptr + offset + 3, 0x80 | (code & 0x3F))
      offset = offset + 4
    }
  }
  offset
}

///| Compute the UTF-8 byte length of a MoonBit String.
fn utf8_byte_len(s : String) -> Int {
  let mut len = 0
  for c in s {
    let code = c.to_int()
    if code < 0x80 {
      len = len + 1
    } else if code < 0x800 {
      len = len + 2
    } else if code < 0x10000 {
      len = len + 3
    } else {
      len = len + 4
    }
  }
  len
}

///| Allocate memory and write a String as UTF-8. Returns (ptr, byte_len).
pub fn write_string_to_mem(s : String) -> (Int, Int) {
  let byte_len = utf8_byte_len(s)
  if byte_len == 0 {
    return (0, 0)
  }
  let ptr = alloc(byte_len)
  if ptr == 0 {
    return (0, 0)
  }
  let written = write_utf8(s, ptr)
  (ptr, written)
}

///| Read a UTF-8 byte sequence from linear memory into a MoonBit String.
pub fn read_string_from_mem(ptr : Int, len : Int) -> String {
  if len == 0 {
    return ""
  }
  let mut result = ""
  let mut i = 0
  while i < len {
    let b = wasm_load8(ptr + i)
    if b < 0x80 {
      result = result + Int::unsafe_to_char(b).to_string()
      i = i + 1
    } else if b < 0xE0 {
      let c = ((b & 0x1F) << 6) | (wasm_load8(ptr + i + 1) & 0x3F)
      result = result + Int::unsafe_to_char(c).to_string()
      i = i + 2
    } else if b < 0xF0 {
      let c = ((b & 0x0F) << 12) |
        ((wasm_load8(ptr + i + 1) & 0x3F) << 6) |
        (wasm_load8(ptr + i + 2) & 0x3F)
      result = result + Int::unsafe_to_char(c).to_string()
      i = i + 3
    } else {
      let c = ((b & 0x07) << 18) |
        ((wasm_load8(ptr + i + 1) & 0x3F) << 12) |
        ((wasm_load8(ptr + i + 2) & 0x3F) << 6) |
        (wasm_load8(ptr + i + 3) & 0x3F)
      result = result + Int::unsafe_to_char(c).to_string()
      i = i + 4
    }
  }
  result
}

// ---------------------------------------------------------------------------
// High-level helpers
// ---------------------------------------------------------------------------

///| Serialize a JSON string to linear memory and return a packed i64.
pub fn serialize_to_mem(json : String) -> Int64 {
  let (ptr, len) = write_string_to_mem(json)
  pack_result(ptr, len)
}

///| Serialize a node definition and return a packed i64.
pub fn serialize_definition(def : NodeDefinition) -> Int64 {
  serialize_to_mem(def.to_json())
}

///| Serialize an execution result and return a packed i64.
pub fn serialize_result(result : ExecutionResult) -> Int64 {
  serialize_to_mem(result.to_json())
}

///| Serialize a multi-node package and return a packed i64.
pub fn serialize_package(pkg : PackageNodes) -> Int64 {
  serialize_to_mem(pkg.to_json())
}

///| Parse execution input from raw memory bytes.
pub fn parse_input(ptr : Int, len : Int) -> ExecutionInput {
  let json_str = read_string_from_mem(ptr, len)
  let val = parse_json(json_str)
  ExecutionInput::from_json(val)
}
