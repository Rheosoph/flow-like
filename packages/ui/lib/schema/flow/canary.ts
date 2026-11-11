export interface ICanary {
	board_id: string;
	board_version?: number[] | null;
	created_at: ISystemTime;
	node_id: string;
	updated_at: ISystemTime;
	variables: { [key: string]: IVariable };
	weight: number;
	[property: string]: any;
}

export interface ISystemTime {
	nanos_since_epoch: number;
	secs_since_epoch: number;
	[property: string]: any;
}

export interface IVariable {
	category?: null | string;
	data_type: IVariableType;
	default_value?: number[] | null;
	description?: null | string;
	editable: boolean;
	exposed: boolean;
	hash?: number | null;
	id: string;
	name: string;
	schema?: null | string;
	secret: boolean;
	value_type: IValueType;
	[property: string]: any;
}

export enum IVariableType {
	Boolean = "Boolean",
	Byte = "Byte",
	Date = "Date",
	Execution = "Execution",
	Float = "Float",
	Generic = "Generic",
	Integer = "Integer",
	PathBuf = "PathBuf",
	String = "String",
	Struct = "Struct",
	Geometry = "Geometry",
}

export enum IValueType {
	Array = "Array",
	HashMap = "HashMap",
	HashSet = "HashSet",
	Normal = "Normal",
}
