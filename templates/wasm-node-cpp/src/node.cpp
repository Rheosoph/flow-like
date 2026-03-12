/**
 * Flow-Like WASM Node Template (C++) — Component Model
 *
 * Uses wasi-sdk + wit-bindgen-c to produce a WASM Component with full
 * WASI Preview 2 support (TCP/UDP/DNS via WASI sockets).
 *
 * Building:
 *   mise run setup    # install wasi-sdk, wasm-tools, wit-bindgen
 *   mise run build    # generate bindings, compile, create component
 *
 * The compiled component will be at: node.wasm
 */

#include <cstdint>
#include <cstring>
#include <string>
#include <unordered_map>
#include <vector>

// wit-bindgen-c generated header (run `mise run generate` first)
#include "flow_like_node.h"

// cabi_realloc is defined in the generated flow_like_node.c
extern "C" void* cabi_realloc(void* ptr, size_t old_size, size_t align, size_t new_size);

// ============================================================================
// ABI Version
// ============================================================================

static constexpr uint32_t ABI_VERSION = 1;

// ============================================================================
// String helpers for wit-bindgen-c types
// ============================================================================

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

static void set_wit_result(const std::string& src, flow_like_node_string_t* ret) {
    size_t len = src.size();
    uint8_t* buf = static_cast<uint8_t*>(cabi_realloc(nullptr, 0, 1, len));
    memcpy(buf, src.data(), len);
    ret->ptr = buf;
    ret->len = len;
}

// ============================================================================
// JSON helpers
// ============================================================================

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

// ============================================================================
// Pin / Node definition types
// ============================================================================

struct PinDefinition {
    std::string name;
    std::string friendly_name;
    std::string description;
    std::string pin_type;   // "Input" or "Output"
    std::string data_type;  // "Exec", "String", "I64", "F64", "Bool", etc.
    std::string default_value;

    static PinDefinition input(const char* n, const char* fn, const char* desc, const char* dt) {
        return {n, fn, desc, "Input", dt, ""};
    }
    static PinDefinition output(const char* n, const char* fn, const char* desc, const char* dt) {
        return {n, fn, desc, "Output", dt, ""};
    }
    PinDefinition& with_default(const std::string& v) { default_value = v; return *this; }

    std::string to_json() const {
        std::string j = "{\"name\":" + json_quote(name)
            + ",\"friendly_name\":" + json_quote(friendly_name)
            + ",\"description\":" + json_quote(description)
            + ",\"pin_type\":\"" + pin_type + "\""
            + ",\"data_type\":\"" + data_type + "\"";
        if (!default_value.empty()) j += ",\"default_value\":" + default_value;
        j += "}";
        return j;
    }
};

struct NodeDefinition {
    std::string name;
    std::string friendly_name;
    std::string description;
    std::string category;
    bool        long_running = false;
    uint32_t    abi_version  = ABI_VERSION;
    std::vector<PinDefinition> pins;
    std::vector<std::string> permissions;

    NodeDefinition& add_pin(PinDefinition pin) { pins.push_back(std::move(pin)); return *this; }
    NodeDefinition& add_permission(const std::string& p) { permissions.push_back(p); return *this; }

    std::string to_json() const {
        std::string pins_json = "[";
        for (size_t i = 0; i < pins.size(); ++i) {
            if (i > 0) pins_json += ",";
            pins_json += pins[i].to_json();
        }
        pins_json += "]";

        std::string j = "{\"name\":" + json_quote(name)
            + ",\"friendly_name\":" + json_quote(friendly_name)
            + ",\"description\":" + json_quote(description)
            + ",\"category\":" + json_quote(category)
            + ",\"pins\":" + pins_json
            + ",\"long_running\":" + (long_running ? "true" : "false")
            + ",\"abi_version\":" + std::to_string(abi_version);
        if (!permissions.empty()) {
            j += ",\"permissions\":[";
            for (size_t i = 0; i < permissions.size(); ++i) {
                if (i > 0) j += ",";
                j += json_quote(permissions[i]);
            }
            j += "]";
        }
        j += "}";
        return j;
    }
};

// ============================================================================
// Execution result
// ============================================================================

struct ExecutionResult {
    std::unordered_map<std::string, std::string> outputs;
    std::string              error;
    std::vector<std::string> activate_exec;
    bool                     pending = false;

    std::string to_json() const {
        std::string out_json = "{";
        bool first = true;
        for (const auto& kv : outputs) {
            if (!first) out_json += ",";
            out_json += json_quote(kv.first) + ":" + kv.second;
            first = false;
        }
        out_json += "}";

        std::string exec_json = "[";
        for (size_t i = 0; i < activate_exec.size(); ++i) {
            if (i > 0) exec_json += ",";
            exec_json += json_quote(activate_exec[i]);
        }
        exec_json += "]";

        std::string j = "{\"outputs\":" + out_json
            + ",\"activate_exec\":" + exec_json
            + ",\"pending\":" + (pending ? "true" : "false");
        if (!error.empty()) j += ",\"error\":" + json_quote(error);
        j += "}";
        return j;
    }
};

// ============================================================================
// Context — wraps WIT-generated import functions
// ============================================================================

class Context {
public:
    Context() : result_{} {}

    // -- Inputs (via WIT pins interface) --
    std::string get_input_raw(const std::string& name) const {
        flow_like_node_string_t wname = to_wit_string(name);
        flow_like_node_string_t ret;
        if (flow_like_node_pins_get_input(&wname, &ret)) {
            return from_wit_string(&ret);
        }
        return "";
    }

    std::string get_string(const std::string& name, const std::string& def = "") const {
        std::string v = get_input_raw(name);
        if (v.empty()) return def;
        if (v.size() >= 2 && v.front() == '"' && v.back() == '"')
            return v.substr(1, v.size() - 2);
        return v;
    }

    int64_t get_i64(const std::string& name, int64_t def = 0) const {
        std::string v = get_input_raw(name);
        if (v.empty()) return def;
        return std::strtoll(v.c_str(), nullptr, 10);
    }

    double get_f64(const std::string& name, double def = 0.0) const {
        std::string v = get_input_raw(name);
        if (v.empty()) return def;
        return std::strtod(v.c_str(), nullptr);
    }

    bool get_bool(const std::string& name, bool def = false) const {
        std::string v = get_input_raw(name);
        if (v.empty()) return def;
        return v == "true";
    }

    // -- Outputs --
    void set_output(const std::string& name, const std::string& json_value) {
        flow_like_node_string_t wname = to_wit_string(name);
        flow_like_node_string_t wval  = to_wit_string(json_value);
        flow_like_node_pins_set_output(&wname, &wval);
        result_.outputs[name] = json_value;
    }

    void activate_exec(const std::string& pin) {
        flow_like_node_string_t wpin = to_wit_string(pin);
        flow_like_node_pins_activate_exec(&wpin);
        result_.activate_exec.push_back(pin);
    }

    // -- Logging --
    void debug(const std::string& msg) const { wit_log(0, msg); }
    void info(const std::string& msg)  const { wit_log(1, msg); }
    void warn(const std::string& msg)  const { wit_log(2, msg); }
    void error(const std::string& msg) const { wit_log(3, msg); }

    // -- Streaming --
    void stream_text(const std::string& t) const {
        flow_like_node_string_t wt = to_wit_string(t);
        flow_like_node_streaming_text(&wt);
    }

    void stream_event(const std::string& event_type, const std::string& data) const {
        flow_like_node_string_t wev = to_wit_string(event_type);
        flow_like_node_string_t wdata = to_wit_string(data);
        flow_like_node_streaming_emit(&wev, &wdata);
    }

    // -- Finalize --
    ExecutionResult success() {
        activate_exec("exec_out");
        return std::move(result_);
    }

    ExecutionResult fail(const std::string& msg) {
        result_.error = msg;
        return std::move(result_);
    }

private:
    ExecutionResult result_;

    void wit_log(uint8_t level, const std::string& msg) const {
        flow_like_node_string_t wmsg = to_wit_string(msg);
        flow_like_node_logging_log(level, &wmsg);
    }
};

// ============================================================================
// Node Definition
// ============================================================================

static NodeDefinition build_definition() {
    NodeDefinition def;
    def.name          = "my_custom_node_cpp";
    def.friendly_name = "My Custom Node (C++)";
    def.description   = "A template WASM node built with C++ (Component Model)";
    def.category      = "Custom/WASM";
    def.add_permission("streaming");

    def.add_pin(PinDefinition::input("exec",        "Execute",    "Trigger execution",          "Exec"));
    def.add_pin(PinDefinition::input("input_text",  "Input Text", "Text to process",            "String").with_default("\"\""));
    def.add_pin(PinDefinition::input("multiplier",  "Multiplier", "Number of times to repeat",  "I64").with_default("1"));

    def.add_pin(PinDefinition::output("exec_out",    "Done",            "Execution complete",             "Exec"));
    def.add_pin(PinDefinition::output("output_text", "Output Text",     "Processed text",                 "String"));
    def.add_pin(PinDefinition::output("char_count",  "Character Count", "Number of characters in output", "I64"));

    return def;
}

// ============================================================================
// Node Execution
// ============================================================================

static ExecutionResult handle_run(Context& ctx) {
    std::string input_text = ctx.get_string("input_text");
    int64_t multiplier     = ctx.get_i64("multiplier", 1);
    if (multiplier < 0) multiplier = 0;

    ctx.debug("Processing: '" + input_text + "' x " + std::to_string(multiplier));

    std::string output;
    output.reserve(input_text.size() * static_cast<size_t>(multiplier));
    for (int64_t i = 0; i < multiplier; ++i) {
        output += input_text;
    }
    int64_t char_count = static_cast<int64_t>(output.size());

    ctx.stream_text("Generated " + std::to_string(char_count) + " characters");

    ctx.set_output("output_text", json_quote(output));
    ctx.set_output("char_count",  std::to_string(char_count));

    return ctx.success();
}

// ============================================================================
// WIT Component Exports (wit-bindgen-c naming convention)
// ============================================================================

extern "C" {

void exports_flow_like_node_get_node(flow_like_node_string_t* ret) {
    static NodeDefinition def = build_definition();
    std::string json = def.to_json();
    set_wit_result(json, ret);
}

void exports_flow_like_node_get_nodes(flow_like_node_string_t* ret) {
    static NodeDefinition def = build_definition();
    std::string json = "[" + def.to_json() + "]";
    set_wit_result(json, ret);
}

void exports_flow_like_node_run(flow_like_node_string_t* input, flow_like_node_string_t* ret) {
    (void)input;  // inputs are read via WIT pins interface
    Context ctx;
    ExecutionResult result = handle_run(ctx);
    std::string json = result.to_json();
    set_wit_result(json, ret);
}

uint32_t exports_flow_like_node_get_abi_version(void) {
    return ABI_VERSION;
}

}  // extern "C"
