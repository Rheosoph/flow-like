//! Memory utilities for WASM interactions
//!
//! Provides safe abstractions for reading/writing WASM linear memory.

use crate::error::{WasmError, WasmResult};
use wasmtime::Memory;

/// Helper for WASM memory operations
pub struct WasmMemory;

impl WasmMemory {
    /// Read bytes from WASM memory
    pub fn read_bytes(
        memory: &Memory,
        store: &impl wasmtime::AsContext,
        ptr: u32,
        len: u32,
    ) -> WasmResult<Vec<u8>> {
        let data = memory.data(store);
        let start = ptr as usize;
        let end = start
            .checked_add(len as usize)
            .ok_or_else(|| WasmError::memory_access("Memory address overflow"))?;

        if end > data.len() {
            return Err(WasmError::memory_access(format!(
                "Memory access out of bounds: trying to read {} bytes at offset {}, but memory size is {}",
                len, ptr, data.len()
            )));
        }

        Ok(data[start..end].to_vec())
    }

    /// Read a UTF-8 string from WASM memory
    pub fn read_string(
        memory: &Memory,
        store: &impl wasmtime::AsContext,
        ptr: u32,
        len: u32,
    ) -> WasmResult<String> {
        let bytes = Self::read_bytes(memory, store, ptr, len)?;
        String::from_utf8(bytes)
            .map_err(|e| WasmError::memory_access(format!("Invalid UTF-8 string: {}", e)))
    }

    /// Write bytes to WASM memory
    pub fn write_bytes(
        memory: &Memory,
        store: &mut impl wasmtime::AsContextMut,
        ptr: u32,
        data: &[u8],
    ) -> WasmResult<()> {
        let mem_data = memory.data_mut(store);
        let start = ptr as usize;
        let end = start
            .checked_add(data.len())
            .ok_or_else(|| WasmError::memory_access("Memory address overflow"))?;

        if end > mem_data.len() {
            return Err(WasmError::memory_access(format!(
                "Memory access out of bounds: trying to write {} bytes at offset {}, but memory size is {}",
                data.len(), ptr, mem_data.len()
            )));
        }

        mem_data[start..end].copy_from_slice(data);
        Ok(())
    }

    /// Write a string to WASM memory
    pub fn write_string(
        memory: &Memory,
        store: &mut impl wasmtime::AsContextMut,
        ptr: u32,
        s: &str,
    ) -> WasmResult<()> {
        Self::write_bytes(memory, store, ptr, s.as_bytes())
    }

    /// Get current memory size in bytes
    pub fn size(memory: &Memory, store: &impl wasmtime::AsContext) -> usize {
        memory.data(store).len()
    }

    /// Grow memory by the specified number of pages (64KB each)
    pub fn grow(
        memory: &Memory,
        store: &mut impl wasmtime::AsContextMut,
        pages: u64,
    ) -> WasmResult<u64> {
        memory
            .grow(store, pages)
            .map_err(|e| WasmError::memory_access(format!("Failed to grow memory: {}", e)))
    }
}

/// Result buffer management for host function returns
///
/// When host functions need to return variable-length data, they write to a
/// result buffer and return a packed pointer+length value.
#[derive(Debug)]
pub struct ResultBuffer {
    /// Pointer to allocated buffer in WASM memory
    pub ptr: u32,
    /// Length of data in buffer
    pub len: u32,
}

impl ResultBuffer {
    /// Create a new result buffer by allocating memory in WASM
    pub fn new(ptr: u32, len: u32) -> Self {
        Self { ptr, len }
    }

    /// Pack into i64 for return value
    #[inline]
    pub fn pack(&self) -> i64 {
        crate::abi::WasmAbi::pack_ptr_len(self.ptr, self.len)
    }

    /// Unpack from i64 return value
    #[inline]
    pub fn unpack(packed: i64) -> Option<Self> {
        if packed < 0 {
            return None;
        }
        let (ptr, len) = crate::abi::WasmAbi::unpack_ptr_len(packed);
        Some(Self { ptr, len })
    }
}

/// String marshalling utilities
pub struct StringMarshaller;

impl StringMarshaller {
    /// Calculate the size needed to store a JSON value
    pub fn json_size(value: &serde_json::Value) -> usize {
        serde_json::to_string(value).map(|s| s.len()).unwrap_or(2)
    }

    /// Serialize a value to JSON bytes
    pub fn to_json_bytes<T: serde::Serialize>(value: &T) -> WasmResult<Vec<u8>> {
        serde_json::to_vec(value).map_err(WasmError::Json)
    }

    /// Deserialize JSON bytes to a value
    pub fn from_json_bytes<T: serde::de::DeserializeOwned>(bytes: &[u8]) -> WasmResult<T> {
        serde_json::from_slice(bytes).map_err(WasmError::Json)
    }
}

/// Allocator wrapper for WASM memory
///
/// Uses the module's exported alloc/dealloc functions if available,
/// otherwise uses a simple bump allocator for temporary data.
pub struct WasmAllocator {
    /// Function to call for allocation (if module exports one)
    #[allow(dead_code)]
    // records whether the module exports its own alloc; bump path is selected by instance.rs::allocate today
    has_alloc: bool,
    /// Simple bump pointer for modules without allocator
    bump_ptr: u32,
    /// Maximum memory we're allowed to use
    max_memory: u32,
}

impl WasmAllocator {
    pub fn new(has_alloc: bool, initial_ptr: u32, max_memory: u32) -> Self {
        Self {
            has_alloc,
            bump_ptr: initial_ptr,
            max_memory,
        }
    }

    /// Allocate memory using bump allocator (for modules without alloc export)
    pub fn bump_alloc(&mut self, size: u32, align: u32) -> WasmResult<u32> {
        // Align the bump pointer
        let aligned = (self.bump_ptr + align - 1) & !(align - 1);
        let end = aligned
            .checked_add(size)
            .ok_or_else(|| WasmError::OutOfMemory {
                requested: size as usize,
                limit: self.max_memory as usize,
            })?;

        if end > self.max_memory {
            return Err(WasmError::OutOfMemory {
                requested: size as usize,
                limit: self.max_memory as usize,
            });
        }

        self.bump_ptr = end;
        Ok(aligned)
    }

    /// Reset bump allocator (for reuse between calls)
    pub fn reset(&mut self, ptr: u32) {
        self.bump_ptr = ptr;
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_result_buffer_pack_unpack() {
        let buf = ResultBuffer::new(0x1000, 256);
        let packed = buf.pack();
        let unpacked = ResultBuffer::unpack(packed).unwrap();
        assert_eq!(buf.ptr, unpacked.ptr);
        assert_eq!(buf.len, unpacked.len);
    }

    #[test]
    fn test_bump_allocator() {
        let mut alloc = WasmAllocator::new(false, 1000, 10000);

        // First allocation
        let ptr1 = alloc.bump_alloc(100, 8).unwrap();
        assert_eq!(ptr1, 1000); // Already aligned

        // Second allocation (should be aligned)
        let ptr2 = alloc.bump_alloc(50, 8).unwrap();
        assert_eq!(ptr2, 1104); // 1000 + 100 = 1100, aligned to 8 = 1104

        // Reset and allocate again
        alloc.reset(1000);
        let ptr3 = alloc.bump_alloc(100, 8).unwrap();
        assert_eq!(ptr3, 1000);
    }

    #[test]
    fn test_bump_allocator_oom() {
        let mut alloc = WasmAllocator::new(false, 9900, 10000);

        // Should fail - not enough space
        let result = alloc.bump_alloc(200, 8);
        assert!(result.is_err());
    }
}
