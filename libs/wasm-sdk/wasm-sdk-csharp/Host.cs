namespace FlowLike.Wasm.Sdk;

public abstract class HostBridge
{
    public virtual void Log(int level, string message) { }
    public virtual void Stream(string eventType, string data) { }
    public virtual void StreamText(string content) { }
    public virtual object? GetVariable(string name) => null;
    public virtual bool SetVariable(string name, object? value) => false;
    public virtual void DeleteVariable(string name) { }
    public virtual bool HasVariable(string name) => false;
    public virtual object? CacheGet(string key) => null;
    public virtual void CacheSet(string key, object? value) { }
    public virtual void CacheDelete(string key) { }
    public virtual bool CacheHas(string key) => false;
    public virtual long TimeNow() => 0;
    public virtual double Random() => 0;
    public virtual Dictionary<string, object?>? StorageDir(bool nodeScoped) => null;
    public virtual Dictionary<string, object?>? UploadDir() => null;
    public virtual Dictionary<string, object?>? CacheDir(bool nodeScoped, bool userScoped) => null;
    public virtual Dictionary<string, object?>? UserDir(bool nodeScoped) => null;
    public virtual byte[]? StorageRead(Dictionary<string, object?> flowPath) => null;
    public virtual bool StorageWrite(Dictionary<string, object?> flowPath, byte[] data) => false;
    public virtual List<Dictionary<string, object?>>? StorageList(Dictionary<string, object?> flowPath) => null;
    public virtual List<List<float>>? EmbedText(Dictionary<string, object?> bit, List<string> texts) => null;
    public virtual Dictionary<string, object?>? GetOAuthToken(string provider) => null;
    public virtual bool HasOAuthToken(string provider) => false;
    public virtual string? HttpRequest(byte method, string url, string headers, byte[]? body) => null;
}

public class MockHostBridge : HostBridge
{
    public List<(int Level, string Message)> Logs { get; } = [];
    public List<(string EventType, string Data)> Streams { get; } = [];
    public Dictionary<string, object?> Variables { get; } = [];
    public Dictionary<string, object?> CacheData { get; } = [];
    public Dictionary<string, byte[]> Storage { get; } = [];
    public Dictionary<string, Dictionary<string, object?>> OAuthTokens { get; } = [];

    public override void Log(int level, string message) => Logs.Add((level, message));
    public override void Stream(string eventType, string data) => Streams.Add((eventType, data));
    public override void StreamText(string content) => Streams.Add(("text", content));

    public override object? GetVariable(string name) => Variables.GetValueOrDefault(name);
    public override bool SetVariable(string name, object? value) { Variables[name] = value; return true; }
    public override void DeleteVariable(string name) => Variables.Remove(name);
    public override bool HasVariable(string name) => Variables.ContainsKey(name);

    public override object? CacheGet(string key) => CacheData.GetValueOrDefault(key);
    public override void CacheSet(string key, object? value) => CacheData[key] = value;
    public override void CacheDelete(string key) => CacheData.Remove(key);
    public override bool CacheHas(string key) => CacheData.ContainsKey(key);

    public override Dictionary<string, object?>? StorageDir(bool nodeScoped) =>
        new() { ["path"] = nodeScoped ? "storage/node" : "storage", ["store_ref"] = "mock_store", ["cache_store_ref"] = null };

    public override Dictionary<string, object?>? UploadDir() =>
        new() { ["path"] = "upload", ["store_ref"] = "mock_store", ["cache_store_ref"] = null };

    public override Dictionary<string, object?>? CacheDir(bool nodeScoped, bool userScoped) =>
        new() { ["path"] = "tmp/cache", ["store_ref"] = "mock_store", ["cache_store_ref"] = null };

    public override Dictionary<string, object?>? UserDir(bool nodeScoped) =>
        new() { ["path"] = "users/mock", ["store_ref"] = "mock_store", ["cache_store_ref"] = null };

    public override byte[]? StorageRead(Dictionary<string, object?> flowPath)
    {
        var path = flowPath.GetValueOrDefault("path")?.ToString() ?? "";
        return Storage.GetValueOrDefault(path);
    }

    public override bool StorageWrite(Dictionary<string, object?> flowPath, byte[] data)
    {
        var path = flowPath.GetValueOrDefault("path")?.ToString() ?? "";
        Storage[path] = data;
        return true;
    }

    public override List<Dictionary<string, object?>>? StorageList(Dictionary<string, object?> flowPath)
    {
        var prefix = flowPath.GetValueOrDefault("path")?.ToString() ?? "";
        var storeRef = flowPath.GetValueOrDefault("store_ref")?.ToString() ?? "";
        return Storage.Keys.Where(k => k.StartsWith(prefix))
            .Select(k => new Dictionary<string, object?> { ["path"] = k, ["store_ref"] = storeRef, ["cache_store_ref"] = null })
            .ToList();
    }

    public override List<List<float>>? EmbedText(Dictionary<string, object?> bit, List<string> texts) =>
        texts.Select(_ => new List<float> { 0.1f, 0.2f, 0.3f }).ToList();

    public override Dictionary<string, object?>? GetOAuthToken(string provider) =>
        OAuthTokens.GetValueOrDefault(provider);

    public override bool HasOAuthToken(string provider) => OAuthTokens.ContainsKey(provider);

    public override string? HttpRequest(byte method, string url, string headers, byte[]? body) =>
        Json.Serialize(new Dictionary<string, object> { ["status"] = 200, ["headers"] = new Dictionary<string, string>(), ["body"] = "{}" });
}

public static class Host
{
    private static HostBridge _current = new DefaultHostBridge();

    public static HostBridge Current
    {
        get => _current;
        set => _current = value;
    }

    private sealed class DefaultHostBridge : HostBridge { }
}
