namespace FlowLike.Wasm.Sdk;

public class Context
{
    private readonly ExecutionInput _input;
    private readonly ExecutionResult _result;
    private readonly HostBridge _host;

    public Context(ExecutionInput input, HostBridge? host = null)
    {
        _input = input;
        _result = ExecutionResult.Ok();
        _host = host ?? Host.Current;
    }

    public static Context FromJson(string json, HostBridge? host = null) =>
        new(ExecutionInput.FromJson(json), host);

    public string NodeId => _input.NodeId;
    public string NodeName => _input.NodeName;
    public string RunId => _input.RunId;
    public string AppId => _input.AppId;
    public string BoardId => _input.BoardId;
    public string UserId => _input.UserId;
    public bool StreamEnabled => _input.StreamState;
    public int LogLevelValue => _input.LogLevelValue;

    public object? GetInput(string name) =>
        _input.Inputs.TryGetValue(name, out var val) ? val : null;

    public string? GetString(string name, string? defaultValue = null)
    {
        var val = GetInput(name);
        return val is not null ? val.ToString() : defaultValue;
    }

    public long? GetI64(string name, long? defaultValue = null)
    {
        var val = GetInput(name);
        if (val is null) return defaultValue;
        return Convert.ToInt64(val);
    }

    public double? GetF64(string name, double? defaultValue = null)
    {
        var val = GetInput(name);
        if (val is null) return defaultValue;
        return Convert.ToDouble(val);
    }

    public bool? GetBool(string name, bool? defaultValue = null)
    {
        var val = GetInput(name);
        if (val is null) return defaultValue;
        return Convert.ToBoolean(val);
    }

    public object RequireInput(string name) =>
        GetInput(name) ?? throw new InvalidOperationException($"Required input '{name}' not provided");

    public void SetOutput(string name, object? value) => _result.SetOutput(name, value);
    public void ActivateExec(string pinName) => _result.Exec(pinName);
    public void SetPending(bool pending) => _result.SetPending(pending);

    public void Debug(string message) { if (_input.LogLevelValue <= (int)LogLevel.Debug) _host.Log((int)LogLevel.Debug, message); }
    public void Info(string message) { if (_input.LogLevelValue <= (int)LogLevel.Info) _host.Log((int)LogLevel.Info, message); }
    public void Warn(string message) { if (_input.LogLevelValue <= (int)LogLevel.Warn) _host.Log((int)LogLevel.Warn, message); }
    public void Error(string message) { if (_input.LogLevelValue <= (int)LogLevel.Error) _host.Log((int)LogLevel.Error, message); }

    public void StreamText(string text)
    {
        if (_input.StreamState) _host.Stream("text", text);
    }

    public void StreamJson(object data)
    {
        if (_input.StreamState) _host.Stream("json", Json.Serialize(data));
    }

    public void StreamProgress(double progress, string message)
    {
        if (_input.StreamState)
        {
            var payload = Json.Serialize(new Dictionary<string, object> { ["progress"] = progress, ["message"] = message });
            _host.Stream("progress", payload);
        }
    }

    public object? GetVariable(string name) => _host.GetVariable(name);
    public bool SetVariable(string name, object? value) => _host.SetVariable(name, value);
    public void DeleteVariable(string name) => _host.DeleteVariable(name);
    public bool HasVariable(string name) => _host.HasVariable(name);

    public object? CacheGet(string key) => _host.CacheGet(key);
    public void CacheSet(string key, object? value) => _host.CacheSet(key, value);
    public void CacheDelete(string key) => _host.CacheDelete(key);
    public bool CacheHas(string key) => _host.CacheHas(key);

    public Dictionary<string, object?>? StorageDir(bool nodeScoped = false) => _host.StorageDir(nodeScoped);
    public Dictionary<string, object?>? UploadDir() => _host.UploadDir();
    public Dictionary<string, object?>? CacheDir(bool nodeScoped = false, bool userScoped = false) => _host.CacheDir(nodeScoped, userScoped);
    public Dictionary<string, object?>? UserDir(bool nodeScoped = false) => _host.UserDir(nodeScoped);

    public byte[]? StorageRead(Dictionary<string, object?> flowPath) => _host.StorageRead(flowPath);
    public bool StorageWrite(Dictionary<string, object?> flowPath, byte[] data) => _host.StorageWrite(flowPath, data);
    public List<Dictionary<string, object?>>? StorageList(Dictionary<string, object?> flowPath) => _host.StorageList(flowPath);

    public Dictionary<string, object?>? HttpRequest(byte method, string url, Dictionary<string, string>? headers = null, byte[]? body = null)
    {
        var result = _host.HttpRequest(method, url, Json.Serialize(headers ?? new Dictionary<string, string>()), body);
        if (result == null) return null;
        return Json.Deserialize<Dictionary<string, object?>>(result);
    }

    public Dictionary<string, object?>? HttpGet(string url, Dictionary<string, string>? headers = null) =>
        HttpRequest(0, url, headers);

    public Dictionary<string, object?>? HttpPost(string url, byte[]? body = null, Dictionary<string, string>? headers = null) =>
        HttpRequest(1, url, headers, body);

    public ExecutionResult Success()
    {
        _result.Exec("exec_out");
        return _result;
    }

    public ExecutionResult Fail(string error)
    {
        _result.Error = error;
        return _result;
    }

    public ExecutionResult Finish() => _result;
}
