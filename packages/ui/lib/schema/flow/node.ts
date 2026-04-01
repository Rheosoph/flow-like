import type { INodePermission } from "./board";

export interface INode {
	category: string;
	comment?: null | string;
	coordinates?: number[] | null;
	description: string;
	docs?: null | string;
	error?: null | string;
	event_callback?: boolean | null;
	friendly_name: string;
	hash?: number | null;
	fn_refs?: IFnRefs | null;
	icon?: null | string;
	id: string;
	layer?: null | string;
	long_running?: boolean | null;
	name: string;
	pins: { [key: string]: IPin };
	scores?: null | INodeScores;
	start?: boolean | null;
	/** Schema version for node migration. When catalog version > placed version, pins are synced. */
	version?: number | null;
	/** If true, this node can only run locally (compute-intensive, RPA, browser automation) */
	only_offline?: boolean;
	/** WASM metadata for external nodes. Undefined for built-in catalog nodes. */
	wasm?: INodeWasm | null;
	[property: string]: any;
}

export interface INodeWasm {
	package_id: string;
	permissions: INodePermission[];
}

export interface IPin {
	connected_to: string[];
	data_type: IVariableType;
	default_value?: number[] | null;
	depends_on: string[];
	description: string;
	friendly_name: string;
	id: string;
	index: number;
	name: string;
	options?: null | IPinOptions;
	pin_type: IPinType;
	schema?: null | string;
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
}

export interface IPinOptions {
	enforce_generic_value_type?: boolean | null;
	enforce_schema?: boolean | null;
	range?: number[] | null;
	sensitive?: boolean | null;
	step?: number | null;
	valid_values?: string[] | null;
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
 * Represents quality metrics for a node. Scores range from 0 to 10 (low - high).
 * A higher score indicates a larger issue or resource impact in the category.
 *
 * Score Categories:
 * - `privacy` — Data protection and confidentiality (0 low - 10 high)
 * - `security` — Resistance to attack and exposure
 * - `performance` — Computational cost / latency (high values mean worse performance)
 * - `governance` — Compliance, auditability and policy alignment
 * - `reliability` — Stability, error rates and recoverability
 * - `cost` — Resource / financial impact of running this node
 */
export interface INodeScores {
	governance: number;
	performance: number;
	privacy: number;
	security: number;
	reliability: number;
	cost: number;
	[property: string]: any;
}

export interface IFnRefs {
	fn_refs: string[];
	can_reference_fns: boolean;
	can_be_referenced_by_fns: boolean;
	[property: string]: any;
}
