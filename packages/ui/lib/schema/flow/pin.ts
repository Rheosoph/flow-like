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
    pin_type:       IPinTypeEnum;
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

export enum IPinTypeEnum {
    Input = "Input",
    Output = "Output",
}

export enum IValueType {
    Array = "Array",
    HashMap = "HashMap",
    HashSet = "HashSet",
    Normal = "Normal",
}
