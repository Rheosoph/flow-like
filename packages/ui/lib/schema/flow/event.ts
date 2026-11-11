export interface IEvent {
    active:         boolean;
    board_id:       string;
    board_version?: any[] | null;
    canary?:        null | ICanaryObject;
    config:         number[];
    /**
     * Process-mining case-key mappings: business key name → dot-path into
     * the invocation payload (e.g. `order_id` → `order.id`). Extracted on
     * every run so cases group by business object automatically.
     */
    correlation_mappings?: { [key: string]: string } | null;
    created_at:            ICreatedAt;
    default_page_id?:      null | string;
    description:           string;
    event_type:            string;
    event_version:         any[];
    /**
     * Where this event runs (Local/offline vs Remote/server). Inherited from
     * the board's `execution_mode` when that mode is not Hybrid.
     */
    execution_mode?: string;
    /**
     * Whether the event is publicly reachable or only callable by connected
     * apps via the app-connection proxy. Only meaningful for REST/MCP events.
     */
    exposure?: string;
    id:        string;
    /**
     * Input pins copied from the node (populated at upsert time)
     */
    inputs?: IInputElement[];
    /**
     * Whether this is the default event/route for the app (shown at "/")
     */
    is_default?: boolean;
    name:        string;
    node_id:     string;
    notes?:      INotesClass | null;
    priority:    number;
    /**
     * URL route path that maps to this event (e.g., "/", "/dashboard")
     */
    route?:     null | string;
    updated_at: ICreatedAt;
    variables:  { [key: string]: IVariableValue };
    /**
     * Canary/shadow targets. Read through [`Event::variant_set`]: when this
     * is empty a legacy `canary` still counts as a single ILive variant.
     */
    variants?: IVariantElement[];
    [property: string]: any;
}

export interface ICanaryObject {
    board_id:       string;
    board_version?: any[] | null;
    created_at:     ICreatedAt;
    node_id:        string;
    updated_at:     ICreatedAt;
    variables:      { [key: string]: IVariableValue };
    weight:         number;
    [property: string]: any;
}

export interface ICreatedAt {
    nanos_since_epoch: number;
    secs_since_epoch:  number;
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

/**
 * Simplified input pin metadata for events (used when board can't be fetched)
 */
export interface IInputElement {
    data_type: string;
    /**
     * A sensitive pin's default exists but is withheld; an optional input left out still runs
     * with it.
     */
    default_omitted?: boolean;
    default_value?:   number[] | null;
    description:      string;
    friendly_name:    string;
    id:               string;
    index:            number;
    /**
     * The [`EVENT_INPUTS_FORMAT`] this input was built with; 0 for inputs stored before it.
     */
    inputs_format?: number;
    name:           string;
    optional?:      boolean;
    range?:         any[] | null;
    schema?:        null | string;
    /**
     * The pin is marked sensitive: its default is never copied here.
     */
    sensitive?:    boolean;
    step?:         number | null;
    valid_values?: string[] | null;
    value_type:    string;
    [property: string]: any;
}

export interface INotesClass {
    NOTES?: string;
    URL?:   string;
}

/**
 * An alternate dispatch target for an event: a canary (`Live`) or shadow
 * deployment of another board/version/node. Read through
 * [`Event::variant_set`], which falls back to the legacy `canary` field.
 */
export interface IVariantElement {
    board_id: string;
    /**
     * `None` floats on latest (the etag is bound at dispatch); the `u32::MAX`
     * sentinel is reserved and refused at upsert.
     */
    board_version?: any[] | null;
    created_at:     ICreatedAt;
    /**
     * Page-event variants only: repoints what a matched route renders.
     */
    default_page_id?: null | string;
    mode:             IMode;
    /**
     * Unique per event, `[a-z0-9-]`; "canary" is the conventional name.
     */
    name:       string;
    node_id:    string;
    updated_at: ICreatedAt;
    /**
     * Merged over the event's variables per key, variant wins.
     */
    variables: { [key: string]: IVariableValue };
    [property: string]: any;
}

/**
 * How a variant handles its share of traffic: `Live` replaces the primary
 * target for `weight` of triggers, `Shadow` additionally runs the variant for
 * `sample_rate` of triggers and discards the result.
 */
export interface IMode {
    Live?:   ILive;
    Shadow?: IShadow;
}

export interface ILive {
    weight: number;
    [property: string]: any;
}

export interface IShadow {
    sample_rate: number;
    [property: string]: any;
}
