package com.flowlike.sdk;

import java.util.HashMap;
import java.util.Map;

public final class Context {

    private final Types.ExecutionInput input;
    private final Types.ExecutionResult result;
    private final Map<String, String> outputs;

    public Context(Types.ExecutionInput input) {
        this.input = input;
        this.result = Types.successResult();
        this.outputs = new HashMap<>();
    }

    // --- Metadata ---

    public String nodeId() { return input.nodeId; }
    public String nodeName() { return input.nodeName; }
    public String runId() { return input.runId; }
    public String appId() { return input.appId; }
    public String boardId() { return input.boardId; }
    public String userId() { return input.userId; }
    public boolean streamEnabled() { return input.streamState; }
    public int logLevelValue() { return input.logLevel; }

    // --- Input getters ---

    public String getRawInput(String name) {
        return input.inputs.get(name);
    }

    public boolean hasInput(String name) {
        return input.inputs.containsKey(name);
    }

    public String getString(String name, String defaultValue) {
        String v = input.inputs.get(name);
        if (v == null) return defaultValue;
        if (v.length() >= 2 && v.charAt(0) == '"' && v.charAt(v.length() - 1) == '"') {
            return v.substring(1, v.length() - 1);
        }
        return v;
    }

    public String getString(String name) {
        return getString(name, "");
    }

    public long getI64(String name, long defaultValue) {
        String v = input.inputs.get(name);
        if (v == null) return defaultValue;
        try {
            return Long.parseLong(v);
        } catch (NumberFormatException e) {
            return defaultValue;
        }
    }

    public double getF64(String name, double defaultValue) {
        String v = input.inputs.get(name);
        if (v == null) return defaultValue;
        try {
            return Double.parseDouble(v);
        } catch (NumberFormatException e) {
            return defaultValue;
        }
    }

    public boolean getBool(String name, boolean defaultValue) {
        String v = input.inputs.get(name);
        if (v == null) return defaultValue;
        return "true".equals(v);
    }

    // --- Output setters ---

    public void setOutput(String name, String value) {
        outputs.put(name, value);
    }

    public void activateExec(String pinName) {
        result.activateExec.add(pinName);
    }

    public void setPending(boolean pending) {
        result.pending = pending;
    }

    public void setError(String error) {
        result.error = error;
    }

    // --- Level-gated logging ---

    private boolean shouldLog(int level) {
        return level >= input.logLevel;
    }

    public void debug(String msg) {
        if (shouldLog(Types.LOG_LEVEL_DEBUG)) Host.logDebug(msg);
    }

    public void info(String msg) {
        if (shouldLog(Types.LOG_LEVEL_INFO)) Host.logInfo(msg);
    }

    public void warn(String msg) {
        if (shouldLog(Types.LOG_LEVEL_WARN)) Host.logWarn(msg);
    }

    public void error(String msg) {
        if (shouldLog(Types.LOG_LEVEL_ERROR)) Host.logError(msg);
    }

    // --- Conditional streaming ---

    public void streamText(String text) {
        if (streamEnabled()) Host.streamText(text);
    }

    public void streamJson(String data) {
        if (streamEnabled()) Host.streamEmit("json", data);
    }

    public void streamProgress(float progress, String message) {
        if (!streamEnabled()) return;
        String payload = "{\"progress\":" + progress + ",\"message\":\"" + Json.escape(message) + "\"}";
        Host.streamEmit("progress", payload);
    }

    // --- Variables ---

    public String getVariable(String name) {
        return Host.getVariable(name);
    }

    public void setVariable(String name, String value) {
        Host.setVariable(name, value);
    }

    public void deleteVariable(String name)          { Host.deleteVariable(name); }
    public boolean hasVariable(String name)          { return Host.hasVariable(name); }

    // --- Cache ---

    public String cacheGet(String key)               { return Host.cacheGet(key); }
    public void cacheSet(String key, String value)   { Host.cacheSet(key, value); }
    public void cacheDelete(String key)              { Host.cacheDelete(key); }
    public boolean cacheHas(String key)              { return Host.cacheHas(key); }

    // --- Dirs ---

    public String storageDir(boolean nodeScoped)                       { return Host.storageDir(nodeScoped); }
    public String uploadDir()                                          { return Host.uploadDir(); }
    public String cacheDir(boolean nodeScoped, boolean userScoped)     { return Host.cacheDir(nodeScoped, userScoped); }
    public String userDir(boolean nodeScoped)                          { return Host.userDir(nodeScoped); }

    // --- Storage I/O ---

    public String storageRead(String path)                             { return Host.storageRead(path); }
    public boolean storageWrite(String path, String data)              { return Host.storageWrite(path, data); }
    public String storageList(String flowPathJson)                     { return Host.storageList(flowPathJson); }

    // --- Embeddings ---

    public String embedText(String bitJson, String textsJson)          { return Host.embedText(bitJson, textsJson); }

    // --- HTTP ---

    public boolean httpRequest(int method, String url, String headers, String body) {
        return Host.httpRequest(method, url, headers, body);
    }

    // --- Auth ---

    public String getOAuthToken(String provider)     { return Host.getOAuthToken(provider); }
    public boolean hasOAuthToken(String provider)    { return Host.hasOAuthToken(provider); }

    // --- Time / Random ---

    public long timeNow()  { return Host.timeNow(); }
    public long random()   { return Host.random(); }

    // --- Finalize ---

    public Types.ExecutionResult finish() {
        for (Map.Entry<String, String> entry : outputs.entrySet()) {
            result.outputs.put(entry.getKey(), entry.getValue());
        }
        return result;
    }

    public Types.ExecutionResult success() {
        activateExec("exec_out");
        return finish();
    }

    public Types.ExecutionResult fail(String error) {
        setError(error);
        return finish();
    }
}
