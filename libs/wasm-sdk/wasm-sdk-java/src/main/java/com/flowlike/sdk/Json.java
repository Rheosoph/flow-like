package com.flowlike.sdk;

import java.util.HashMap;
import java.util.Map;

public final class Json {

    private Json() {}

    public static String escape(String s) {
        if (s == null) return "";
        StringBuilder b = new StringBuilder(s.length());
        for (int i = 0; i < s.length(); i++) {
            char c = s.charAt(i);
            switch (c) {
                case '"':  b.append("\\\""); break;
                case '\\': b.append("\\\\"); break;
                case '\n': b.append("\\n");  break;
                case '\r': b.append("\\r");  break;
                case '\t': b.append("\\t");  break;
                default:   b.append(c);
            }
        }
        return b.toString();
    }

    public static String quote(String s) {
        return "\"" + escape(s) + "\"";
    }

    public static Types.ExecutionInput parseExecutionInput(String s) {
        Types.ExecutionInput input = new Types.ExecutionInput();
        if (s == null || s.isEmpty()) return input;

        int[] pos = {0};

        skipWhitespace(s, pos);
        if (pos[0] >= s.length() || s.charAt(pos[0]) != '{') return input;
        pos[0]++;

        while (pos[0] < s.length()) {
            skipWhitespace(s, pos);
            if (pos[0] >= s.length() || s.charAt(pos[0]) == '}') break;
            if (s.charAt(pos[0]) == ',') { pos[0]++; continue; }

            String key = readString(s, pos);
            skipWhitespace(s, pos);
            if (pos[0] < s.length() && s.charAt(pos[0]) == ':') pos[0]++;
            skipWhitespace(s, pos);

            switch (key) {
                case "node_id":      input.nodeId = readString(s, pos); break;
                case "node_name":    input.nodeName = readString(s, pos); break;
                case "run_id":       input.runId = readString(s, pos); break;
                case "app_id":       input.appId = readString(s, pos); break;
                case "board_id":     input.boardId = readString(s, pos); break;
                case "user_id":      input.userId = readString(s, pos); break;
                case "stream_state": {
                    String v = readValue(s, pos);
                    input.streamState = "true".equals(v);
                    break;
                }
                case "log_level": {
                    String v = readValue(s, pos);
                    if (v.length() == 1 && v.charAt(0) >= '0' && v.charAt(0) <= '9') {
                        input.logLevel = v.charAt(0) - '0';
                    }
                    break;
                }
                case "inputs":
                    input.inputs.putAll(readMap(s, pos));
                    break;
                default:
                    readValue(s, pos);
            }
        }
        return input;
    }

    private static void skipWhitespace(String s, int[] pos) {
        while (pos[0] < s.length()) {
            char c = s.charAt(pos[0]);
            if (c != ' ' && c != '\t' && c != '\n' && c != '\r') break;
            pos[0]++;
        }
    }

    private static String readString(String s, int[] pos) {
        if (pos[0] >= s.length() || s.charAt(pos[0]) != '"') return "";
        pos[0]++;
        StringBuilder b = new StringBuilder();
        while (pos[0] < s.length() && s.charAt(pos[0]) != '"') {
            if (s.charAt(pos[0]) == '\\') {
                pos[0]++;
                if (pos[0] < s.length()) {
                    char esc = s.charAt(pos[0]);
                    switch (esc) {
                        case '"':  b.append('"'); break;
                        case '\\': b.append('\\'); break;
                        case 'n':  b.append('\n'); break;
                        case 'r':  b.append('\r'); break;
                        case 't':  b.append('\t'); break;
                        default:   b.append(esc);
                    }
                }
            } else {
                b.append(s.charAt(pos[0]));
            }
            pos[0]++;
        }
        if (pos[0] < s.length()) pos[0]++;
        return b.toString();
    }

    private static String readValue(String s, int[] pos) {
        skipWhitespace(s, pos);
        if (pos[0] >= s.length()) return "";
        char c = s.charAt(pos[0]);
        if (c == '"') {
            String v = readString(s, pos);
            return "\"" + v + "\"";
        }
        if (c == '{' || c == '[') {
            return readBracketed(s, pos, c == '{' ? '{' : '[', c == '{' ? '}' : ']');
        }
        int start = pos[0];
        while (pos[0] < s.length()) {
            char ch = s.charAt(pos[0]);
            if (ch == ',' || ch == '}' || ch == ']' || ch == ' ' || ch == '\t' || ch == '\n' || ch == '\r') break;
            pos[0]++;
        }
        return s.substring(start, pos[0]);
    }

    private static String readBracketed(String s, int[] pos, char open, char close) {
        int depth = 0;
        int start = pos[0];
        while (pos[0] < s.length()) {
            char c = s.charAt(pos[0]);
            if (c == open) {
                depth++;
            } else if (c == close) {
                depth--;
                if (depth == 0) {
                    pos[0]++;
                    return s.substring(start, pos[0]);
                }
            } else if (c == '"') {
                pos[0]++;
                while (pos[0] < s.length() && s.charAt(pos[0]) != '"') {
                    if (s.charAt(pos[0]) == '\\') pos[0]++;
                    pos[0]++;
                }
            }
            pos[0]++;
        }
        return s.substring(start, pos[0]);
    }

    private static Map<String, String> readMap(String s, int[] pos) {
        Map<String, String> map = new HashMap<>();
        skipWhitespace(s, pos);
        if (pos[0] >= s.length() || s.charAt(pos[0]) != '{') {
            readValue(s, pos);
            return map;
        }
        pos[0]++;
        while (pos[0] < s.length()) {
            skipWhitespace(s, pos);
            if (pos[0] >= s.length() || s.charAt(pos[0]) == '}') {
                pos[0]++;
                break;
            }
            if (s.charAt(pos[0]) == ',') { pos[0]++; continue; }
            String k = readString(s, pos);
            skipWhitespace(s, pos);
            if (pos[0] < s.length() && s.charAt(pos[0]) == ':') pos[0]++;
            String v = readValue(s, pos);
            map.put(k, v);
        }
        return map;
    }

    public static final class ObjectBuilder {
        private final StringBuilder b = new StringBuilder();
        private boolean hasField;

        public ObjectBuilder() {
            b.append('{');
        }

        public ObjectBuilder field(String key, String value) {
            separator();
            b.append(quote(key)).append(':').append(quote(value));
            return this;
        }

        public ObjectBuilder rawField(String key, String rawValue) {
            separator();
            b.append(quote(key)).append(':').append(rawValue);
            return this;
        }

        public ObjectBuilder intField(String key, int value) {
            separator();
            b.append(quote(key)).append(':').append(value);
            return this;
        }

        public ObjectBuilder longField(String key, long value) {
            separator();
            b.append(quote(key)).append(':').append(value);
            return this;
        }

        public ObjectBuilder boolField(String key, boolean value) {
            separator();
            b.append(quote(key)).append(':').append(value ? "true" : "false");
            return this;
        }

        public ObjectBuilder doubleField(String key, double value) {
            separator();
            b.append(quote(key)).append(':').append(value);
            return this;
        }

        private void separator() {
            if (hasField) b.append(',');
            hasField = true;
        }

        public String build() {
            b.append('}');
            return b.toString();
        }
    }
}
