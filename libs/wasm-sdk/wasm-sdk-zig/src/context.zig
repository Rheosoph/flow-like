const std = @import("std");
const types = @import("types.zig");
const host = @import("host.zig");
const mem = @import("memory.zig");

const ExecutionInput = types.ExecutionInput;
const ExecutionResult = types.ExecutionResult;

pub const Context = struct {
    input: ExecutionInput,
    result: ExecutionResult,

    pub fn init(input: ExecutionInput) Context {
        return .{
            .input = input,
            .result = ExecutionResult.success(mem.allocator),
        };
    }

    // --- Metadata ---

    pub fn nodeId(self: *const Context) []const u8 {
        return self.input.node_id;
    }
    pub fn nodeName(self: *const Context) []const u8 {
        return self.input.node_name;
    }
    pub fn runId(self: *const Context) []const u8 {
        return self.input.run_id;
    }
    pub fn appId(self: *const Context) []const u8 {
        return self.input.app_id;
    }
    pub fn boardId(self: *const Context) []const u8 {
        return self.input.board_id;
    }
    pub fn userId(self: *const Context) []const u8 {
        return self.input.user_id;
    }
    pub fn streamEnabled(self: *const Context) bool {
        return self.input.stream_state;
    }
    pub fn logLevelValue(self: *const Context) u8 {
        return self.input.log_level;
    }

    // --- Input getters ---

    pub fn getInput(self: *const Context, name: []const u8) ?[]const u8 {
        return self.input.inputs.get(name);
    }

    pub fn getString(self: *const Context, name: []const u8, default: []const u8) []const u8 {
        const v = self.input.inputs.get(name) orelse return default;
        if (v.len >= 2 and v[0] == '"' and v[v.len - 1] == '"') {
            return v[1 .. v.len - 1];
        }
        return v;
    }

    pub fn getI64(self: *const Context, name: []const u8, default: i64) i64 {
        const v = self.input.inputs.get(name) orelse return default;
        return std.fmt.parseInt(i64, v, 10) catch default;
    }

    pub fn getF64(self: *const Context, name: []const u8, default: f64) f64 {
        const v = self.input.inputs.get(name) orelse return default;
        return std.fmt.parseFloat(f64, v) catch default;
    }

    pub fn getBool(self: *const Context, name: []const u8, default: bool) bool {
        const v = self.input.inputs.get(name) orelse return default;
        return std.mem.eql(u8, v, "true");
    }

    // --- Output setters ---

    pub fn setOutput(self: *Context, name: []const u8, value: []const u8) void {
        self.result.setOutput(name, value);
    }

    pub fn activateExec(self: *Context, pin_name: []const u8) void {
        self.result.activateExecPin(pin_name);
    }

    pub fn setPending(self: *Context, pending: bool) void {
        self.result.pending = pending;
    }

    pub fn setError(self: *Context, err: []const u8) void {
        self.result.error_msg = err;
    }

    // --- Level-gated logging ---

    fn shouldLog(self: *const Context, level: u8) bool {
        return level >= self.input.log_level;
    }

    pub fn debug(self: *const Context, msg: []const u8) void {
        if (self.shouldLog(types.log_level_debug)) host.logDebug(msg);
    }

    pub fn info(self: *const Context, msg: []const u8) void {
        if (self.shouldLog(types.log_level_info)) host.logInfo(msg);
    }

    pub fn warn(self: *const Context, msg: []const u8) void {
        if (self.shouldLog(types.log_level_warn)) host.logWarn(msg);
    }

    pub fn logError(self: *const Context, msg: []const u8) void {
        if (self.shouldLog(types.log_level_error)) host.logError(msg);
    }

    // --- Streaming ---

    pub fn streamText(self: *const Context, txt: []const u8) void {
        if (self.streamEnabled()) host.streamText(txt);
    }

    pub fn streamJson(self: *const Context, data: []const u8) void {
        if (self.streamEnabled()) host.streamEmit("json", data);
    }

    pub fn streamProgress(self: *const Context, progress: f32, message: []const u8) void {
        if (!self.streamEnabled()) return;
        var buf: [256]u8 = undefined;
        const json = std.fmt.bufPrint(&buf, "{{\"progress\":{d},\"message\":\"{s}\"}}", .{ progress, message }) catch return;
        host.streamEmit("progress", json);
    }

    // --- Variables ---

    pub fn getVariable(self: *const Context, name: []const u8) []const u8 {
        _ = self;
        return host.getVariable(name);
    }

    pub fn setVariable(self: *const Context, name: []const u8, value: []const u8) void {
        _ = self;
        host.setVariable(name, value);
    }

    // --- Finalize ---

    pub fn finish(self: *Context) ExecutionResult {
        return self.result;
    }

    pub fn success(self: *Context) ExecutionResult {
        self.activateExec("exec_out");
        return self.finish();
    }

    pub fn fail(self: *Context, err: []const u8) ExecutionResult {
        self.setError(err);
        return self.finish();
    }
};
