// Flow-Like WASM SDK – Implementation

#include "flow_like_sdk.h"
#include "../src/json.h"

namespace flowlike {

// ============================================================================
// NodeScores
// ============================================================================

std::string NodeScores::to_json() const {
    return "{\"privacy\":" + std::to_string(privacy) +
           ",\"security\":" + std::to_string(security) +
           ",\"performance\":" + std::to_string(performance) +
           ",\"governance\":" + std::to_string(governance) +
           ",\"reliability\":" + std::to_string(reliability) +
           ",\"cost\":" + std::to_string(cost) + "}";
}

// ============================================================================
// PinDefinition
// ============================================================================

PinDefinition PinDefinition::input(const std::string& name,
                                   const std::string& friendly_name,
                                   const std::string& description,
                                   DataType data_type) {
    PinDefinition p;
    p.name          = name;
    p.friendly_name = friendly_name;
    p.description   = description;
    p.pin_type      = PinType::Input;
    p.data_type     = data_type;
    return p;
}

PinDefinition PinDefinition::output(const std::string& name,
                                    const std::string& friendly_name,
                                    const std::string& description,
                                    DataType data_type) {
    PinDefinition p;
    p.name          = name;
    p.friendly_name = friendly_name;
    p.description   = description;
    p.pin_type      = PinType::Output;
    p.data_type     = data_type;
    return p;
}

std::string PinDefinition::to_json() const {
    std::string j = "{\"name\":" + json::quote(name)
        + ",\"friendly_name\":" + json::quote(friendly_name)
        + ",\"description\":" + json::quote(description)
        + ",\"pin_type\":\"" + (pin_type == PinType::Input ? "Input" : "Output") + "\""
        + ",\"data_type\":\"" + data_type_str(data_type) + "\"";
    if (!default_value.empty()) j += ",\"default_value\":" + default_value;
    if (!value_type.empty())    j += ",\"value_type\":" + json::quote(value_type);
    if (!schema.empty())        j += ",\"schema\":" + json::quote(schema);
    if (has_step)                j += ",\"step\":" + std::to_string(step);
    if (has_sensitive)           j += ",\"sensitive\":" + std::string(sensitive ? "true" : "false");
    if (has_enforce_schema)      j += ",\"enforce_schema\":" + std::string(enforce_schema ? "true" : "false");
    if (has_enforce_generic_value_type) j += ",\"enforce_generic_value_type\":" + std::string(enforce_generic_value_type ? "true" : "false");
    j += "}";
    return j;
}

// ============================================================================
// NodeDefinition
// ============================================================================

std::string NodeDefinition::to_json() const {
    std::string pins_json = "[";
    for (size_t i = 0; i < pins.size(); ++i) {
        if (i > 0) pins_json += ",";
        pins_json += pins[i].to_json();
    }
    pins_json += "]";

    std::string j = "{\"name\":" + json::quote(name)
        + ",\"friendly_name\":" + json::quote(friendly_name)
        + ",\"description\":" + json::quote(description)
        + ",\"category\":" + json::quote(category)
        + ",\"pins\":" + pins_json
        + ",\"long_running\":" + (long_running ? "true" : "false")
        + ",\"abi_version\":" + std::to_string(abi_version);
    if (!icon.empty())  j += ",\"icon\":" + json::quote(icon);
    if (has_scores)     j += ",\"scores\":" + scores.to_json();
    if (!docs.empty())  j += ",\"docs\":" + json::quote(docs);
    if (!permissions.empty()) {
        j += ",\"permissions\":[";
        for (size_t i = 0; i < permissions.size(); ++i) {
            if (i > 0) j += ",";
            j += json::quote(permissions[i]);
        }
        j += "]";
    }
    j += "}";
    return j;
}

// ============================================================================
// ExecutionResult
// ============================================================================

std::string ExecutionResult::to_json() const {
    std::string out_json = "{";
    bool first = true;
    for (const auto& kv : outputs) {
        if (!first) out_json += ",";
        out_json += json::quote(kv.first) + ":" + kv.second;
        first = false;
    }
    out_json += "}";

    std::string exec_json = "[";
    for (size_t i = 0; i < activate_exec.size(); ++i) {
        if (i > 0) exec_json += ",";
        exec_json += json::quote(activate_exec[i]);
    }
    exec_json += "]";

    std::string j = "{\"outputs\":" + out_json
        + ",\"activate_exec\":" + exec_json
        + ",\"pending\":" + (pending ? "true" : "false");
    if (!error.empty()) j += ",\"error\":" + json::quote(error);
    j += "}";
    return j;
}

// ============================================================================
// Parse execution input
// ============================================================================

ExecutionInput parse_execution_input(const std::string& raw) {
    ExecutionInput inp;
    json::parse_inputs(raw, inp.inputs);
    inp.node_id      = json::extract_string(raw, "node_id");
    inp.node_name    = json::extract_string(raw, "node_name");
    inp.run_id       = json::extract_string(raw, "run_id");
    inp.app_id       = json::extract_string(raw, "app_id");
    inp.board_id     = json::extract_string(raw, "board_id");
    inp.user_id      = json::extract_string(raw, "user_id");
    inp.stream_state = json::extract_bool(raw, "stream_state");
    inp.log_level    = static_cast<uint8_t>(json::extract_int(raw, "log_level"));
    return inp;
}

// ============================================================================
// Serialization
// ============================================================================

int64_t serialize_definition(const NodeDefinition& def) {
    return pack_result(def.to_json());
}

int64_t serialize_result(const ExecutionResult& result) {
    return pack_result(result.to_json());
}

}  // namespace flowlike
