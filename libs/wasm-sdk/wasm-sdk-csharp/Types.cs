namespace FlowLike.Wasm.Sdk;

public enum LogLevel
{
    Debug = 0,
    Info = 1,
    Warn = 2,
    Error = 3,
    Fatal = 4,
}

public static class PinType
{
    public const string Exec = "Exec";
    public const string String = "String";
    public const string I64 = "I64";
    public const string F64 = "F64";
    public const string Bool = "Bool";
    public const string Generic = "Generic";
    public const string Bytes = "Bytes";
    public const string Date = "Date";
    public const string PathBuf = "PathBuf";
    public const string Struct = "Struct";

    private static readonly HashSet<string> All = [Exec, String, I64, F64, Bool, Generic, Bytes, Date, PathBuf, Struct];

    public static string Validate(string dataType)
    {
        if (!All.Contains(dataType))
            throw new ArgumentException($"Invalid pin data type: {dataType}. Must be one of [{string.Join(", ", All)}]");
        return dataType;
    }
}

/// <summary>How the data is contained (scalar vs collection).</summary>
public static class ValueType
{
    public const string Normal = "Normal";
    public const string Array = "Array";
    public const string HashMap = "HashMap";
    public const string HashSet = "HashSet";

    private static readonly HashSet<string> All = [Normal, Array, HashMap, HashSet];

    public static string Validate(string valueType)
    {
        if (!All.Contains(valueType))
            throw new ArgumentException($"Invalid value type: {valueType}. Must be one of [{string.Join(", ", All)}]");
        return valueType;
    }
}

/// <summary>Alias: prefer DataType over PinType for clarity (in core, PinType means Input/Output).</summary>
public static class DataType
{
    public const string Exec = PinType.Exec;
    public const string String = PinType.String;
    public const string I64 = PinType.I64;
    public const string F64 = PinType.F64;
    public const string Bool = PinType.Bool;
    public const string Generic = PinType.Generic;
    public const string Bytes = PinType.Bytes;
    public const string Date = PinType.Date;
    public const string PathBuf = PinType.PathBuf;
    public const string Struct = PinType.Struct;

    public static string Validate(string dataType) => PinType.Validate(dataType);
}

public record NodeScores(
    int Privacy = 0,
    int Security = 0,
    int Performance = 0,
    int Governance = 0,
    int Reliability = 0,
    int Cost = 0
);

public class PinDefinition
{
    public string Name { get; set; } = "";
    public string FriendlyName { get; set; } = "";
    public string Description { get; set; } = "";
    public string PinKind { get; set; } = "";
    public string DataType { get; set; } = "";
    public object? DefaultValue { get; set; }
    public string? ValueType { get; set; }
    public string? Schema { get; set; }
    public List<string>? ValidValues { get; set; }
    public (double Min, double Max)? Range { get; set; }
    public double? Step { get; set; }
    public bool? Sensitive { get; set; }
    public bool? EnforceSchema { get; set; }
    public bool? EnforceGenericValueType { get; set; }

    public static PinDefinition InputExec(string name = "exec", string description = "")
    {
        return new PinDefinition
        {
            Name = name,
            FriendlyName = Humanize(name),
            Description = string.IsNullOrEmpty(description) ? $"Input: {name}" : description,
            PinKind = "Input",
            DataType = PinType.Exec,
        };
    }

    public static PinDefinition OutputExec(string name = "exec_out", string description = "")
    {
        return new PinDefinition
        {
            Name = name,
            FriendlyName = Humanize(name),
            Description = string.IsNullOrEmpty(description) ? $"Output: {name}" : description,
            PinKind = "Output",
            DataType = PinType.Exec,
        };
    }

    public static PinDefinition InputPin(string name, string dataType, string description = "", object? defaultValue = null, string? friendlyName = null)
    {
        PinType.Validate(dataType);
        return new PinDefinition
        {
            Name = name,
            FriendlyName = friendlyName ?? Humanize(name),
            Description = string.IsNullOrEmpty(description) ? $"Input: {name}" : description,
            PinKind = "Input",
            DataType = dataType,
            DefaultValue = defaultValue,
        };
    }

    public static PinDefinition OutputPin(string name, string dataType, string description = "", string? friendlyName = null)
    {
        PinType.Validate(dataType);
        return new PinDefinition
        {
            Name = name,
            FriendlyName = friendlyName ?? Humanize(name),
            Description = string.IsNullOrEmpty(description) ? $"Output: {name}" : description,
            PinKind = "Output",
            DataType = dataType,
        };
    }

    public PinDefinition WithDefault(object value) { DefaultValue = value; return this; }
    public PinDefinition WithValueType(string valueType) { FlowLike.Wasm.Sdk.ValueType.Validate(valueType); this.ValueType = valueType; return this; }
    public PinDefinition WithSchema(string schema) { Schema = schema; return this; }

    /// <summary>
    /// Derive a JSON Schema from a .NET type using System.Text.Json and attach
    /// it to this pin. Requires .NET 9+ (built-in JsonSchemaExporter).
    /// <example>
    /// <code>
    /// record Config(double Threshold, string Label);
    /// var pin = PinDefinition.InputPin("config", PinType.Struct)
    ///     .WithSchemaType&lt;Config&gt;();
    /// </code>
    /// </example>
    /// </summary>
    public PinDefinition WithSchemaType<T>()
    {
        var node = System.Text.Json.Schema.JsonSchemaExporter.GetJsonSchemaAsNode(
            Json.DefaultOptions, typeof(T), exporterOptions: null);
        Schema = node.ToJsonString();
        return this;
    }
    public PinDefinition WithValidValues(List<string> values) { ValidValues = values; return this; }
    public PinDefinition WithRange(double min, double max) { Range = (min, max); return this; }
    public PinDefinition WithStep(double step) { Step = step; return this; }
    public PinDefinition WithSensitive(bool sensitive = true) { Sensitive = sensitive; return this; }
    public PinDefinition WithEnforceSchema(bool enforce = true) { EnforceSchema = enforce; return this; }
    public PinDefinition WithEnforceGenericValueType(bool enforce = true) { EnforceGenericValueType = enforce; return this; }

    private static string Humanize(string name) =>
        string.Join(" ", name.Split('_').Where(w => w.Length > 0).Select(w => char.ToUpper(w[0]) + w[1..]));
}

public class NodeDefinition
{
    public string Name { get; set; } = "";
    public string FriendlyName { get; set; } = "";
    public string Description { get; set; } = "";
    public string Category { get; set; } = "";
    public string? Icon { get; set; }
    public List<PinDefinition> Pins { get; set; } = [];
    public NodeScores? Scores { get; set; }
    public bool? LongRunning { get; set; }
    public string? Docs { get; set; }
    public List<string> Permissions { get; set; } = [];
    public int AbiVersion { get; set; } = SdkConstants.AbiVersion;

    public NodeDefinition(string name, string friendlyName, string description, string category)
    {
        Name = name;
        FriendlyName = friendlyName;
        Description = description;
        Category = category;
    }

    public NodeDefinition AddPin(PinDefinition pin) { Pins.Add(pin); return this; }
    public NodeDefinition SetScores(NodeScores scores) { Scores = scores; return this; }
    public NodeDefinition SetLongRunning(bool longRunning) { LongRunning = longRunning; return this; }
    public NodeDefinition AddPermission(string permission) { Permissions.Add(permission); return this; }

    public string ToJson() => Json.Serialize(ToDictionary());

    public Dictionary<string, object?> ToDictionary()
    {
        var d = new Dictionary<string, object?>
        {
            ["name"] = Name,
            ["friendly_name"] = FriendlyName,
            ["description"] = Description,
            ["category"] = Category,
            ["pins"] = Pins.Select(PinToDictionary).ToList(),
            ["abi_version"] = AbiVersion,
        };
        if (Icon is not null) d["icon"] = Icon;
        if (Scores is not null) d["scores"] = ScoresToDictionary(Scores);
        if (LongRunning is not null) d["long_running"] = LongRunning;
        if (Docs is not null) d["docs"] = Docs;
        if (Permissions.Count > 0) d["permissions"] = Permissions;
        return d;
    }

    private static Dictionary<string, object?> PinToDictionary(PinDefinition p)
    {
        var d = new Dictionary<string, object?>
        {
            ["name"] = p.Name,
            ["friendly_name"] = p.FriendlyName,
            ["description"] = p.Description,
            ["pin_type"] = p.PinKind,
            ["data_type"] = p.DataType,
        };
        if (p.DefaultValue is not null) d["default_value"] = p.DefaultValue;
        if (p.ValueType is not null) d["value_type"] = p.ValueType;
        if (p.Schema is not null) d["schema"] = p.Schema;
        if (p.ValidValues is not null) d["valid_values"] = p.ValidValues;
        if (p.Range is not null) d["range"] = new[] { p.Range.Value.Min, p.Range.Value.Max };
        if (p.Step is not null) d["step"] = p.Step;
        if (p.Sensitive is not null) d["sensitive"] = p.Sensitive;
        if (p.EnforceSchema is not null) d["enforce_schema"] = p.EnforceSchema;
        if (p.EnforceGenericValueType is not null) d["enforce_generic_value_type"] = p.EnforceGenericValueType;
        return d;
    }

    private static Dictionary<string, int> ScoresToDictionary(NodeScores s) =>
        new()
        {
            ["privacy"] = s.Privacy,
            ["security"] = s.Security,
            ["performance"] = s.Performance,
            ["governance"] = s.Governance,
            ["reliability"] = s.Reliability,
            ["cost"] = s.Cost,
        };
}

public class ExecutionInput
{
    public Dictionary<string, object?> Inputs { get; set; } = [];
    public string NodeId { get; set; } = "";
    public string RunId { get; set; } = "";
    public string AppId { get; set; } = "";
    public string BoardId { get; set; } = "";
    public string UserId { get; set; } = "";
    public bool StreamState { get; set; }
    public int LogLevelValue { get; set; } = (int)LogLevel.Info;
    public string NodeName { get; set; } = "";

    public static ExecutionInput FromJson(string json)
    {
        var data = Json.Deserialize<Dictionary<string, object?>>(json) ?? [];
        return FromDictionary(data);
    }

    public static ExecutionInput FromDictionary(Dictionary<string, object?> data)
    {
        return new ExecutionInput
        {
            Inputs = Json.GetDictionary(data, "inputs"),
            NodeId = Json.GetString(data, "node_id"),
            RunId = Json.GetString(data, "run_id"),
            AppId = Json.GetString(data, "app_id"),
            BoardId = Json.GetString(data, "board_id"),
            UserId = Json.GetString(data, "user_id"),
            StreamState = Json.GetBool(data, "stream_state"),
            LogLevelValue = Json.GetInt(data, "log_level", (int)LogLevel.Info),
            NodeName = Json.GetString(data, "node_name"),
        };
    }
}

public class ExecutionResult
{
    public Dictionary<string, object?> Outputs { get; set; } = [];
    public string? Error { get; set; }
    public List<string> ActivateExec { get; set; } = [];
    public bool? Pending { get; set; }

    public static ExecutionResult Ok() => new();
    public static ExecutionResult Fail(string message) => new() { Error = message };

    public ExecutionResult SetOutput(string name, object? value) { Outputs[name] = value; return this; }
    public ExecutionResult Exec(string pinName) { ActivateExec.Add(pinName); return this; }
    public ExecutionResult SetPending(bool pending) { Pending = pending; return this; }

    public string ToJson() => Json.Serialize(ToDictionary());

    public Dictionary<string, object?> ToDictionary()
    {
        var d = new Dictionary<string, object?>
        {
            ["outputs"] = Outputs,
            ["activate_exec"] = ActivateExec,
        };
        if (Error is not null) d["error"] = Error;
        if (Pending is not null) d["pending"] = Pending;
        return d;
    }
}

public static class SdkConstants
{
    public const int AbiVersion = 1;
}
