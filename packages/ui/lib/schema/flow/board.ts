export interface IBoard {
    comments:       { [key: string]: ICommentValue };
    created_at:     ITimestamp;
    description:    string;
    execution_mode: IExecutionMode;
    /**
     * Minimum document format understood by readers and writers. Missing versions mean 1.
     */
    format_version?: number;
    hash?:           number | null;
    id:              string;
    layers:          { [key: string]: ILayerValue };
    log_level:       ILogLevel;
    name:            string;
    nodes:           { [key: string]: INodeValue };
    page_ids:        string[];
    refs:            { [key: string]: string };
    stage:           IStage;
    updated_at:      ITimestamp;
    variables:       { [key: string]: IVariableValue };
    version:         any[];
    viewport:        any[];
    [property: string]: any;
}

export interface ICommentValue {
    author?:      null | string;
    color?:       null | string;
    comment_type: ICommentType;
    content:      string;
    coordinates:  any[];
    hash?:        number | null;
    height?:      number | null;
    id:           string;
    is_locked?:   boolean | null;
    layer?:       null | string;
    /**
     * Soft reference to a board node this comment is attached to (e.g. the statement a
     * FlowScript thread anchors on). Presentation metadata only — dangling references are
     * legal (the node may be deleted later); consumers must treat a missing node as unanchored.
     */
    node_id?:  null | string;
    timestamp: ITimestamp;
    width?:    number | null;
    z_index?:  number | null;
    [property: string]: any;
}

export enum ICommentType {
    Image = "Image",
    Text = "Text",
    Video = "Video",
}

export interface ITimestamp {
    nanos_since_epoch: number;
    secs_since_epoch:  number;
    [property: string]: any;
}

export enum IExecutionMode {
    Hybrid = "Hybrid",
    Local = "Local",
    Remote = "Remote",
}

export interface ILayerValue {
    cache?: null | ICacheObject;
    /**
     * Folder the layer is filed under in the sidebar, nested with forward slashes.
     * Empty means the top level. Purely organizational, it does not affect execution.
     */
    category?:        null | string;
    color?:           null | string;
    comment?:         null | string;
    comments:         { [key: string]: ICommentValue };
    coordinates:      any[];
    error?:           null | string;
    hash?:            number | null;
    id:               string;
    in_coordinates?:  any[] | null;
    name:             string;
    nodes:            { [key: string]: INodeValue };
    out_coordinates?: any[] | null;
    parent_id?:       null | string;
    pins:             { [key: string]: IPinValue };
    type:             string;
    variables:        { [key: string]: IVariableValue };
    [property: string]: any;
}

/**
 * Result caching for a layer invoked as a function.
 *
 * A hit replaces the whole call: the function body never runs, so its side effects do
 * not happen either. Only turn this on for layers whose outputs are a function of their
 * inputs.
 */
export interface ICacheObject {
    enabled?: boolean;
    /**
     * Namespace every entry for this layer is written under, so one layer's cache can be
     * invalidated without touching the rest of the app's.
     */
    prefix?: string;
    scope?:  string;
    /**
     * Lifetime of an entry in seconds. `None` or `0` keeps it until it is invalidated.
     */
    ttl_seconds?: number | null;
    [property: string]: any;
}

export interface INodeValue {
    /**
     * FlowScript member name inside `namespace` (`trim`, `fetch`, `createIssue`).
     * Presentation only — never part of node identity. `None` derives it from `name`.
     */
    alias?: null | string;
    /**
     * The editor generated this reroute and may replace it during automatic layout.
     */
    auto_reroute?: boolean | null;
    category:      string;
    /**
     * FlowScript names the text gave this node's values, keyed by output pin name; `""` is the
     * node's own binding (`const name = …`). Presentation only: identity stays `id`, and the
     * renderer falls back to a generated name when a stored one is invalid or already taken.
     */
    code_names?:     { [key: string]: string } | null;
    comment?:        null | string;
    coordinates?:    any[] | null;
    description:     string;
    docs?:           null | string;
    error?:          null | string;
    event_callback?: boolean | null;
    fn_refs?:        null | IFnRefsObject;
    friendly_name:   string;
    hash?:           number | null;
    icon?:           null | string;
    id:              string;
    layer?:          null | string;
    long_running?:   boolean | null;
    name:            string;
    /**
     * FlowScript namespace path (`string`, `http`, `jira`, `ui`; dotted when nested).
     * Presentation only — never part of node identity (`name` stays the id). `None` derives
     * it from `category` (`flow_like_ast::naming::derive_namespace`).
     */
    namespace?: null | string;
    /**
     * OAuth provider IDs this node requires (references Hub's oauth_providers config)
     */
    oauth_providers?: string[] | null;
    /**
     * If true, this node can only run locally (compute-intensive, RPA, browser automation)
     */
    only_offline?: boolean;
    pins:          { [key: string]: IPinValue };
    /**
     * The editor hides this node's unconnected data pins behind its latch.
     * Presentation only; execution never reads it.
     */
    pins_collapsed?: boolean | null;
    /**
     * Name of the data input pin that receives the value in FlowScript method form
     * (`s` in `s.trim()`). Presentation only. `None` applies the default rule
     * (`flowscript_receiver`); `Some("")` opts out: the node is callable statically only.
     */
    receiver?: null | string;
    /**
     * OAuth scopes required by this node (provider_id -> scopes)
     */
    required_oauth_scopes?: { [key: string]: string[] } | null;
    scores?:                null | IScoresObject;
    start?:                 boolean | null;
    /**
     * Schema version for node migration. When catalog version > placed version, pins are
     * synced.
     * None means unversioned (legacy). Bump this when changing pins in get_node().
     */
    version?: number | null;
    /**
     * WASM metadata for external nodes. None for built-in catalog nodes.
     * Populated automatically when placing or pasting nodes; never trust frontend-supplied
     * values.
     */
    wasm?: null | IWASMObject;
    [property: string]: any;
}

export interface IFnRefsObject {
    can_be_referenced_by_fns: boolean;
    can_reference_fns:        boolean;
    fn_refs:                  string[];
    [property: string]: any;
}

export interface IPinValue {
    connected_to:   string[];
    data_type:      IDataType;
    default_value?: number[] | null;
    depends_on:     string[];
    description:    string;
    friendly_name:  string;
    id:             string;
    index:          number;
    name:           string;
    options?:       null | IOptionsObject;
    pin_type:       IPinType;
    schema?:        null | string;
    value_type:     IValueType;
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

export interface IOptionsObject {
    enforce_generic_value_type?: boolean | null;
    enforce_schema?:             boolean | null;
    optional?:                   boolean | null;
    range?:                      any[] | null;
    sensitive?:                  boolean | null;
    step?:                       number | null;
    valid_values?:               string[] | null;
    [property: string]: any;
}

export enum IPinType {
    Input = "Input",
    Output = "Output",
}

export enum IValueType {
    Array = "Array",
    HashMap = "HashMap",
    HashSet = "HashSet",
    Normal = "Normal",
}

/**
 * Represents quality metrics for a node, with scores ranging from 0 to 10 (low - high).
 * Higher values indicate higher risk/impact in the given category. Use 0 for "none/low"
 * and 10 for "very high".
 *
 * # Score Categories
 * * `privacy` - Measures data protection and confidentiality (0 low - 10 high).
 * * `security` - Assesses resistance against potential attacks and exposure (0 low - 10
 * high).
 * * `performance` - Evaluates computational efficiency and speed. Higher means worse
 * performance.
 * * `governance` - Indicates compliance and auditability with policies and regulations.
 * * `reliability` - Measures stability, error rates and recoverability.
 * * `cost` - Represents resource/cost impact for running this node.
 */
export interface IScoresObject {
    cost:        number;
    governance:  number;
    performance: number;
    privacy:     number;
    reliability: number;
    security:    number;
    [property: string]: any;
}

export interface IWASMObject {
    package_id:   string;
    permissions?: string[];
    [property: string]: any;
}

export interface IVariableValue {
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

export enum ILogLevel {
    Debug = "Debug",
    Error = "Error",
    Fatal = "Fatal",
    Info = "Info",
    Warn = "Warn",
}

export enum IStage {
    Dev = "Dev",
    Int = "Int",
    PreProd = "PreProd",
    Prod = "Prod",
    QA = "QA",
}
