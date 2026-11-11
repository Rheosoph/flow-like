export interface IRunPayload {
    /**
     * When true (default), secret variables from runtime_variables are ignored
     * unless they are also marked as runtime_configured.
     * Set to false only for trusted local (desktop) execution.
     */
    filter_secrets?: boolean | null;
    id:              string;
    payload?:        any;
    /**
     * Runtime-configured variables and secrets (for local execution).
     * These override board variable defaults when present.
     */
    runtime_variables?: { [key: string]: IRuntimeVariableValue } | null;
    [property: string]: any;
}

export interface IRuntimeVariableValue {
    category?:      null | string;
    data_type:      IDataType;
    default_value?: number[] | null;
    description?:   null | string;
    editable:       boolean;
    exposed:        boolean;
    hash?:          number | null;
    id:             string;
    name:           string;
    /**
     * If true, this variable is configured per-user at runtime (stored locally, not in flow)
     */
    runtime_configured?: boolean;
    schema?:             null | string;
    secret:              boolean;
    value_type:          IValueType;
    [property: string]: any;
}

export enum IDataType {
    Boolean = "Boolean",
    Byte = "Byte",
    Date = "Date",
    Execution = "Execution",
    Float = "Float",
    Generic = "Generic",
    Geometry = "Geometry",
    Integer = "Integer",
    PathBuf = "PathBuf",
    String = "String",
    Struct = "Struct",
}

export enum IValueType {
    Array = "Array",
    HashMap = "HashMap",
    HashSet = "HashSet",
    Normal = "Normal",
}
