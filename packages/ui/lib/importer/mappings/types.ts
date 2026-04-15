/** Describes an n8n source node and how to extract parameters from it. */
export interface N8nNodeDef {
	/** n8n node type string, e.g. "n8n-nodes-base.if" */
	type: string;
	/** Is this a trigger / start node? */
	isEvent?: boolean;
	/**
	 * Parameter extraction map.
	 * Key = logical name used as `$name` in flow defaults.
	 * Value = dot-path into n8nNode (e.g. "parameters.url") or extraction rule.
	 */
	parameters?: Record<string, string | ParameterRule>;
	/** Warnings shown to the user during import. */
	warnings?: string[];
}

export interface ParameterRule {
	/** Dot-path into n8nNode */
	path: string;
	/** Fallback path if primary is empty */
	fallback?: string;
	/** Default value if extraction yields nothing */
	default?: unknown;
	/** Transform the extracted value */
	transform?: "uppercase" | "lowercase" | "number";
}

// ── Flow-side definitions ──────────────────────────────────

/** A direct 1:1 mapping to a single catalog node. */
export interface FlowDirectDef {
	mode: "direct";
	/** Catalog node name */
	catalog: string;
	/** Skip adding exec_in/exec_out (node already has them from catalog) */
	skipExecPins?: boolean;
	/** Static or parameter-referenced pin defaults. Use "$paramName" for extracted values. */
	defaults?: Record<string, unknown>;
}

/** A layer mapping with multiple internal nodes wired together. */
export interface FlowLayerDef {
	mode: "layer";
	/** Skip adding exec_in/exec_out on the primary node */
	skipExecPins?: boolean;
	/** Internal nodes composing the layer */
	nodes: FlowLayerNode[];
	/** Internal connections: ["nodeId:pinName", "nodeId:pinName"] */
	connections?: [string, string][];
	/** Pin defaults across all nodes. Key = "nodeId:pinName", value = static or "$param". */
	defaults?: Record<string, unknown>;
}

export interface FlowLayerNode {
	/** Local ID within this layer definition */
	id: string;
	/** Catalog node name */
	catalog: string;
	/** Position offset from the n8n node's position: [dx, dy] */
	offset?: [number, number];
	/** If true, this is the primary node (receives external exec connections) */
	primary?: boolean;
	/** Friendly name suffix appended to the n8n node name */
	nameSuffix?: string;
}

export type FlowNodeDef = FlowDirectDef | FlowLayerDef;

/** A complete mapping pair: n8n source → flow-like target. */
export interface NodeMappingDef {
	n8n: N8nNodeDef;
	flow: FlowNodeDef;
}

export interface N8nManualMappingOverride {
	name?: string;
	category?: string;
	n8n?: Omit<Partial<N8nNodeDef>, "type">;
	flow: FlowNodeDef;
}

export type N8nManualMappingOverrides = Record<
	string,
	N8nManualMappingOverride
>;

export interface ResolvedN8nMappingDef {
	name: string;
	source: "built-in" | "override";
	category?: string;
	n8n: N8nNodeDef;
	flow: FlowNodeDef;
}
