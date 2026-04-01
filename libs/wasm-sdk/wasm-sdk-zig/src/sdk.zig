// Flow-Like WASM SDK for Zig
//
// Provides types, host bindings, context helpers, and memory management
// for building Flow-Like WASM nodes in Zig.
//
// Files:
//   types.zig   — JSON-serializable types (NodeDefinition, PinDefinition, etc.)
//   host.zig    — Host import declarations and Zig wrapper functions
//   context.zig — Context struct with high-level helpers
//   memory.zig  — alloc/dealloc exports and memory helpers

pub const types = @import("types.zig");
pub const host = @import("host.zig");
pub const context = @import("context.zig");
pub const mem = @import("memory.zig");

// Re-export commonly used types
pub const NodeDefinition = types.NodeDefinition;
pub const PinDefinition = types.PinDefinition;
pub const NodeScores = types.NodeScores;
pub const DataType = types.DataType;
pub const PinDirection = types.PinDirection;
pub const ExecutionInput = types.ExecutionInput;
pub const ExecutionResult = types.ExecutionResult;
pub const Context = context.Context;

// Re-export factory functions
pub const inputPin = PinDefinition.inputPin;
pub const outputPin = PinDefinition.outputPin;

// Re-export memory helpers
pub const allocator = mem.allocator;
pub const wasmAlloc = mem.wasmAlloc;
pub const wasmDealloc = mem.wasmDealloc;
pub const packResult = mem.packResult;
pub const packI64 = mem.packI64;
pub const unpackI64 = mem.unpackI64;
pub const ptrToSlice = mem.ptrToSlice;

pub const abi_version: u32 = 1;

pub fn parseInput(ptr: u32, len: u32) ExecutionInput {
    const json_str = mem.ptrToSlice(ptr, len);
    return ExecutionInput.fromJson(mem.allocator, json_str);
}

pub fn serializeDefinition(def: *const NodeDefinition) i64 {
    const json = def.toJson(mem.allocator) catch return 0;
    return mem.packResult(json);
}

pub fn serializeResult(result: *const ExecutionResult) i64 {
    const json = result.toJson(mem.allocator) catch return 0;
    return mem.packResult(json);
}

pub fn jsonString(s: []const u8) []const u8 {
    return types.jsonStringAlloc(mem.allocator, s) catch s;
}
