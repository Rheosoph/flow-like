export interface IBoard {
	comments: { [key: string]: IComment };
	created_at: ISystemTime;
	description: string;
	id: string;
	layers: { [key: string]: ILayer };
	log_level: ILogLevel;
	execution_mode: IExecutionMode;
	name: string;
	nodes: { [key: string]: INode };
	refs: { [key: string]: string };
	stage: IExecutionStage;
	updated_at: ISystemTime;
	variables: { [key: string]: IVariable };
	version: number[];
	viewport: number[];
	page_ids: string[];
	[property: string]: any;
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
	[property: string]: any;
}

export enum INodePermission {
	NetworkHttp = "network:http",
	NetworkWebsocket = "network:websocket",
	NetworkTcp = "network:tcp",
	NetworkUdp = "network:udp",
	NetworkDns = "network:dns",
	StorageRead = "storage:read",
	StorageWrite = "storage:write",
	Variables = "variables",
	Cache = "cache",
	Streaming = "streaming",
	Models = "models",
	A2ui = "a2ui",
	OAuth = "oauth",
	Functions = "functions",
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
 * - `performance` — Computational cost / latency (higher is worse)
 * - `governance` — Compliance and auditability
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

export enum ILogLevel {
	Debug = "Debug",
	Error = "Error",
	Fatal = "Fatal",
	Info = "Info",
	Warn = "Warn",
}

export enum IExecutionStage {
	Dev = "Dev",
	Int = "Int",
	PreProd = "PreProd",
	Prod = "Prod",
	QA = "QA",
}

export enum IExecutionMode {
	Hybrid = "Hybrid",
	Remote = "Remote",
	Local = "Local",
}
