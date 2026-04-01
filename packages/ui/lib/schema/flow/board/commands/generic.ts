import type { INodePermission } from "../../board";

export interface IGeneric {
	command_type: ICommandType;
	comment?: IComment;
	old_comment?: null | IComment;
	node?: INode;
	new_comments?: IComment[];
	new_nodes?: INode[];
	offset?: number[];
	original_comments?: IComment[];
	original_nodes?: INode[];
	from_coordinates?: number[] | null;
	node_id?: string;
	to_coordinates?: number[];
	connected_nodes?: INode[];
	old_node?: null | INode;
	from_node?: string;
	from_pin?: string;
	to_node?: string;
	to_pin?: string;
	old_pin?: null | IPin;
	pin?: IPin;
	variable?: IVariable;
	old_variable?: null | IVariable;
	[property: string]: any;
}

export enum ICommandType {
	AddNode = "AddNode",
	ConnectPin = "ConnectPin",
	CopyPaste = "CopyPaste",
	DisconnectPin = "DisconnectPin",
	MoveNode = "MoveNode",
	RemoveComment = "RemoveComment",
	RemoveNode = "RemoveNode",
	RemoveVariable = "RemoveVariable",
	UpdateNode = "UpdateNode",
	UpsertComment = "UpsertComment",
	UpsertPin = "UpsertPin",
	UpsertVariable = "UpsertVariable",
}

export interface IComment {
	author?: null | string;
	comment_type: ICommentType;
	content: string;
	coordinates: number[];
	id: string;
	timestamp: ISystemTime;
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
	friendly_name: string;
	icon?: null | string;
	id: string;
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
	valid_values?: string[] | null;
	value_type: IValueType;
	[property: string]: any;
}

export enum IVariableType {
	Boolean = "Boolean",
	Byte = "Byte",
	Date = "Date",
	Execution = "Execution",
	Float = "Float",
	IGeneric = "Generic",
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
