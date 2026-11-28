//! Core WASI shims shared by execution and metadata hosts.

use crate::error::{WasmError, WasmResult};
use wasmtime::{Caller, Linker, Memory};

pub(crate) const WASI_ERRNO_FAULT: i32 = 21;

fn wasi_memory<T: 'static>(caller: &mut Caller<'_, T>, ptr: i32) -> Result<(Memory, usize), ()> {
    let offset = usize::try_from(ptr).map_err(|_| ())?;
    let memory = caller
        .get_export("memory")
        .and_then(|export| export.into_memory())
        .ok_or(())?;
    Ok((memory, offset))
}

fn write_wasi_bytes<T: 'static>(
    caller: &mut Caller<'_, T>,
    ptr: i32,
    bytes: &[u8],
) -> Result<(), ()> {
    let (memory, offset) = wasi_memory(caller, ptr)?;
    memory.write(caller, offset, bytes).map_err(|_| ())
}

fn write_wasi_u32<T: 'static>(caller: &mut Caller<'_, T>, ptr: i32, value: u32) -> Result<(), ()> {
    write_wasi_bytes(caller, ptr, &value.to_le_bytes())
}

fn read_wasi_u32<T: 'static>(caller: &mut Caller<'_, T>, ptr: i32) -> Result<u32, ()> {
    let (memory, offset) = wasi_memory(caller, ptr)?;
    let mut bytes = [0; 4];
    memory.read(&*caller, offset, &mut bytes).map_err(|_| ())?;
    Ok(u32::from_le_bytes(bytes))
}

/// Guests get no arguments and no environment, so both counts are zero.
fn write_empty_list_sizes<T: 'static>(
    caller: &mut Caller<'_, T>,
    count: i32,
    buf_size: i32,
) -> i32 {
    if write_wasi_u32(caller, count, 0).is_err() || write_wasi_u32(caller, buf_size, 0).is_err() {
        return WASI_ERRNO_FAULT;
    }
    0
}

/// Register WASI snapshot_preview1 stubs for TinyGo/Go WASM modules
pub(crate) fn register<T: 'static>(linker: &mut Linker<T>) -> WasmResult<()> {
    linker
        .func_wrap(
            "wasi_snapshot_preview1",
            "proc_exit",
            |_caller: Caller<'_, T>, _code: i32| {},
        )
        .map_err(|e| {
            WasmError::Initialization(format!("Failed to register wasi proc_exit stub: {}", e))
        })?;

    linker
        .func_wrap(
            "wasi_snapshot_preview1",
            "fd_write",
            |mut caller: Caller<'_, T>, _fd: i32, iovs: i32, iovs_len: i32, nwritten: i32| -> i32 {
                // Output is discarded; reporting it as written lets guest stdio loops finish.
                let mut written = 0u32;
                for index in 0..iovs_len.max(0) {
                    let Some(len_ptr) = index
                        .checked_mul(8)
                        .and_then(|entry| entry.checked_add(iovs))
                        .and_then(|entry| entry.checked_add(4))
                    else {
                        return WASI_ERRNO_FAULT;
                    };
                    let Ok(len) = read_wasi_u32(&mut caller, len_ptr) else {
                        return WASI_ERRNO_FAULT;
                    };
                    written = written.saturating_add(len);
                }
                if write_wasi_u32(&mut caller, nwritten, written).is_err() {
                    return WASI_ERRNO_FAULT;
                }
                0
            },
        )
        .map_err(|e| {
            WasmError::Initialization(format!("Failed to register wasi fd_write stub: {}", e))
        })?;

    linker
        .func_wrap(
            "wasi_snapshot_preview1",
            "fd_close",
            |_caller: Caller<'_, T>, _fd: i32| -> i32 { 0 },
        )
        .map_err(|e| {
            WasmError::Initialization(format!("Failed to register wasi fd_close stub: {}", e))
        })?;

    linker
        .func_wrap(
            "wasi_snapshot_preview1",
            "fd_seek",
            |mut caller: Caller<'_, T>,
             _fd: i32,
             _offset: i64,
             _whence: i32,
             newoffset: i32|
             -> i32 {
                if write_wasi_bytes(&mut caller, newoffset, &0u64.to_le_bytes()).is_err() {
                    return WASI_ERRNO_FAULT;
                }
                0
            },
        )
        .map_err(|e| {
            WasmError::Initialization(format!("Failed to register wasi fd_seek stub: {}", e))
        })?;

    linker
        .func_wrap(
            "wasi_snapshot_preview1",
            "fd_fdstat_get",
            |_caller: Caller<'_, T>, _fd: i32, _buf: i32| -> i32 { 0 },
        )
        .map_err(|e| {
            WasmError::Initialization(format!("Failed to register wasi fd_fdstat_get stub: {}", e))
        })?;

    linker
        .func_wrap(
            "wasi_snapshot_preview1",
            "environ_sizes_get",
            |mut caller: Caller<'_, T>, count: i32, buf_size: i32| -> i32 {
                write_empty_list_sizes(&mut caller, count, buf_size)
            },
        )
        .map_err(|e| {
            WasmError::Initialization(format!(
                "Failed to register wasi environ_sizes_get stub: {}",
                e
            ))
        })?;

    linker
        .func_wrap(
            "wasi_snapshot_preview1",
            "environ_get",
            |_caller: Caller<'_, T>, _environ: i32, _environ_buf: i32| -> i32 { 0 },
        )
        .map_err(|e| {
            WasmError::Initialization(format!("Failed to register wasi environ_get stub: {}", e))
        })?;

    linker
        .func_wrap(
            "wasi_snapshot_preview1",
            "args_sizes_get",
            |mut caller: Caller<'_, T>, argc: i32, argv_buf_size: i32| -> i32 {
                write_empty_list_sizes(&mut caller, argc, argv_buf_size)
            },
        )
        .map_err(|e| {
            WasmError::Initialization(format!(
                "Failed to register wasi args_sizes_get stub: {}",
                e
            ))
        })?;

    linker
        .func_wrap(
            "wasi_snapshot_preview1",
            "args_get",
            |_caller: Caller<'_, T>, _argv: i32, _argv_buf: i32| -> i32 { 0 },
        )
        .map_err(|e| {
            WasmError::Initialization(format!("Failed to register wasi args_get stub: {}", e))
        })?;

    linker
        .func_wrap(
            "wasi_snapshot_preview1",
            "clock_time_get",
            |_caller: Caller<'_, T>, _clock_id: i32, _precision: i64, _time: i32| -> i32 { 0 },
        )
        .map_err(|e| {
            WasmError::Initialization(format!(
                "Failed to register wasi clock_time_get stub: {}",
                e
            ))
        })?;

    linker
        .func_wrap(
            "wasi_snapshot_preview1",
            "fd_read",
            |mut caller: Caller<'_, T>, _fd: i32, _iovs: i32, _iovs_len: i32, nread: i32| -> i32 {
                if write_wasi_u32(&mut caller, nread, 0).is_err() {
                    return WASI_ERRNO_FAULT;
                }
                0
            },
        )
        .map_err(|e| {
            WasmError::Initialization(format!("Failed to register wasi fd_read stub: {}", e))
        })?;

    linker
        .func_wrap(
            "wasi_snapshot_preview1",
            "random_get",
            |_caller: Caller<'_, T>, _buf: i32, _buf_len: i32| -> i32 { 0 },
        )
        .map_err(|e| {
            WasmError::Initialization(format!("Failed to register wasi random_get stub: {}", e))
        })?;

    // fd_prestat_get / fd_prestat_dir_name. used by Swift/WASM to discover preopened dirs.
    // We have none, so return EBADF (8) immediately.
    linker
        .func_wrap(
            "wasi_snapshot_preview1",
            "fd_prestat_get",
            |_caller: Caller<'_, T>, _fd: i32, _buf: i32| -> i32 {
                8 // WASI_EBADF. no preopened directories
            },
        )
        .map_err(|e| {
            WasmError::Initialization(format!(
                "Failed to register wasi fd_prestat_get stub: {}",
                e
            ))
        })?;

    linker
        .func_wrap(
            "wasi_snapshot_preview1",
            "fd_prestat_dir_name",
            |_caller: Caller<'_, T>, _fd: i32, _path: i32, _path_len: i32| -> i32 {
                8 // WASI_EBADF
            },
        )
        .map_err(|e| {
            WasmError::Initialization(format!(
                "Failed to register wasi fd_prestat_dir_name stub: {}",
                e
            ))
        })?;

    // path_open. opens a file relative to a preopened directory; no filesystem in WASM sandbox.
    linker
        .func_wrap(
            "wasi_snapshot_preview1",
            "path_open",
            |_caller: Caller<'_, T>,
             _dirfd: i32,
             _dirflags: i32,
             _path: i32,
             _path_len: i32,
             _oflags: i32,
             _fs_rights_base: i64,
             _fs_rights_inheriting: i64,
             _fdflags: i32,
             _opened_fd: i32|
             -> i32 { 28 }, // WASI_ENOSYS
        )
        .map_err(|e| {
            WasmError::Initialization(format!("Failed to register wasi path_open stub: {}", e))
        })?;

    Ok(())
}
