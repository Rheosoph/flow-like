// Flow-Like WASM Node Template (Zig) — Component Model
//
// Uses wit-bindgen-c generated C headers imported via @cImport.
//
// Build:
//   mise run setup      # install wasm-tools, wit-bindgen
//   mise run generate   # generate C bindings from WIT
//   mise run build      # compile + wrap into WASM component
//
// The compiled component will be at: node.wasm

const std = @import("std");

// wit-bindgen-c generated header
const wit = @cImport({
    @cInclude("flow_like_node.h");
});

// cabi_realloc is defined in the generated C code but not in the header
extern fn cabi_realloc(?*anyopaque, usize, usize, usize) ?[*]u8;

// ============================================================================
// ABI Version
// ============================================================================

const ABI_VERSION: u32 = 1;

// ============================================================================
// String helpers
// ============================================================================

fn toWitString(s: []const u8) wit.flow_like_node_string_t {
    return .{
        .ptr = @constCast(@ptrCast(s.ptr)),
        .len = s.len,
    };
}

fn fromWitString(ws: *const wit.flow_like_node_string_t) []const u8 {
    if (ws.ptr == null or ws.len == 0) return "";
    const p: [*]const u8 = @ptrCast(ws.ptr.?);
    return p[0..ws.len];
}

fn setWitResult(src: []const u8, ret: *wit.flow_like_node_string_t) void {
    const buf: [*]u8 = cabi_realloc(
        null,
        0,
        1,
        src.len,
    ) orelse return;
    @memcpy(buf[0..src.len], src);
    ret.ptr = buf;
    ret.len = src.len;
}

// ============================================================================
// JSON helpers
// ============================================================================

fn jsonQuote(comptime capacity: usize, s: []const u8) []const u8 {
    var buf: [capacity]u8 = undefined;
    var pos: usize = 0;

    buf[pos] = '"';
    pos += 1;

    for (s) |c| {
        switch (c) {
            '"' => {
                buf[pos] = '\\';
                buf[pos + 1] = '"';
                pos += 2;
            },
            '\\' => {
                buf[pos] = '\\';
                buf[pos + 1] = '\\';
                pos += 2;
            },
            '\n' => {
                buf[pos] = '\\';
                buf[pos + 1] = 'n';
                pos += 2;
            },
            '\r' => {
                buf[pos] = '\\';
                buf[pos + 1] = 'r';
                pos += 2;
            },
            '\t' => {
                buf[pos] = '\\';
                buf[pos + 1] = 't';
                pos += 2;
            },
            else => {
                buf[pos] = c;
                pos += 1;
            },
        }
    }
    buf[pos] = '"';
    pos += 1;

    const result = allocator.alloc(u8, pos) catch return "\"\"";
    @memcpy(result, buf[0..pos]);
    return result;
}

// ============================================================================
// Allocator (use page_allocator for WASM)
// ============================================================================

const allocator = std.heap.page_allocator;

// ============================================================================
// Pin / Node definition JSON builders
// ============================================================================

const PinType = enum { input, output };
const DataType = enum { exec, string, i64_type, f64_type, bool_type };

fn dataTypeStr(dt: DataType) []const u8 {
    return switch (dt) {
        .exec => "Exec",
        .string => "String",
        .i64_type => "I64",
        .f64_type => "F64",
        .bool_type => "Bool",
    };
}

fn buildPinJson(name: []const u8, friendly_name: []const u8, description: []const u8, pin_type: PinType, data_type: DataType, default_value: ?[]const u8) []const u8 {
    const pt = if (pin_type == .input) "Input" else "Output";
    const dt = dataTypeStr(data_type);
    const default_part = if (default_value) |dv|
        std.fmt.allocPrint(allocator, ",\"default_value\":{s}", .{dv}) catch ""
    else
        "";

    return std.fmt.allocPrint(allocator,
        "{{\"name\":\"{s}\",\"friendly_name\":\"{s}\",\"description\":\"{s}\",\"pin_type\":\"{s}\",\"data_type\":\"{s}\"{s}}}",
        .{ name, friendly_name, description, pt, dt, default_part },
    ) catch "{}";
}

fn buildNodeJson() []const u8 {
    const pins = [_][]const u8{
        buildPinJson("exec", "Execute", "Trigger execution", .input, .exec, null),
        buildPinJson("input_text", "Input Text", "Text to process", .input, .string, "\"\\\"\\\"\""),
        buildPinJson("multiplier", "Multiplier", "Number of times to repeat", .input, .i64_type, "\"1\""),
        buildPinJson("exec_out", "Done", "Execution complete", .output, .exec, null),
        buildPinJson("output_text", "Output Text", "Processed text", .output, .string, null),
        buildPinJson("char_count", "Character Count", "Number of characters in output", .output, .i64_type, null),
    };

    var pins_json = std.ArrayList(u8).init(allocator);
    pins_json.appendSlice("[") catch {};
    for (pins, 0..) |p, i| {
        if (i > 0) pins_json.appendSlice(",") catch {};
        pins_json.appendSlice(p) catch {};
    }
    pins_json.appendSlice("]") catch {};

    return std.fmt.allocPrint(allocator,
        "{{\"name\":\"my_custom_node_zig\",\"friendly_name\":\"My Custom Node (Zig)\",\"description\":\"A template WASM node built with Zig (Component Model)\",\"category\":\"Custom/WASM\",\"pins\":{s},\"long_running\":false,\"abi_version\":{d},\"permissions\":[\"streaming\"]}}",
        .{ pins_json.items, ABI_VERSION },
    ) catch "{}";
}

// ============================================================================
// Execution result builder
// ============================================================================

const ExecutionResult = struct {
    outputs_json: std.ArrayList(u8),
    exec_pins_json: std.ArrayList(u8),
    output_count: usize,
    exec_count: usize,
    err: ?[]const u8,

    fn init() ExecutionResult {
        return .{
            .outputs_json = std.ArrayList(u8).init(allocator),
            .exec_pins_json = std.ArrayList(u8).init(allocator),
            .output_count = 0,
            .exec_count = 0,
            .err = null,
        };
    }

    fn toJson(self: *ExecutionResult) []const u8 {
        const err_part = if (self.err) |e|
            std.fmt.allocPrint(allocator, ",\"error\":\"{s}\"", .{e}) catch ""
        else
            "";
        return std.fmt.allocPrint(allocator,
            "{{\"outputs\":{{{s}}},\"activate_exec\":[{s}],\"pending\":false{s}}}",
            .{ self.outputs_json.items, self.exec_pins_json.items, err_part },
        ) catch "{}";
    }
};

// ============================================================================
// Context — wraps WIT import functions
// ============================================================================

const Context = struct {
    result: ExecutionResult,

    fn init() Context {
        return .{ .result = ExecutionResult.init() };
    }

    fn getInputRaw(self: *const Context, name: []const u8) ?[]const u8 {
        _ = self;
        var wname = toWitString(name);
        var ret: wit.flow_like_node_string_t = undefined;
        if (wit.flow_like_node_pins_get_input(&wname, &ret)) {
            return fromWitString(&ret);
        }
        return null;
    }

    fn getString(self: *const Context, name: []const u8, default: []const u8) []const u8 {
        const raw = self.getInputRaw(name) orelse return default;
        if (raw.len == 0) return default;
        // Strip surrounding quotes if present
        if (raw.len >= 2 and raw[0] == '"' and raw[raw.len - 1] == '"') {
            return raw[1 .. raw.len - 1];
        }
        return raw;
    }

    fn getI64(self: *const Context, name: []const u8, default: i64) i64 {
        const raw = self.getInputRaw(name) orelse return default;
        if (raw.len == 0) return default;
        return std.fmt.parseInt(i64, raw, 10) catch default;
    }

    fn setOutput(self: *Context, name: []const u8, json_value: []const u8) void {
        var wname = toWitString(name);
        var wval = toWitString(json_value);
        wit.flow_like_node_pins_set_output(&wname, &wval);

        if (self.result.output_count > 0) {
            self.result.outputs_json.appendSlice(",") catch {};
        }
        self.result.outputs_json.appendSlice("\"") catch {};
        self.result.outputs_json.appendSlice(name) catch {};
        self.result.outputs_json.appendSlice("\":") catch {};
        self.result.outputs_json.appendSlice(json_value) catch {};
        self.result.output_count += 1;
    }

    fn activateExec(self: *Context, pin: []const u8) void {
        var wpin = toWitString(pin);
        wit.flow_like_node_pins_activate_exec(&wpin);

        if (self.result.exec_count > 0) {
            self.result.exec_pins_json.appendSlice(",") catch {};
        }
        self.result.exec_pins_json.appendSlice("\"") catch {};
        self.result.exec_pins_json.appendSlice(pin) catch {};
        self.result.exec_pins_json.appendSlice("\"") catch {};
        self.result.exec_count += 1;
    }

    fn log(_: *const Context, level: u8, msg: []const u8) void {
        var wmsg = toWitString(msg);
        wit.flow_like_node_logging_log(level, &wmsg);
    }

    fn streamText(_: *const Context, content: []const u8) void {
        var wcontent = toWitString(content);
        wit.flow_like_node_streaming_text(&wcontent);
    }

    fn success(self: *Context) []const u8 {
        self.activateExec("exec_out");
        return self.result.toJson();
    }

    fn fail(self: *Context, msg: []const u8) []const u8 {
        self.result.err = msg;
        return self.result.toJson();
    }
};

// ============================================================================
// Node Execution
// ============================================================================

fn handleRun(ctx: *Context) []const u8 {
    const input_text = ctx.getString("input_text", "");
    const multiplier = ctx.getI64("multiplier", 1);

    ctx.log(0, "Processing input text");

    var buf = std.ArrayList(u8).init(allocator);
    var i: i64 = 0;
    while (i < multiplier) : (i += 1) {
        buf.appendSlice(input_text) catch {};
    }
    const output_text = buf.items;
    const char_count = output_text.len;

    const msg = std.fmt.allocPrint(allocator, "Generated {d} characters", .{char_count}) catch "Generated characters";
    ctx.streamText(msg);

    const quoted_output = jsonQuote(8192, output_text);
    ctx.setOutput("output_text", quoted_output);

    const count_str = std.fmt.allocPrint(allocator, "{d}", .{char_count}) catch "0";
    ctx.setOutput("char_count", count_str);

    return ctx.success();
}

// ============================================================================
// WIT Component Exports (wit-bindgen-c naming convention)
// ============================================================================

export fn exports_flow_like_node_get_node(ret: *wit.flow_like_node_string_t) void {
    const json = buildNodeJson();
    setWitResult(json, ret);
}

export fn exports_flow_like_node_get_nodes(ret: *wit.flow_like_node_string_t) void {
    const node_json = buildNodeJson();
    const json = std.fmt.allocPrint(allocator, "[{s}]", .{node_json}) catch "[]";
    setWitResult(json, ret);
}

export fn exports_flow_like_node_run(input: *wit.flow_like_node_string_t, ret: *wit.flow_like_node_string_t) void {
    _ = input; // inputs are read via WIT pins interface
    var ctx = Context.init();
    const json = handleRun(&ctx);
    setWitResult(json, ret);
}

export fn exports_flow_like_node_get_abi_version() u32 {
    return ABI_VERSION;
}

// WASI reactor: provide main to satisfy libc linkage
pub fn main() void {}
