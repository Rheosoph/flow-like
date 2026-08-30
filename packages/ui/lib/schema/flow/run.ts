import type { INodePermission } from "./board";

export interface IRun {
	board: IBoard;
	end: ISystemTime;
	id: string;
	log_level: ILogLevel;
	payload: { [key: string]: IRunPayload };
	start: ISystemTime;
	status: IRunStatus;
	sub: string;
	traces: ITrace[];
	[property: string]: any;
}

export interface IBoard {
	comments: { [key: string]: IComment };
	created_at: ISystemTime;
	description: string;
	id: string;
	layers: { [key: string]: ILayer };
	log_level: ILogLevel;
	name: string;
	nodes: { [key: string]: INode };
	refs: { [key: string]: string };
	stage: IExecutionStage;
	updated_at: ISystemTime;
	variables: { [key: string]: IVariable };
	version: number[];
	viewport: number[];
	[property: string]: any;
}

export interface IComment {
	author?: null | string;
	comment_type: ICommentType;
	content: string;
	coordinates: number[];
	id: string;
	timestamp: ISystemTime;
	/** Board node this comment is attached to (presentation only — may dangle if the node is deleted; missing node = unanchored). */
	node_id?: null | string;
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
	comments: { [key: string]: IComment };
	coordinates: number[];
	id: string;
	name: string;
	nodes: { [key: string]: INode };
	parent_id?: null | string;
	pins: { [key: string]: IPin };
	type: ILayerType;
	variables: { [key: string]: IVariable };
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
 * Score Categories (brief): privacy, security, performance, governance, reliability, cost.
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
	Module = "Module",
}

export interface IVariable {
	category?: null | string;
	data_type: IVariableType;
	default_value?: number[] | null;
	description?: null | string;
	editable: boolean;
	exposed: boolean;
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

export interface IRunPayload {
	id: string;
	payload?: any;
	runtime_variables?: Record<string, IVariable>;
	[property: string]: any;
}

export enum IRunStatus {
	Failed = "Failed",
	Running = "Running",
	Stopped = "Stopped",
	Success = "Success",
}

export interface ITrace {
	end: ISystemTime;
	id: string;
	logs: ILogMessage[];
	node_id: string;
	start: ISystemTime;
	variables?: { [key: string]: IVariable } | null;
	[property: string]: any;
}

export interface ILogMessage {
	end: ISystemTime;
	log_level: ILogLevel;
	message: string;
	operation_id?: null | string;
	start: ISystemTime;
	stats?: null | ILogStat;
	[property: string]: any;
}

export interface ILogStat {
	bit_ids?: string[] | null;
	token_in?: number | null;
	token_out?: number | null;
	[property: string]: any;
}
