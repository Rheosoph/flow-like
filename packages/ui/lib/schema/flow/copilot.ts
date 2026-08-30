import type { ILayerCache } from "./board";

/// Agent types for the multi-agent system
export type AgentType = "Explain" | "Edit";

/// Role in the chat conversation
export type ChatRole = "User" | "Assistant";

/// An image attachment in a chat message
export interface ChatImage {
	/** Base64-encoded image data (without data URL prefix) */
	data: string;
	/** MIME type (e.g., "image/png", "image/jpeg") */
	media_type: string;
}

/// A message in the chat history
export interface ChatMessage {
	role: ChatRole;
	content: string;
	/** Optional images attached to this message (for vision models) */
	images?: ChatImage[];
}

export interface Suggestion {
	node_type: string;
	reason: string;
	connection_description: string;
	position?: { x: number; y: number };
	connections: Array<{
		from_node_id: string;
		from_pin: string;
		to_pin: string;
	}>;
}

/// Pin definition for placeholder nodes
export interface PlaceholderPinDef {
	name: string; // Internal name for the pin
	friendly_name: string; // Display name for the pin
	description?: string;
	pin_type: "Input" | "Output";
	data_type:
		| "String"
		| "Integer"
		| "Float"
		| "Boolean"
		| "Struct"
		| "Generic"
		| "Execution"
		| "Date"
		| "PathBuf"
		| "Byte";
	value_type?: "Normal" | "Array" | "HashMap" | "HashSet";
	/** JSON Schema carried by a typed Struct boundary. */
	schema?: string;
	/** Require connected pins to agree with schema. */
	enforce_schema?: boolean;
}

export type BoardCommand =
	| {
			command_type: "AddNode";
			node_type: string;
			ref_id?: string; // Reference ID for this node (e.g., "$0", "$1") used in connections
			position?: { x: number; y: number };
			friendly_name?: string;
			/** Extra pins appended to a new catalog node; supported only for events_generic outputs. */
			additional_pins?: PlaceholderPinDef[];
			/** Target layer ID to place the node in. If undefined, uses current layer. */
			target_layer?: string;
			summary?: string;
	  }
	| {
			command_type: "AddPlaceholder";
			name: string;
			ref_id?: string;
			position?: { x: number; y: number };
			pins?: PlaceholderPinDef[];
			/** Target layer ID to place the placeholder in. If undefined, uses current layer. */
			target_layer?: string;
			summary?: string;
	  }
	| { command_type: "RemoveNode"; node_id: string; summary?: string }
	| {
			command_type: "ConnectPins";
			from_node: string;
			from_pin: string;
			to_node: string;
			to_pin: string;
			summary?: string;
	  }
	| {
			command_type: "DisconnectPins";
			from_node: string;
			from_pin: string;
			to_node: string;
			to_pin: string;
			summary?: string;
	  }
	| {
			command_type: "UpdateNodePin";
			node_id: string;
			pin_id: string;
			value: unknown;
			summary?: string;
	  }
	| {
			command_type: "RenameNode";
			node_id: string;
			friendly_name: string;
			summary?: string;
	  }
	| {
			command_type: "SetNodeFunctionRefs";
			node_id: string;
			/** Referenced targets as ref tokens (`$N` ref-ids, node/layer anchors, or names). */
			fn_refs: string[];
			summary?: string;
	  }
	| {
			command_type: "MoveNode";
			node_id: string;
			position: { x: number; y: number };
			/** Target layer ID to move the node to. If undefined, moves within current layer. */
			target_layer?: string;
			summary?: string;
	  }
	| {
			command_type: "CreateVariable";
			variable_id?: string;
			name: string;
			data_type: string;
			value_type?: string;
			default_value?: unknown;
			description?: string;
			category?: string;
			schema?: string;
			exposed?: boolean;
			secret?: boolean;
			editable?: boolean;
			runtime_configured?: boolean;
			target_layer?: string;
			summary?: string;
	  }
	| {
			command_type: "UpdateVariable";
			variable_id: string;
			name?: string;
			data_type?: string;
			value_type?: string;
			default_value?: unknown;
			clear_default_value?: boolean;
			description?: string;
			clear_description?: boolean;
			category?: string;
			clear_category?: boolean;
			schema?: string;
			clear_schema?: boolean;
			exposed?: boolean;
			secret?: boolean;
			editable?: boolean;
			runtime_configured?: boolean;
			value?: unknown;
			summary?: string;
	  }
	| { command_type: "DeleteVariable"; variable_id: string; summary?: string }
	| {
			command_type: "CreateComment";
			content: string;
			position?: { x: number; y: number };
			width?: number;
			height?: number;
			color?: string;
			/** Target layer ID to place the comment in. If undefined, uses current layer. */
			target_layer?: string;
			summary?: string;
	  }
	| {
			command_type: "UpdateComment";
			comment_id: string;
			content?: string;
			color?: string;
			summary?: string;
	  }
	| { command_type: "DeleteComment"; comment_id: string; summary?: string }
	| {
			command_type: "CreateLayer";
			name: string;
			ref_id?: string;
			layer_type?: "Collapsed" | "Function" | "Macro" | "Module";
			color?: string;
			node_ids?: string[];
			pins?: PlaceholderPinDef[];
			/** Result-cache settings for a Function layer. */
			cache?: ILayerCache | null;
			position?: { x: number; y: number };
			/** Parent layer ID for nesting. If undefined, creates at current layer. */
			target_layer?: string;
			summary?: string;
	  }
	| {
			command_type: "UpdateLayerCache";
			layer_id: string;
			/** New cache settings, or explicit null to remove caching. */
			cache: ILayerCache | null;
			summary?: string;
	  }
	| {
			command_type: "AddNodesToLayer";
			layer_id: string;
			node_ids: string[];
			summary?: string;
	  }
	| {
			command_type: "RemoveNodesFromLayer";
			layer_id: string;
			node_ids: string[];
			summary?: string;
	  }
	| {
			/** Rename an existing layer without touching its contents or position. */
			command_type: "RenameLayer";
			layer_id: string;
			name: string;
			summary?: string;
	  }
	| {
			/**
			 * Re-home nodes, comments, or layers into a module layer (undefined = board root).
			 * FlowScript emits this when an anchored function, event, or module block is written
			 * in a different module than the one it lives in on the board.
			 */
			command_type: "MoveToLayer";
			ids: string[];
			target_layer?: string;
			summary?: string;
	  };

/// Response from the copilot
export interface CopilotResponse {
	agent_type: AgentType;
	message: string;
	commands: BoardCommand[];
	suggestions: Suggestion[];
	flowscript_workspace?: string;
}

export interface PlanStep {
	id: string;
	description: string;
	status: "Pending" | "InProgress" | "Completed" | "Failed";
	tool_name?: string;
}
