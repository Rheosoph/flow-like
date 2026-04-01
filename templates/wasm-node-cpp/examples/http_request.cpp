/// HTTP Request Node — Demonstrates declaring HTTP permissions (C++)
///
/// This example shows how to declare the "http" permission and use the
/// WIT-generated host import to make outbound HTTP requests from a C++ WASM node.
/// Copy this pattern into your node.cpp when you need network access.
///
/// With the Component Model you also get native WASI sockets support for
/// direct TCP/UDP/DNS — but for simple HTTP, the host-provided request
/// function is the easiest route.

#include <cstdint>
#include <cstring>
#include <string>
#include <vector>

#include "flow_like_node.h"

// ── Helpers (same as in node.cpp — in a real project, share via a header) ───

static flow_like_node_string_t to_wit_string(const std::string& s) {
    flow_like_node_string_t ws;
    ws.ptr = reinterpret_cast<uint8_t*>(const_cast<char*>(s.data()));
    ws.len = s.size();
    return ws;
}

static std::string from_wit_string(const flow_like_node_string_t* ws) {
    if (!ws || !ws->ptr || ws->len == 0) return "";
    return std::string(reinterpret_cast<const char*>(ws->ptr), ws->len);
}

static std::string json_quote(const std::string& s) {
    std::string out;
    out.reserve(s.size() + 2);
    out += '"';
    for (char c : s) {
        switch (c) {
            case '"':  out += "\\\""; break;
            case '\\': out += "\\\\"; break;
            case '\n': out += "\\n";  break;
            case '\r': out += "\\r";  break;
            case '\t': out += "\\t";  break;
            default:   out += c;      break;
        }
    }
    out += '"';
    return out;
}

// ── HTTP GET via WIT import ─────────────────────────────────────────────
//
// The WIT http interface provides:
//   request(method: u8, url: string, headers: string, body: option<list<u8>>) -> option<string>
//
// method: 0 = GET, 1 = POST, 2 = PUT, 3 = DELETE, 4 = PATCH
//
// Example usage:

static std::string http_get(const std::string& url, const std::string& headers_json) {
    flow_like_node_string_t w_url     = to_wit_string(url);
    flow_like_node_string_t w_headers = to_wit_string(headers_json);

    // No body for GET
    flow_like_node_option_list_u8_t no_body;
    no_body.is_some = false;

    flow_like_node_option_string_t result;
    flow_like_node_http_request(0, &w_url, &w_headers, &no_body, &result);

    if (result.is_some) {
        return from_wit_string(&result.val);
    }
    return "";
}

/// Build the node definition for this example.
/// Note: add_permission("http") is required for the host to allow requests.
///
/// In your node.cpp, the build_http_get_definition() return value would be
/// serialised by exports_flow_like_node_get_node / get_nodes.
///
/// This file is not compiled as-is; it is reference code you copy into your
/// node.cpp.

// static NodeDefinition build_http_get_definition() {
//     NodeDefinition def;
//     def.name          = "http_get_request_cpp";
//     def.friendly_name = "HTTP GET Request (C++)";
//     def.description   = "Sends a GET request to a URL and reports the result";
//     def.category      = "Network/HTTP";
//     def.add_permission("http");
//
//     def.add_pin(PinDefinition::input("exec", "Execute", "Trigger", "Exec"));
//     def.add_pin(PinDefinition::input("url", "URL", "Target URL", "String")
//                     .with_default("\"https://httpbin.org/get\""));
//     def.add_pin(PinDefinition::input("headers_json", "Headers", "JSON headers", "String")
//                     .with_default("\"{}\""));
//     def.add_pin(PinDefinition::output("exec_out", "Done", "Fires after request", "Exec"));
//     def.add_pin(PinDefinition::output("response", "Response", "Response body", "String"));
//     return def;
// }
