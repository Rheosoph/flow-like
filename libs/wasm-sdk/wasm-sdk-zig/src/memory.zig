const std = @import("std");

var arena = std.heap.ArenaAllocator.init(std.heap.page_allocator);
pub const allocator = arena.allocator();

// ---------------------------------------------------------------------------
// Pack / unpack helpers — upper 32 bits = pointer, lower 32 bits = length
// ---------------------------------------------------------------------------

pub fn packI64(ptr: u32, len: u32) i64 {
    return @as(i64, @intCast(ptr)) << 32 | @as(i64, @intCast(len));
}

pub fn unpackHigh(val: i64) u32 {
    return @intCast(@as(u64, @bitCast(val)) >> 32);
}

pub fn unpackLow(val: i64) u32 {
    return @intCast(@as(u64, @bitCast(val)) & 0xFFFFFFFF);
}

pub fn unpackI64(val: i64) struct { ptr: u32, len: u32 } {
    return .{ .ptr = unpackHigh(val), .len = unpackLow(val) };
}

// ---------------------------------------------------------------------------
// Slice ↔ raw pointer conversion
// ---------------------------------------------------------------------------

pub fn ptrToSlice(ptr: u32, len: u32) []const u8 {
    if (ptr == 0 or len == 0) return "";
    const p: [*]const u8 = @ptrFromInt(ptr);
    return p[0..len];
}

pub fn sliceToRaw(s: []const u8) struct { ptr: u32, len: u32 } {
    if (s.len == 0) return .{ .ptr = 0, .len = 0 };
    return .{ .ptr = @intFromPtr(s.ptr), .len = @intCast(s.len) };
}

pub fn packResult(s: []const u8) i64 {
    if (s.len == 0) return 0;
    // Copy into a fresh, dedicated allocation so the host always reads
    // from a stable, contiguous region that cannot overlap with internal
    // ArrayList bookkeeping or freed capacity.
    const buf = allocator.alloc(u8, s.len) catch return 0;
    @memcpy(buf, s);
    return packI64(@intFromPtr(buf.ptr), @intCast(buf.len));
}

// ---------------------------------------------------------------------------
// Exported WASM memory management
// ---------------------------------------------------------------------------

pub fn wasmAlloc(size: u32) u32 {
    const slice = allocator.alloc(u8, @intCast(size)) catch return 0;
    return @intFromPtr(slice.ptr);
}

pub fn wasmDealloc(_: u32, _: u32) void {
    // Arena allocator — individual frees are a no-op.
    // Memory is reclaimed when the WASM instance is destroyed.
}
