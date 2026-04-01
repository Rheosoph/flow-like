#pragma once
// Flow-Like WASM SDK – Lightweight JSON utilities (header-only, no external deps)

#include <cstdint>
#include <cstdlib>
#include <cstring>
#include <string>
#include <unordered_map>
#include <vector>

namespace flowlike {
namespace json {

// ============================================================================
// Escape / quote
// ============================================================================

inline std::string escape(const std::string& s) {
    std::string out;
    out.reserve(s.size() + 8);
    for (char c : s) {
        switch (c) {
            case '"':  out += "\\\""; break;
            case '\\': out += "\\\\"; break;
            case '\n': out += "\\n";  break;
            case '\r': out += "\\r";  break;
            case '\t': out += "\\t";  break;
            default:
                if (static_cast<unsigned char>(c) < 0x20) {
                    char buf[8];
                    snprintf(buf, sizeof(buf), "\\u%04x", static_cast<unsigned char>(c));
                    out += buf;
                } else {
                    out += c;
                }
        }
    }
    return out;
}

inline std::string quote(const std::string& s) {
    return "\"" + escape(s) + "\"";
}

// ============================================================================
// Builder – construct JSON strings incrementally
// ============================================================================

class Builder {
public:
    Builder& object_start() { buf_ += '{'; return *this; }
    Builder& object_end()   { trim_comma(); buf_ += '}'; return *this; }
    Builder& array_start()  { buf_ += '['; return *this; }
    Builder& array_end()    { trim_comma(); buf_ += ']'; return *this; }

    Builder& key(const std::string& k) {
        buf_ += quote(k) + ":";
        return *this;
    }

    Builder& value_string(const std::string& v) { buf_ += quote(v) + ","; return *this; }
    Builder& value_int(int64_t v)                { buf_ += std::to_string(v) + ","; return *this; }
    Builder& value_uint(uint64_t v)              { buf_ += std::to_string(v) + ","; return *this; }
    Builder& value_float(double v)               { buf_ += std::to_string(v) + ","; return *this; }
    Builder& value_bool(bool v)                  { buf_ += (v ? "true," : "false,"); return *this; }
    Builder& value_null()                        { buf_ += "null,"; return *this; }
    Builder& value_raw(const std::string& raw)   { buf_ += raw + ","; return *this; }

    Builder& kv_string(const std::string& k, const std::string& v) { return key(k).value_string(v); }
    Builder& kv_int(const std::string& k, int64_t v)               { return key(k).value_int(v); }
    Builder& kv_bool(const std::string& k, bool v)                 { return key(k).value_bool(v); }
    Builder& kv_raw(const std::string& k, const std::string& v)    { return key(k).value_raw(v); }

    std::string str() const { return buf_; }

private:
    void trim_comma() {
        if (!buf_.empty() && buf_.back() == ',') buf_.pop_back();
    }
    std::string buf_;
};

// ============================================================================
// Minimal parser helpers – extract typed values from a flat JSON object
// ============================================================================

inline bool is_ws(char c) { return c == ' ' || c == '\t' || c == '\n' || c == '\r'; }

inline std::string extract_string(const std::string& json, const std::string& key) {
    std::string needle = "\"" + key + "\"";
    auto pos = json.find(needle);
    if (pos == std::string::npos) return "";
    size_t i = pos + needle.size();
    while (i < json.size() && (is_ws(json[i]) || json[i] == ':')) ++i;
    if (i >= json.size() || json[i] != '"') return "";
    ++i;
    std::string result;
    while (i < json.size() && json[i] != '"') {
        if (json[i] == '\\' && i + 1 < json.size()) {
            ++i;
            switch (json[i]) {
                case '"':  result += '"';  break;
                case '\\': result += '\\'; break;
                case 'n':  result += '\n'; break;
                case 'r':  result += '\r'; break;
                case 't':  result += '\t'; break;
                default:   result += json[i]; break;
            }
        } else {
            result += json[i];
        }
        ++i;
    }
    return result;
}

inline bool extract_bool(const std::string& json, const std::string& key) {
    std::string needle = "\"" + key + "\"";
    auto pos = json.find(needle);
    if (pos == std::string::npos) return false;
    size_t i = pos + needle.size();
    while (i < json.size() && (is_ws(json[i]) || json[i] == ':')) ++i;
    return (i + 3 < json.size() && json.substr(i, 4) == "true");
}

inline int64_t extract_int(const std::string& json, const std::string& key) {
    std::string needle = "\"" + key + "\"";
    auto pos = json.find(needle);
    if (pos == std::string::npos) return 0;
    size_t i = pos + needle.size();
    while (i < json.size() && (is_ws(json[i]) || json[i] == ':')) ++i;
    bool neg = false;
    if (i < json.size() && json[i] == '-') { neg = true; ++i; }
    int64_t num = 0;
    while (i < json.size() && json[i] >= '0' && json[i] <= '9') {
        num = num * 10 + (json[i] - '0');
        ++i;
    }
    return neg ? -num : num;
}

// Parse the "inputs" sub-object from execution JSON into a map
inline void parse_inputs(const std::string& json, std::unordered_map<std::string, std::string>& out) {
    auto inputs_pos = json.find("\"inputs\"");
    if (inputs_pos == std::string::npos) return;
    size_t obj_start = json.find('{', inputs_pos + 8);
    if (obj_start == std::string::npos) return;

    int depth = 1;
    size_t obj_end = obj_start + 1;
    while (depth > 0 && obj_end < json.size()) {
        if (json[obj_end] == '{') ++depth;
        else if (json[obj_end] == '}') --depth;
        ++obj_end;
    }
    std::string sub = json.substr(obj_start, obj_end - obj_start);

    size_t i = 1;
    while (i < sub.size() - 1) {
        while (i < sub.size() && is_ws(sub[i])) ++i;
        if (i >= sub.size() - 1 || sub[i] == '}') break;
        if (sub[i] != '"') { ++i; continue; }

        size_t ks = ++i;
        while (i < sub.size() && sub[i] != '"') ++i;
        std::string k = sub.substr(ks, i - ks);
        ++i;

        while (i < sub.size() && (is_ws(sub[i]) || sub[i] == ':')) ++i;

        size_t vs = i;
        if (sub[i] == '"') {
            ++i;
            while (i < sub.size()) {
                if (sub[i] == '"' && sub[i - 1] != '\\') break;
                ++i;
            }
            ++i;
        } else if (sub[i] == '{') {
            int d = 1; ++i;
            while (d > 0 && i < sub.size()) { if (sub[i] == '{') ++d; else if (sub[i] == '}') --d; ++i; }
        } else if (sub[i] == '[') {
            int d = 1; ++i;
            while (d > 0 && i < sub.size()) { if (sub[i] == '[') ++d; else if (sub[i] == ']') --d; ++i; }
        } else {
            while (i < sub.size() && !is_ws(sub[i]) && sub[i] != ',' && sub[i] != '}') ++i;
        }
        out[k] = sub.substr(vs, i - vs);

        while (i < sub.size() && (is_ws(sub[i]) || sub[i] == ',')) ++i;
    }
}

}  // namespace json
}  // namespace flowlike
