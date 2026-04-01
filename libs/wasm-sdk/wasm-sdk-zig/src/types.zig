const std = @import("std");

pub const abi_version: u32 = 1;

pub const log_level_debug: u8 = 0;
pub const log_level_info: u8 = 1;
pub const log_level_warn: u8 = 2;
pub const log_level_error: u8 = 3;
pub const log_level_fatal: u8 = 4;

// ---------------------------------------------------------------------------
// DataType — the kind of data a pin carries
// ---------------------------------------------------------------------------

pub const DataType = enum {
    exec,
    string,
    i64_type,
    f64_type,
    bool_type,
    generic,
    byte,
    date_time,
    path_buf,
    struct_type,

    pub fn jsonName(self: DataType) []const u8 {
        return switch (self) {
            .exec => "Exec",
            .string => "String",
            .i64_type => "I64",
            .f64_type => "F64",
            .bool_type => "Bool",
            .generic => "Generic",
            .byte => "Bytes",
            .date_time => "Date",
            .path_buf => "PathBuf",
            .struct_type => "Struct",
        };
    }
};

// ---------------------------------------------------------------------------
// PinDirection — input vs output
// ---------------------------------------------------------------------------

pub const PinDirection = enum {
    input,
    output,

    pub fn jsonName(self: PinDirection) []const u8 {
        return switch (self) {
            .input => "Input",
            .output => "Output",
        };
    }
};

// ---------------------------------------------------------------------------
// ValueType — how the data is contained (scalar vs collection)
// ---------------------------------------------------------------------------

pub const ValueTypeKind = enum {
    normal,
    array,
    hash_map,
    hash_set,

    pub fn jsonName(self: ValueTypeKind) []const u8 {
        return switch (self) {
            .normal => "Normal",
            .array => "Array",
            .hash_map => "HashMap",
            .hash_set => "HashSet",
        };
    }
};

// ---------------------------------------------------------------------------
// NodeScores
// ---------------------------------------------------------------------------

pub const NodeScores = struct {
    privacy: u8 = 0,
    security: u8 = 0,
    performance: u8 = 0,
    governance: u8 = 0,
    reliability: u8 = 0,
    cost: u8 = 0,

    pub fn writeJson(self: *const NodeScores, writer: anytype) !void {
        try writer.writeAll("{\"privacy\":");
        try writeInt(writer, self.privacy);
        try writer.writeAll(",\"security\":");
        try writeInt(writer, self.security);
        try writer.writeAll(",\"performance\":");
        try writeInt(writer, self.performance);
        try writer.writeAll(",\"governance\":");
        try writeInt(writer, self.governance);
        try writer.writeAll(",\"reliability\":");
        try writeInt(writer, self.reliability);
        try writer.writeAll(",\"cost\":");
        try writeInt(writer, self.cost);
        try writer.writeByte('}');
    }
};

// ---------------------------------------------------------------------------
// PinDefinition
// ---------------------------------------------------------------------------

pub const PinDefinition = struct {
    name: []const u8,
    friendly_name: []const u8,
    description: []const u8,
    pin_type: PinDirection,
    data_type: DataType,
    default_value: ?[]const u8 = null,
    value_type: ?[]const u8 = null,
    schema: ?[]const u8 = null,
    step: ?f64 = null,
    sensitive: ?bool = null,
    enforce_schema: ?bool = null,
    enforce_generic_value_type: ?bool = null,

    pub fn inputPin(name: []const u8, friendly_name: []const u8, description: []const u8, data_type: DataType) PinDefinition {
        return .{
            .name = name,
            .friendly_name = friendly_name,
            .description = description,
            .pin_type = .input,
            .data_type = data_type,
        };
    }

    pub fn outputPin(name: []const u8, friendly_name: []const u8, description: []const u8, data_type: DataType) PinDefinition {
        return .{
            .name = name,
            .friendly_name = friendly_name,
            .description = description,
            .pin_type = .output,
            .data_type = data_type,
        };
    }

    pub fn withDefault(self: PinDefinition, value: []const u8) PinDefinition {
        var pin = self;
        pin.default_value = value;
        return pin;
    }

    /// Set the value type (e.g. "Array", "HashMap", "HashSet").
    pub fn withValueType(self: PinDefinition, vt: []const u8) PinDefinition {
        var pin = self;
        pin.value_type = vt;
        return pin;
    }

    /// Attach a raw JSON Schema string to this pin.
    pub fn withSchema(self: PinDefinition, s: []const u8) PinDefinition {
        var pin = self;
        pin.schema = s;
        return pin;
    }

    pub fn withStep(self: PinDefinition, step: f64) PinDefinition {
        var pin = self;
        pin.step = step;
        return pin;
    }

    pub fn withSensitive(self: PinDefinition, sensitive: bool) PinDefinition {
        var pin = self;
        pin.sensitive = sensitive;
        return pin;
    }

    pub fn withEnforceSchema(self: PinDefinition, enforce: bool) PinDefinition {
        var pin = self;
        pin.enforce_schema = enforce;
        return pin;
    }

    pub fn withEnforceGenericValueType(self: PinDefinition, enforce: bool) PinDefinition {
        var pin = self;
        pin.enforce_generic_value_type = enforce;
        return pin;
    }

    pub fn writeJson(self: *const PinDefinition, writer: anytype) !void {
        try writer.writeAll("{\"name\":");
        try writeJsonString(writer, self.name);
        try writer.writeAll(",\"friendly_name\":");
        try writeJsonString(writer, self.friendly_name);
        try writer.writeAll(",\"description\":");
        try writeJsonString(writer, self.description);
        try writer.writeAll(",\"pin_type\":\"");
        try writer.writeAll(self.pin_type.jsonName());
        try writer.writeAll("\",\"data_type\":\"");
        try writer.writeAll(self.data_type.jsonName());
        try writer.writeByte('"');
        if (self.default_value) |dv| {
            try writer.writeAll(",\"default_value\":");
            try writer.writeAll(dv);
        }
        if (self.value_type) |vt| {
            try writer.writeAll(",\"value_type\":");
            try writeJsonString(writer, vt);
        }
        if (self.schema) |s| {
            try writer.writeAll(",\"schema\":");
            try writeJsonString(writer, s);
        }
        if (self.step) |st| {
            try writer.writeAll(",\"step\":");
            var buf: [32]u8 = undefined;
            const str = std.fmt.bufPrint(&buf, "{d}", .{st}) catch "0";
            try writer.writeAll(str);
        }
        if (self.sensitive) |sn| {
            try writer.writeAll(",\"sensitive\":");
            try writer.writeAll(if (sn) "true" else "false");
        }
        if (self.enforce_schema) |es| {
            try writer.writeAll(",\"enforce_schema\":");
            try writer.writeAll(if (es) "true" else "false");
        }
        if (self.enforce_generic_value_type) |eg| {
            try writer.writeAll(",\"enforce_generic_value_type\":");
            try writer.writeAll(if (eg) "true" else "false");
        }
        try writer.writeByte('}');
    }
};

// ---------------------------------------------------------------------------
// NodeDefinition
// ---------------------------------------------------------------------------

pub const NodeDefinition = struct {
    name: []const u8 = "",
    friendly_name: []const u8 = "",
    description: []const u8 = "",
    category: []const u8 = "",
    icon: ?[]const u8 = null,
    pins: std.ArrayList(PinDefinition),
    scores: ?NodeScores = null,
    long_running: bool = false,
    docs: ?[]const u8 = null,
    permissions: std.ArrayList([]const u8),
    abi_ver: u32 = abi_version,

    pub fn init(alloc: std.mem.Allocator) NodeDefinition {
        return .{
            .pins = std.ArrayList(PinDefinition).init(alloc),
            .permissions = std.ArrayList([]const u8).init(alloc),
        };
    }

    pub fn addPin(self: *NodeDefinition, pin: PinDefinition) void {
        self.pins.append(pin) catch {};
    }

    pub fn addPermission(self: *NodeDefinition, perm: []const u8) void {
        self.permissions.append(perm) catch {};
    }

    pub fn setScores(self: *NodeDefinition, scores: NodeScores) void {
        self.scores = scores;
    }

    pub fn writeJson(self: *const NodeDefinition, writer: anytype) !void {
        try writer.writeAll("{\"name\":");
        try writeJsonString(writer, self.name);
        try writer.writeAll(",\"friendly_name\":");
        try writeJsonString(writer, self.friendly_name);
        try writer.writeAll(",\"description\":");
        try writeJsonString(writer, self.description);
        try writer.writeAll(",\"category\":");
        try writeJsonString(writer, self.category);
        try writer.writeAll(",\"pins\":[");
        for (self.pins.items, 0..) |*pin, i| {
            if (i > 0) try writer.writeByte(',');
            try pin.writeJson(writer);
        }
        try writer.writeAll("],\"long_running\":");
        try writer.writeAll(if (self.long_running) "true" else "false");
        try writer.writeAll(",\"abi_version\":");
        try writeInt(writer, self.abi_ver);
        if (self.icon) |ic| {
            try writer.writeAll(",\"icon\":");
            try writeJsonString(writer, ic);
        }
        if (self.scores) |*sc| {
            try writer.writeAll(",\"scores\":");
            try sc.writeJson(writer);
        }
        if (self.docs) |d| {
            try writer.writeAll(",\"docs\":");
            try writeJsonString(writer, d);
        }
        if (self.permissions.items.len > 0) {
            try writer.writeAll(",\"permissions\":[");
            for (self.permissions.items, 0..) |perm, i| {
                if (i > 0) try writer.writeByte(',');
                try writeJsonString(writer, perm);
            }
            try writer.writeByte(']');
        }
        try writer.writeByte('}');
    }

    pub fn toJson(self: *const NodeDefinition, alloc: std.mem.Allocator) ![]const u8 {
        var buf = std.ArrayList(u8).init(alloc);
        try self.writeJson(buf.writer());
        return buf.items;
    }
};

// ---------------------------------------------------------------------------
// ExecutionInput
// ---------------------------------------------------------------------------

pub const ExecutionInput = struct {
    inputs: std.StringHashMap([]const u8),
    node_id: []const u8 = "",
    node_name: []const u8 = "",
    run_id: []const u8 = "",
    app_id: []const u8 = "",
    board_id: []const u8 = "",
    user_id: []const u8 = "",
    stream_state: bool = false,
    log_level: u8 = log_level_info,

    pub fn init(alloc: std.mem.Allocator) ExecutionInput {
        return .{ .inputs = std.StringHashMap([]const u8).init(alloc) };
    }

    pub fn fromJson(alloc: std.mem.Allocator, s: []const u8) ExecutionInput {
        var input = ExecutionInput.init(alloc);
        var pos: usize = 0;

        skipWs(s, &pos);
        if (pos >= s.len or s[pos] != '{') return input;
        pos += 1;

        while (pos < s.len) {
            skipWs(s, &pos);
            if (pos >= s.len or s[pos] == '}') break;
            if (s[pos] == ',') {
                pos += 1;
                continue;
            }

            const key = readString(s, &pos);
            skipWs(s, &pos);
            if (pos < s.len and s[pos] == ':') pos += 1;
            skipWs(s, &pos);

            if (std.mem.eql(u8, key, "node_id")) {
                input.node_id = readString(s, &pos);
            } else if (std.mem.eql(u8, key, "node_name")) {
                input.node_name = readString(s, &pos);
            } else if (std.mem.eql(u8, key, "run_id")) {
                input.run_id = readString(s, &pos);
            } else if (std.mem.eql(u8, key, "app_id")) {
                input.app_id = readString(s, &pos);
            } else if (std.mem.eql(u8, key, "board_id")) {
                input.board_id = readString(s, &pos);
            } else if (std.mem.eql(u8, key, "user_id")) {
                input.user_id = readString(s, &pos);
            } else if (std.mem.eql(u8, key, "stream_state")) {
                const v = readValue(s, &pos);
                input.stream_state = std.mem.eql(u8, v, "true");
            } else if (std.mem.eql(u8, key, "log_level")) {
                const v = readValue(s, &pos);
                if (v.len == 1 and v[0] >= '0' and v[0] <= '9') {
                    input.log_level = v[0] - '0';
                }
            } else if (std.mem.eql(u8, key, "inputs")) {
                parseInputsMap(s, &pos, &input.inputs);
            } else {
                _ = readValue(s, &pos);
            }
        }

        return input;
    }
};

fn parseInputsMap(s: []const u8, pos: *usize, map: *std.StringHashMap([]const u8)) void {
    skipWs(s, pos);
    if (pos.* >= s.len or s[pos.*] != '{') {
        _ = readValue(s, pos);
        return;
    }
    pos.* += 1;

    while (pos.* < s.len) {
        skipWs(s, pos);
        if (pos.* >= s.len or s[pos.*] == '}') {
            pos.* += 1;
            break;
        }
        if (s[pos.*] == ',') {
            pos.* += 1;
            continue;
        }
        const k = readString(s, pos);
        skipWs(s, pos);
        if (pos.* < s.len and s[pos.*] == ':') pos.* += 1;
        const v = readValue(s, pos);
        map.put(k, v) catch {};
    }
}

// ---------------------------------------------------------------------------
// ExecutionResult
// ---------------------------------------------------------------------------

pub const ExecutionResult = struct {
    outputs: std.StringHashMap([]const u8),
    error_msg: ?[]const u8 = null,
    activate_exec: std.ArrayList([]const u8),
    pending: bool = false,

    pub fn init(alloc: std.mem.Allocator) ExecutionResult {
        return .{
            .outputs = std.StringHashMap([]const u8).init(alloc),
            .activate_exec = std.ArrayList([]const u8).init(alloc),
        };
    }

    pub fn success(alloc: std.mem.Allocator) ExecutionResult {
        return init(alloc);
    }

    pub fn fail(alloc: std.mem.Allocator, msg: []const u8) ExecutionResult {
        var r = init(alloc);
        r.error_msg = msg;
        return r;
    }

    pub fn setOutput(self: *ExecutionResult, name: []const u8, value: []const u8) void {
        self.outputs.put(name, value) catch {};
    }

    pub fn activateExecPin(self: *ExecutionResult, pin_name: []const u8) void {
        self.activate_exec.append(pin_name) catch {};
    }

    pub fn writeJson(self: *const ExecutionResult, writer: anytype) !void {
        try writer.writeAll("{\"outputs\":{");
        var first = true;
        var it = self.outputs.iterator();
        while (it.next()) |entry| {
            if (!first) try writer.writeByte(',');
            first = false;
            try writeJsonString(writer, entry.key_ptr.*);
            try writer.writeByte(':');
            try writer.writeAll(entry.value_ptr.*);
        }
        try writer.writeAll("},\"activate_exec\":[");
        for (self.activate_exec.items, 0..) |e, i| {
            if (i > 0) try writer.writeByte(',');
            try writeJsonString(writer, e);
        }
        try writer.writeAll("],\"pending\":");
        try writer.writeAll(if (self.pending) "true" else "false");
        if (self.error_msg) |err| {
            try writer.writeAll(",\"error\":");
            try writeJsonString(writer, err);
        }
        try writer.writeByte('}');
    }

    pub fn toJson(self: *const ExecutionResult, alloc: std.mem.Allocator) ![]const u8 {
        var buf = std.ArrayList(u8).init(alloc);
        try self.writeJson(buf.writer());
        return buf.items;
    }
};

// ---------------------------------------------------------------------------
// JSON helpers
// ---------------------------------------------------------------------------

pub fn writeJsonString(writer: anytype, s: []const u8) !void {
    try writer.writeByte('"');
    for (s) |c| {
        switch (c) {
            '"' => try writer.writeAll("\\\""),
            '\\' => try writer.writeAll("\\\\"),
            '\n' => try writer.writeAll("\\n"),
            '\r' => try writer.writeAll("\\r"),
            '\t' => try writer.writeAll("\\t"),
            else => try writer.writeByte(c),
        }
    }
    try writer.writeByte('"');
}

pub fn jsonStringAlloc(alloc: std.mem.Allocator, s: []const u8) ![]const u8 {
    var buf = std.ArrayList(u8).init(alloc);
    try writeJsonString(buf.writer(), s);
    return buf.items;
}

fn writeInt(writer: anytype, value: anytype) !void {
    var buf: [20]u8 = undefined;
    const str = std.fmt.bufPrint(&buf, "{d}", .{value}) catch return;
    try writer.writeAll(str);
}

fn skipWs(s: []const u8, pos: *usize) void {
    while (pos.* < s.len) {
        switch (s[pos.*]) {
            ' ', '\t', '\n', '\r' => pos.* += 1,
            else => return,
        }
    }
}

fn readString(s: []const u8, pos: *usize) []const u8 {
    if (pos.* >= s.len or s[pos.*] != '"') return "";
    pos.* += 1;
    const start = pos.*;
    while (pos.* < s.len and s[pos.*] != '"') {
        if (s[pos.*] == '\\') pos.* += 1;
        pos.* += 1;
    }
    const result = s[start..pos.*];
    if (pos.* < s.len) pos.* += 1;
    return result;
}

fn readValue(s: []const u8, pos: *usize) []const u8 {
    skipWs(s, pos);
    if (pos.* >= s.len) return "";
    const start = pos.*;
    switch (s[pos.*]) {
        '"' => {
            pos.* += 1;
            while (pos.* < s.len and s[pos.*] != '"') {
                if (s[pos.*] == '\\') pos.* += 1;
                pos.* += 1;
            }
            if (pos.* < s.len) pos.* += 1;
        },
        '{' => skipBraced(s, pos, '{', '}'),
        '[' => skipBraced(s, pos, '[', ']'),
        else => {
            while (pos.* < s.len) {
                switch (s[pos.*]) {
                    ',', '}', ']', ' ', '\t', '\n', '\r' => break,
                    else => pos.* += 1,
                }
            }
        },
    }
    return s[start..pos.*];
}

fn skipBraced(s: []const u8, pos: *usize, open: u8, close: u8) void {
    var depth: i32 = 0;
    while (pos.* < s.len) {
        const c = s[pos.*];
        if (c == open) {
            depth += 1;
        } else if (c == close) {
            depth -= 1;
            if (depth == 0) {
                pos.* += 1;
                return;
            }
        } else if (c == '"') {
            pos.* += 1;
            while (pos.* < s.len and s[pos.*] != '"') {
                if (s[pos.*] == '\\') pos.* += 1;
                pos.* += 1;
            }
        }
        pos.* += 1;
    }
}
