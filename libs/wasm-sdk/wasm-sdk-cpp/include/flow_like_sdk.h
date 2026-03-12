#pragma once
// Flow-Like WASM SDK for C/C++ (Emscripten)

#include <cstdint>
#include <cstdlib>
#include <cstring>
#include <string>
#include <unordered_map>
#include <vector>

namespace flowlike {

// ============================================================================
// ABI
// ============================================================================

static constexpr uint32_t ABI_VERSION = 1;

// ============================================================================
// Pin / data types
// ============================================================================

enum class PinType { Input, Output };

enum class DataType { Exec, String, I64, F64, Bool, Generic, Bytes, Date, PathBuf, Struct };

inline const char* data_type_str(DataType dt) {
    switch (dt) {
        case DataType::Exec:    return "Exec";
        case DataType::String:  return "String";
        case DataType::I64:     return "I64";
        case DataType::F64:     return "F64";
        case DataType::Bool:    return "Bool";
        case DataType::Generic: return "Generic";
        case DataType::Bytes:   return "Bytes";
        case DataType::Date:    return "Date";
        case DataType::PathBuf: return "PathBuf";
        case DataType::Struct:  return "Struct";
    }
    return "String";
}

enum class ValueTypeKind { Normal, Array, HashMap, HashSet };

inline const char* value_type_str(ValueTypeKind vt) {
    switch (vt) {
        case ValueTypeKind::Normal:  return "Normal";
        case ValueTypeKind::Array:   return "Array";
        case ValueTypeKind::HashMap: return "HashMap";
        case ValueTypeKind::HashSet: return "HashSet";
    }
    return "Normal";
}

// ============================================================================
// Node scores
// ============================================================================

struct NodeScores {
    uint8_t privacy     = 0;
    uint8_t security    = 0;
    uint8_t performance = 0;
    uint8_t governance  = 0;
    uint8_t reliability = 0;
    uint8_t cost        = 0;

    std::string to_json() const;
};

// ============================================================================
// Pin definition
// ============================================================================

struct PinDefinition {
    std::string name;
    std::string friendly_name;
    std::string description;
    PinType     pin_type   = PinType::Input;
    DataType    data_type  = DataType::String;
    std::string default_value;  // empty ⇒ absent
    std::string value_type;
    std::string schema;
    double      step        = 0.0;
    bool        has_step    = false;
    bool        sensitive   = false;
    bool        has_sensitive = false;
    bool        enforce_schema = false;
    bool        has_enforce_schema = false;
    bool        enforce_generic_value_type = false;
    bool        has_enforce_generic_value_type = false;

    static PinDefinition input(const std::string& name,
                               const std::string& friendly_name,
                               const std::string& description,
                               DataType data_type);

    static PinDefinition output(const std::string& name,
                                const std::string& friendly_name,
                                const std::string& description,
                                DataType data_type);

    PinDefinition& with_default(const std::string& v) { default_value = v; return *this; }

    /// Set the value type (e.g. "Array", "HashMap", "HashSet").
    PinDefinition& with_value_type(const std::string& v) { value_type = v; return *this; }

    /// Attach a raw JSON Schema string to this pin.
    PinDefinition& with_schema(const std::string& v) { schema = v; return *this; }

    PinDefinition& with_step(double v) { step = v; has_step = true; return *this; }
    PinDefinition& with_sensitive(bool v = true) { sensitive = v; has_sensitive = true; return *this; }
    PinDefinition& with_enforce_schema(bool v = true) { enforce_schema = v; has_enforce_schema = true; return *this; }
    PinDefinition& with_enforce_generic_value_type(bool v = true) { enforce_generic_value_type = v; has_enforce_generic_value_type = true; return *this; }

    std::string to_json() const;
};

// ============================================================================
// Node definition
// ============================================================================

struct NodeDefinition {
    std::string name;
    std::string friendly_name;
    std::string description;
    std::string category;
    std::string icon;           // empty → omitted
    std::string docs;           // empty → omitted
    bool        long_running = false;
    uint32_t    abi_version  = ABI_VERSION;
    std::vector<PinDefinition> pins;
    NodeScores  scores;
    bool        has_scores   = false;
    std::vector<std::string> permissions;

    NodeDefinition& add_pin(PinDefinition pin) { pins.push_back(std::move(pin)); return *this; }
    NodeDefinition& set_scores(NodeScores s)   { scores = s; has_scores = true; return *this; }
    NodeDefinition& add_permission(const std::string& p) { permissions.push_back(p); return *this; }

    std::string to_json() const;
};

// ============================================================================
// Execution input (parsed from JSON supplied by host)
// ============================================================================

struct ExecutionInput {
    std::unordered_map<std::string, std::string> inputs;
    std::string node_id;
    std::string node_name;
    std::string run_id;
    std::string app_id;
    std::string board_id;
    std::string user_id;
    bool        stream_state = false;
    uint8_t     log_level    = 1;
};

// ============================================================================
// Execution result (serialised back to host)
// ============================================================================

struct ExecutionResult {
    std::unordered_map<std::string, std::string> outputs;
    std::string              error;
    std::vector<std::string> activate_exec;
    bool                     pending = false;

    static ExecutionResult ok()                     { return {}; }
    static ExecutionResult fail(const std::string& msg) { ExecutionResult r; r.error = msg; return r; }

    ExecutionResult& set_output(const std::string& name, const std::string& json_value) {
        outputs[name] = json_value;
        return *this;
    }

    ExecutionResult& exec(const std::string& pin) {
        activate_exec.push_back(pin);
        return *this;
    }

    ExecutionResult& set_pending(bool p) { pending = p; return *this; }

    std::string to_json() const;
};

// ============================================================================
// Host function imports  (provided by Flow-Like runtime)
// ============================================================================

extern "C" {

// -- flowlike_log --
__attribute__((import_module("flowlike_log"), import_name("trace")))
void _fl_log_trace(const char* ptr, uint32_t len);
__attribute__((import_module("flowlike_log"), import_name("debug")))
void _fl_log_debug(const char* ptr, uint32_t len);
__attribute__((import_module("flowlike_log"), import_name("info")))
void _fl_log_info(const char* ptr, uint32_t len);
__attribute__((import_module("flowlike_log"), import_name("warn")))
void _fl_log_warn(const char* ptr, uint32_t len);
__attribute__((import_module("flowlike_log"), import_name("error")))
void _fl_log_error(const char* ptr, uint32_t len);
__attribute__((import_module("flowlike_log"), import_name("log_json")))
void _fl_log_json(int32_t level, const char* msg_ptr, uint32_t msg_len, const char* data_ptr, uint32_t data_len);

// -- flowlike_pins --
__attribute__((import_module("flowlike_pins"), import_name("get_input")))
int64_t _fl_get_input(const char* name_ptr, uint32_t name_len);
__attribute__((import_module("flowlike_pins"), import_name("set_output")))
void _fl_set_output(const char* name_ptr, uint32_t name_len, const char* val_ptr, uint32_t val_len);
__attribute__((import_module("flowlike_pins"), import_name("activate_exec")))
void _fl_activate_exec(const char* name_ptr, uint32_t name_len);

// -- flowlike_vars --
__attribute__((import_module("flowlike_vars"), import_name("get")))
int64_t _fl_var_get(const char* name_ptr, uint32_t name_len);
__attribute__((import_module("flowlike_vars"), import_name("set")))
void _fl_var_set(const char* name_ptr, uint32_t name_len, const char* val_ptr, uint32_t val_len);
__attribute__((import_module("flowlike_vars"), import_name("delete")))
void _fl_var_delete(const char* name_ptr, uint32_t name_len);
__attribute__((import_module("flowlike_vars"), import_name("has")))
int32_t _fl_var_has(const char* name_ptr, uint32_t name_len);

// -- flowlike_cache --
__attribute__((import_module("flowlike_cache"), import_name("get")))
int64_t _fl_cache_get(const char* key_ptr, uint32_t key_len);
__attribute__((import_module("flowlike_cache"), import_name("set")))
void _fl_cache_set(const char* key_ptr, uint32_t key_len, const char* val_ptr, uint32_t val_len);
__attribute__((import_module("flowlike_cache"), import_name("delete")))
void _fl_cache_delete(const char* key_ptr, uint32_t key_len);
__attribute__((import_module("flowlike_cache"), import_name("has")))
int32_t _fl_cache_has(const char* key_ptr, uint32_t key_len);

// -- flowlike_meta --
__attribute__((import_module("flowlike_meta"), import_name("get_node_id")))
int64_t _fl_get_node_id(void);
__attribute__((import_module("flowlike_meta"), import_name("get_run_id")))
int64_t _fl_get_run_id(void);
__attribute__((import_module("flowlike_meta"), import_name("get_app_id")))
int64_t _fl_get_app_id(void);
__attribute__((import_module("flowlike_meta"), import_name("get_board_id")))
int64_t _fl_get_board_id(void);
__attribute__((import_module("flowlike_meta"), import_name("get_user_id")))
int64_t _fl_get_user_id(void);
__attribute__((import_module("flowlike_meta"), import_name("is_streaming")))
int32_t _fl_is_streaming(void);
__attribute__((import_module("flowlike_meta"), import_name("get_log_level")))
int32_t _fl_get_log_level(void);
__attribute__((import_module("flowlike_meta"), import_name("time_now")))
int64_t _fl_time_now(void);
__attribute__((import_module("flowlike_meta"), import_name("random")))
int64_t _fl_random(void);

// -- flowlike_storage --
__attribute__((import_module("flowlike_storage"), import_name("read_request")))
int64_t _fl_storage_read(const char* path_ptr, uint32_t path_len);
__attribute__((import_module("flowlike_storage"), import_name("write_request")))
int32_t _fl_storage_write(const char* path_ptr, uint32_t path_len, const char* data_ptr, uint32_t data_len);
__attribute__((import_module("flowlike_storage"), import_name("storage_dir")))
int64_t _fl_storage_dir(int32_t node_scoped);
__attribute__((import_module("flowlike_storage"), import_name("upload_dir")))
int64_t _fl_upload_dir(void);
__attribute__((import_module("flowlike_storage"), import_name("cache_dir")))
int64_t _fl_cache_dir(int32_t node_scoped, int32_t user_scoped);
__attribute__((import_module("flowlike_storage"), import_name("user_dir")))
int64_t _fl_user_dir(int32_t node_scoped);
__attribute__((import_module("flowlike_storage"), import_name("list_request")))
int64_t _fl_storage_list(const char* path_ptr, uint32_t path_len);

// -- flowlike_models --
__attribute__((import_module("flowlike_models"), import_name("embed_text")))
int64_t _fl_embed_text(const char* bit_ptr, uint32_t bit_len, const char* texts_ptr, uint32_t texts_len);

// -- flowlike_http --
__attribute__((import_module("flowlike_http"), import_name("request")))
int32_t _fl_http_request(int32_t method, const char* url_ptr, uint32_t url_len,
                         const char* hdr_ptr, uint32_t hdr_len,
                         const char* body_ptr, uint32_t body_len);

// -- flowlike_stream --
__attribute__((import_module("flowlike_stream"), import_name("emit")))
void _fl_stream_emit(const char* event_ptr, uint32_t event_len, const char* data_ptr, uint32_t data_len);
__attribute__((import_module("flowlike_stream"), import_name("text")))
void _fl_stream_text(const char* text_ptr, uint32_t text_len);

// -- flowlike_auth --
__attribute__((import_module("flowlike_auth"), import_name("get_oauth_token")))
int64_t _fl_get_oauth_token(const char* provider_ptr, uint32_t provider_len);
__attribute__((import_module("flowlike_auth"), import_name("has_oauth_token")))
int32_t _fl_has_oauth_token(const char* provider_ptr, uint32_t provider_len);

}  // extern "C"

// ============================================================================
// Packed i64 helpers  (ptr << 32 | len)
// ============================================================================

inline int64_t pack_i64(uint32_t ptr, uint32_t len) {
    return (static_cast<int64_t>(ptr) << 32) | static_cast<int64_t>(len);
}

inline std::string unpack_string(int64_t packed) {
    if (packed == 0) return "";
    uint32_t ptr = static_cast<uint32_t>(packed >> 32);
    uint32_t len = static_cast<uint32_t>(packed & 0xFFFFFFFF);
    if (ptr == 0 || len == 0) return "";
    return std::string(reinterpret_cast<const char*>(ptr), len);
}

// ============================================================================
// Memory helpers (exported to host)
// ============================================================================

inline uint32_t flow_like_alloc(uint32_t size) {
    void* p = malloc(size);
    return static_cast<uint32_t>(reinterpret_cast<uintptr_t>(p));
}

inline void flow_like_dealloc(uint32_t ptr, uint32_t size) {
    (void)size;
    free(reinterpret_cast<void*>(static_cast<uintptr_t>(ptr)));
}

// Keep a global buffer alive so the host can read serialised data
inline std::string& result_buffer() {
    static std::string buf;
    return buf;
}

inline int64_t pack_result(const std::string& json) {
    result_buffer() = json;
    auto ptr = reinterpret_cast<uintptr_t>(result_buffer().data());
    auto len = static_cast<uint32_t>(result_buffer().size());
    return pack_i64(static_cast<uint32_t>(ptr), len);
}

// ============================================================================
// Logging helpers
// ============================================================================

namespace log {

inline void trace(const std::string& msg) { _fl_log_trace(msg.data(), msg.size()); }
inline void debug(const std::string& msg) { _fl_log_debug(msg.data(), msg.size()); }
inline void info(const std::string& msg)  { _fl_log_info(msg.data(), msg.size()); }
inline void warn(const std::string& msg)  { _fl_log_warn(msg.data(), msg.size()); }
inline void error(const std::string& msg) { _fl_log_error(msg.data(), msg.size()); }

}  // namespace log

// ============================================================================
// Pin I/O helpers
// ============================================================================

namespace pins {

inline std::string get_input(const std::string& name) {
    int64_t packed = _fl_get_input(name.data(), name.size());
    return unpack_string(packed);
}

inline void set_output(const std::string& name, const std::string& json_value) {
    _fl_set_output(name.data(), name.size(), json_value.data(), json_value.size());
}

inline void activate_exec(const std::string& name) {
    _fl_activate_exec(name.data(), name.size());
}

}  // namespace pins

// ============================================================================
// Variable helpers
// ============================================================================

namespace var {

inline std::string get(const std::string& name) {
    return unpack_string(_fl_var_get(name.data(), name.size()));
}

inline void set(const std::string& name, const std::string& value) {
    _fl_var_set(name.data(), name.size(), value.data(), value.size());
}

inline void del(const std::string& name) {
    _fl_var_delete(name.data(), name.size());
}

inline bool has(const std::string& name) {
    return _fl_var_has(name.data(), name.size()) != 0;
}

}  // namespace var

// ============================================================================
// Cache helpers
// ============================================================================

namespace cache {

inline std::string get(const std::string& key) {
    return unpack_string(_fl_cache_get(key.data(), key.size()));
}

inline void set(const std::string& key, const std::string& value) {
    _fl_cache_set(key.data(), key.size(), value.data(), value.size());
}

inline void del(const std::string& key) {
    _fl_cache_delete(key.data(), key.size());
}

inline bool has(const std::string& key) {
    return _fl_cache_has(key.data(), key.size()) != 0;
}

}  // namespace cache

// ============================================================================
// Streaming helpers
// ============================================================================

namespace stream {

inline void emit(const std::string& event_type, const std::string& data) {
    _fl_stream_emit(event_type.data(), event_type.size(), data.data(), data.size());
}

inline void text(const std::string& t) {
    _fl_stream_text(t.data(), t.size());
}

inline void progress(float pct, const std::string& message) {
    std::string data = "{\"progress\":" + std::to_string(pct) + ",\"message\":\"" + message + "\"}";
    emit("progress", data);
}

inline void json(const std::string& json_str) {
    emit("json", json_str);
}

}  // namespace stream

// ============================================================================
// Metadata helpers
// ============================================================================

namespace meta {

inline std::string node_id()  { return unpack_string(_fl_get_node_id()); }
inline std::string run_id()   { return unpack_string(_fl_get_run_id()); }
inline std::string app_id()   { return unpack_string(_fl_get_app_id()); }
inline std::string board_id() { return unpack_string(_fl_get_board_id()); }
inline std::string user_id()  { return unpack_string(_fl_get_user_id()); }
inline bool is_streaming()    { return _fl_is_streaming() != 0; }
inline int32_t log_level()    { return _fl_get_log_level(); }
inline int64_t time_now()     { return _fl_time_now(); }
inline int64_t random()       { return _fl_random(); }

}  // namespace meta

// ============================================================================
// Context – high-level wrapper around execution input
// ============================================================================

class Context {
public:
    explicit Context(const ExecutionInput& input)
        : input_(input), result_(ExecutionResult::ok()) {}

    // -- Metadata --
    const std::string& node_id()   const { return input_.node_id; }
    const std::string& node_name() const { return input_.node_name; }
    const std::string& run_id()    const { return input_.run_id; }
    const std::string& app_id()    const { return input_.app_id; }
    const std::string& board_id()  const { return input_.board_id; }
    const std::string& user_id()   const { return input_.user_id; }
    bool stream_enabled()          const { return input_.stream_state; }
    uint8_t get_log_level()        const { return input_.log_level; }

    // -- Input getters --
    std::string get_raw(const std::string& name) const {
        auto it = input_.inputs.find(name);
        return (it != input_.inputs.end()) ? it->second : "";
    }

    std::string get_string(const std::string& name, const std::string& def = "") const {
        auto it = input_.inputs.find(name);
        if (it == input_.inputs.end()) return def;
        const auto& v = it->second;
        if (v.size() >= 2 && v.front() == '"' && v.back() == '"')
            return v.substr(1, v.size() - 2);
        return v;
    }

    int64_t get_i64(const std::string& name, int64_t def = 0) const {
        auto it = input_.inputs.find(name);
        if (it == input_.inputs.end()) return def;
        return std::strtoll(it->second.c_str(), nullptr, 10);
    }

    double get_f64(const std::string& name, double def = 0.0) const {
        auto it = input_.inputs.find(name);
        if (it == input_.inputs.end()) return def;
        return std::strtod(it->second.c_str(), nullptr);
    }

    bool get_bool(const std::string& name, bool def = false) const {
        auto it = input_.inputs.find(name);
        if (it == input_.inputs.end()) return def;
        return it->second == "true";
    }

    // -- Output setters --
    void set_output(const std::string& name, const std::string& json_value) {
        result_.outputs[name] = json_value;
    }

    void activate_exec(const std::string& pin) {
        result_.activate_exec.push_back(pin);
    }

    void set_pending(bool p) { result_.pending = p; }
    void set_error(const std::string& e) { result_.error = e; }

    // -- Logging (level-gated) --
    void debug(const std::string& msg) const { if (input_.log_level <= 0) log::debug(msg); }
    void info(const std::string& msg)  const { if (input_.log_level <= 1) log::info(msg); }
    void warn(const std::string& msg)  const { if (input_.log_level <= 2) log::warn(msg); }
    void error(const std::string& msg) const { if (input_.log_level <= 3) log::error(msg); }

    // -- Streaming (only when enabled) --
    void stream_text(const std::string& t) const {
        if (input_.stream_state) stream::text(t);
    }
    void stream_json(const std::string& j) const {
        if (input_.stream_state) stream::json(j);
    }
    void stream_progress(float pct, const std::string& msg) const {
        if (input_.stream_state) stream::progress(pct, msg);
    }

    // -- Finalize --
    ExecutionResult finish() { return std::move(result_); }

    ExecutionResult success() {
        activate_exec("exec_out");
        return finish();
    }

    ExecutionResult fail(const std::string& msg) {
        set_error(msg);
        return finish();
    }

private:
    ExecutionInput  input_;
    ExecutionResult result_;
};

// ============================================================================
// Parse execution input from JSON
// ============================================================================

ExecutionInput parse_execution_input(const std::string& json);

// ============================================================================
// Serialization helpers
// ============================================================================

int64_t serialize_definition(const NodeDefinition& def);
int64_t serialize_result(const ExecutionResult& result);

// ============================================================================
// JSON helpers
// ============================================================================

inline std::string json_quote(const std::string& s) {
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
// Convenience macro for defining a node
// ============================================================================

#define FLOW_LIKE_NODE(node_name, friendly, desc, cat) \
    static flowlike::NodeDefinition _fl_make_node_def() { \
        flowlike::NodeDefinition def; \
        def.name          = (node_name); \
        def.friendly_name = (friendly);  \
        def.description   = (desc);      \
        def.category      = (cat);       \
        return def; \
    }

}  // namespace flowlike
