/**
 * An alternate dispatch target for an event: a canary (`Live`) or shadow
 * deployment of another board/version/node. Read through
 * [`Event::variant_set`], which falls back to the legacy `canary` field.
 */
export interface IVariant {
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

export interface ICreatedAt {
    nanos_since_epoch: number;
    secs_since_epoch:  number;
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
