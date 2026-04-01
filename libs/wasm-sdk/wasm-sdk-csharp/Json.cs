using System.Text.Json;
using System.Text.Json.Serialization;

namespace FlowLike.Wasm.Sdk;

public static class Json
{
    private static readonly JsonSerializerOptions Options = new()
    {
        PropertyNamingPolicy = JsonNamingPolicy.SnakeCaseLower,
        DefaultIgnoreCondition = JsonIgnoreCondition.WhenWritingNull,
        WriteIndented = false,
    };

    /// <summary>Default options used by the SDK (exposed for JsonSchemaExporter).</summary>
    public static JsonSerializerOptions DefaultOptions => Options;

    public static string Serialize(object? value) =>
        JsonSerializer.Serialize(value, Options);

    public static T? Deserialize<T>(string json) =>
        JsonSerializer.Deserialize<T>(json, Options);

    internal static string GetString(Dictionary<string, object?> data, string key, string defaultValue = "")
    {
        if (!data.TryGetValue(key, out var val) || val is null) return defaultValue;
        if (val is JsonElement je) return je.GetString() ?? defaultValue;
        return val.ToString() ?? defaultValue;
    }

    internal static int GetInt(Dictionary<string, object?> data, string key, int defaultValue = 0)
    {
        if (!data.TryGetValue(key, out var val) || val is null) return defaultValue;
        if (val is JsonElement je) return je.TryGetInt32(out var i) ? i : defaultValue;
        return Convert.ToInt32(val);
    }

    internal static bool GetBool(Dictionary<string, object?> data, string key, bool defaultValue = false)
    {
        if (!data.TryGetValue(key, out var val) || val is null) return defaultValue;
        if (val is JsonElement je && je.ValueKind is JsonValueKind.True or JsonValueKind.False) return je.GetBoolean();
        return Convert.ToBoolean(val);
    }

    internal static Dictionary<string, object?> GetDictionary(Dictionary<string, object?> data, string key)
    {
        if (!data.TryGetValue(key, out var val) || val is null) return [];
        if (val is JsonElement je && je.ValueKind == JsonValueKind.Object)
        {
            var result = new Dictionary<string, object?>();
            foreach (var prop in je.EnumerateObject())
                result[prop.Name] = ElementToObject(prop.Value);
            return result;
        }
        if (val is Dictionary<string, object?> dict) return dict;
        return [];
    }

    private static object? ElementToObject(JsonElement element) => element.ValueKind switch
    {
        JsonValueKind.String => element.GetString(),
        JsonValueKind.Number when element.TryGetInt64(out var l) => l,
        JsonValueKind.Number => element.GetDouble(),
        JsonValueKind.True => true,
        JsonValueKind.False => false,
        JsonValueKind.Null => null,
        _ => element.GetRawText(),
    };
}
