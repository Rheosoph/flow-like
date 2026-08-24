import type { INodePermission } from "../../board";

export interface IUpsertLayer {
	current_layer?: null | string;
	layer: ILayer;
	node_ids: string[];
	old_layer?: null | ILayer;
	[property: string]: any;
}

export interface ILayer {
	cache?: ILayerCache | null;
	/**
	 * Folder the layer is filed under in the sidebar, nested with forward slashes. Empty
	 * means the top level. Purely organizational, it does not affect execution.
	 */
	category?: null | string;
	color?: null | string;
	comment?: null | string;
	comments: { [key: string]: IComment };
	coordinates: number[];
	error?: null | string;
	hash?: number | null;
	id: string;
	in_coordinates?: number[] | null;
	name: string;
	nodes: { [key: string]: INode };
	out_coordinates?: number[] | null;
	parent_id?: null | string;
	pins: { [key: string]: IPin };
	type: ILayerType;
	variables: { [key: string]: IVariable };
	[property: string]: any;
}

/**
 * Result caching for a layer invoked as a function.
 *
 * A hit replaces the whole call: the function body never runs, so its side effects do
 * not happen either. Only turn this on for layers whose outputs are a function of their
 * inputs.
 */
export interface ILayerCache {
	enabled?: boolean;
	/**
	 * Namespace every entry for this layer is written under, so one layer's cache can be
	 * invalidated without touching the rest of the app's.
	 */
	prefix?: string;
	scope?: ILayerCacheScope;
	/**
	 * Lifetime of an entry in seconds. `None` or `0` keeps it until it is invalidated.
	 */
	ttl_seconds?: number | null;
	[property: string]: any;
}

/**
 * Who a layer's cached results belong to.
 */
export enum ILayerCacheScope {
	App = "app",
	User = "user",
}

export interface IComment {
	author?: null | string;
	color?: null | string;
	comment_type: ICommentType;
	content: string;
	coordinates: number[];
	hash?: number | null;
	height?: number | null;
	id: string;
	is_locked?: boolean | null;
	layer?: null | string;
	timestamp: ISystemTime;
	width?: number | null;
	z_index?: number | null;
	[property: string]: any;
}

export enum ICommentType {
	Image = "Image",
	Text = "Text",
	Video = "Video",
}

export interface ISystemTime {
	nanos_since_epoch: number;
	secs_since_epoch: number;
	[property: string]: any;
}

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
	/** WASM metadata for external nodes. Undefined for built-in catalog nodes. */
	wasm?: INodeWasm | null;
	/** FlowScript namespace path (`string`, `http`, `utils.markdown`). Presentation only. */
	namespace?: null | string;
	/** FlowScript member name inside `namespace` (`trim`, `fetch`). Presentation only. */
	alias?: null | string;
	/** Data input pin that receives the value in method form; `""` = static only. */
	receiver?: null | string;
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
 * Represents quality metrics for a node, with scores ranging from 0 to 10. Higher scores
 * indicate worse performance in each category.
 *
 * # Score Categories * `privacy` - Measures data protection and confidentiality level *
 * `security` - Assesses resistance against potential attacks * `performance` - Evaluates
 * computational efficiency and speed * `governance` - Indicates compliance with policies
 * and regulations
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

export enum ILayerType {
	Collapsed = "Collapsed",
	Function = "Function",
	Macro = "Macro",
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
