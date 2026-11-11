export interface IMoveToLayer {
    ids:       string[];
    previous?: { [key: string]: null | string };
    /**
     * Every pin the move rewired while it unthreaded group bridges, as it was before the move.
     */
    previous_pins?: IPreviousPinElement[];
    target?:        null | string;
    [property: string]: any;
}

/**
 * A pin as it was before a move rewired or removed it.
 */
export interface IPreviousPinElement {
    holder: IHolder;
    pin:    IPin;
    [property: string]: any;
}

/**
 * Where a pin lives.
 */
export interface IHolder {
    Node?:      string;
    Layer?:     string;
    LayerNode?: ILayerNode;
}

export interface ILayerNode {
    layer: string;
    node:  string;
    [property: string]: any;
}

export interface IPin {
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
