extern "wasm" fn test_memory_size() -> Int =
  #|(func (result i32) (memory.size))

test "scratch resets preserve heap gaps and reuse reserved pages" {
  let first = alloc(65536)
  let gap = wasm_memory_grow(1) * 65536
  wasm_store8(gap, 123)
  let second = alloc(65536)
  assert_true(second >= gap + 65536)
  assert_eq(wasm_load8(gap), 123)

  let retained = read_string_from_mem(first, 0) + "package state"
  let pages = test_memory_size()
  for _i = 0; _i < 100; _i = _i + 1 {
    reset_scratch()
    assert_eq(alloc(65536), first)
    assert_eq(alloc(65536), second)
  }
  assert_eq(test_memory_size(), pages)
  assert_eq(retained, "package state")
  assert_eq(wasm_load8(gap), 123)
  assert_eq(alloc(-1), 0)
  assert_eq(alloc(0x7fffffff), 0)
}
