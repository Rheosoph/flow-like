/// HTTP Request Node — Demonstrates declaring HTTP permissions (Zig, Component Model)
///
/// This example shows how to declare the "http" permission and use the
/// WIT-generated host imports to make outbound HTTP requests from a Zig WASM node.
/// Copy this pattern into your main.zig when you need network access.

const std = @import("std");
const wit = @cImport({
    @cInclude("flow_like_node.h");
});

const allocator = std.heap.page_allocator;

// ============================================================================
// Helpers (same as main.zig)
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
    const buf: [*]u8 = @ptrCast(wit.cabi_realloc(
        null,
        0,
        1,
        src.len,
    ) orelse return);
    @memcpy(buf[0..src.len], src);
    ret.ptr = buf;
    ret.len = src.len;
}

fn buildPinJson(name: []const u8, friendly_name: []const u8, description: []const u8, pin_type: []const u8, data_type: []const u8, default_value: ?[]const u8) []const u8 {
    const default_part = if (default_value) |dv|
        std.fmt.allocPrint(allocator, ",\"default_value\":{s}", .{dv}) catch ""
    else
        "";
    return std.fmt.allocPrint(allocator,
        "{{\"name\":\"{s}\",\"friendly_name\":\"{s}\",\"description\":\"{s}\",\"pin_type\":\"{s}\",\"data_type\":\"{s}\"{s}}}",
        .{ name, friendly_name, description, pin_type, data_type, default_part },
    ) catch "{}";
}

// ============================================================================
// Node definition — note "http" permission
// ============================================================================

fn buildHttpGetDefinition() []const u8 {
    const pins = [_][]const u8{
        buildPinJson("exec", "Execute", "Trigger execution", "Input", "Exec", null),
        buildPinJson("url", "URL", "Target URL", "Input", "String", "\"\\\"https://httpbin.org/get\\\"\""),
        buildPinJson("headers_json", "Headers (JSON)", "Request headers as JSON", "Input", "String", "\"\\\"{}\\\"\""),
        buildPinJson("exec_out", "Done", "Fires after the request", "Output", "Exec", null),
        buildPinJson("success", "Success", "Whether the HTTP call was accepted", "Output", "Bool", null),
    };

    var pins_json = std.ArrayList(u8).init(allocator);
    pins_json.appendSlice("[") catch {};
    for (pins, 0..) |p, i| {
        if (i > 0) pins_json.appendSlice(",") catch {};
        pins_json.appendSlice(p) catch {};
    }
    pins_json.appendSlice("]") catch {};

    return std.fmt.allocPrint(allocator,
        "{{\"name\":\"http_get_request_zig\",\"friendly_name\":\"HTTP GET Request (Zig)\",\"description\":\"Sends a GET request to a URL and reports the result\",\"category\":\"Network/HTTP\",\"pins\":{s},\"long_running\":false,\"abi_version\":1,\"permissions\":[\"http\"]}}",
        .{pins_json.items},
    ) catch "{}";
}

// ============================================================================
// Run handler
// ============================================================================

fn handleHttpGet() []const u8 {
    // Read inputs via WIT pins interface
    var url_name = toWitString("url");
    var url_opt: wit.flow_like_node_option_string_t = undefined;
    wit.flow_like_node_pins_get_input(&url_name, &url_opt);
    const url = if (url_opt.is_some) fromWitString(&url_opt.val) else "https://httpbin.org/get";

    var headers_name = toWitString("headers_json");
    var headers_opt: wit.flow_like_node_option_string_t = undefined;
    wit.flow_like_node_pins_get_input(&headers_name, &headers_opt);
    const headers = if (headers_opt.is_some) fromWitString(&headers_opt.val) else "{}";

    var log_msg = toWitString("Sending GET request");
    wit.flow_like_node_logging_log(1, &log_msg);

    // Method 0 = GET
    var wurl = toWitString(url);
    var wheaders = toWitString(headers);
    const no_body = wit.flow_like_node_option_list_u8_t{ .is_some = false, .val = undefined };
    var result_opt: wit.flow_like_node_option_string_t = undefined;
    wit.flow_like_node_http_request(0, &wurl, &wheaders, &no_body, &result_opt);

    const ok = result_opt.is_some;
    const ok_str = if (ok) "true" else "false";

    var out_name = toWitString("success");
    var out_val = toWitString(ok_str);
    wit.flow_like_node_pins_set_output(&out_name, &out_val);

    var exec_pin = toWitString("exec_out");
    wit.flow_like_node_pins_activate_exec(&exec_pin);

    return std.fmt.allocPrint(allocator,
        "{{\"outputs\":{{\"success\":{s}}},\"activate_exec\":[\"exec_out\"],\"pending\":false}}",
        .{ok_str},
    ) catch "{}";
}

// ============================================================================
// WIT exports — wire up to the http_get node
// ============================================================================

// NOTE: This is an example — in a real multi-node package you would dispatch
// based on some identifier. For a single-node package, replace the exports
// in main.zig with the patterns shown here.
