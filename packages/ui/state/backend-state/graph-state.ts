// ─── Graph overlay types (mirrors Rust catalog-core types) ───

export interface NodeSize {
	mode: "fixed" | "by-degree" | "by-column";
	value?: number;
	min?: number;
	max?: number;
	column?: string;
}

export interface LabelStyle {
	color: string;
	icon: string;
	size: NodeSize;
	shape?: string;
	width?: number;
}

export interface PropertyColumn {
	name: string;
	data_type: string;
	nullable?: boolean;
}

export interface NodeLabelMapping {
	label: string;
	table: string;
	id_column: string;
	display_column?: string;
	property_columns: PropertyColumn[];
	style: LabelStyle;
}

export interface EdgeLabelMapping {
	label: string;
	table: string;
	src_column: string;
	dst_column: string;
	src_label: string;
	dst_label: string;
	src_node_column?: string;
	dst_node_column?: string;
	property_columns: PropertyColumn[];
	style: LabelStyle;
}

export interface GraphOverlay {
	id: string;
	name: string;
	description?: string;
	nodes: NodeLabelMapping[];
	edges: EdgeLabelMapping[];
	default_limit: number;
	created_at: string;
	updated_at: string;
}

export interface SubgraphNode {
	id: string;
	label: string;
	caption?: string;
	props: Record<string, unknown>;
	style: LabelStyle;
}

export interface SubgraphEdge {
	id: string;
	source: string;
	target: string;
	label: string;
	props: Record<string, unknown>;
	style: LabelStyle;
}

export interface SubgraphResult {
	nodes: SubgraphNode[];
	edges: SubgraphEdge[];
	truncated: boolean;
}

export interface GraphLabelInfo {
	label: string;
	table: string;
	properties: GraphPropertyInfo[];
}

export interface GraphPropertyInfo {
	name: string;
	data_type: string;
	nullable: boolean;
}

export interface GraphSchema {
	node_labels: GraphLabelInfo[];
	edge_labels: GraphLabelInfo[];
}

export interface ValidationResult {
	ok: boolean;
	issues: string[];
}

// ─── Payloads ───

export interface CreateOverlayPayload {
	name: string;
	description?: string;
	nodes: NodeLabelMapping[];
	edges: EdgeLabelMapping[];
	default_limit?: number;
}

export interface UpdateOverlayPayload {
	name?: string;
	description?: string;
	nodes?: NodeLabelMapping[];
	edges?: EdgeLabelMapping[];
	default_limit?: number;
}

export interface CypherPayload {
	query: string;
	params?: Record<string, unknown>;
	limit?: number;
}

export interface SqlPayload {
	query: string;
	limit?: number;
}

export interface NeighborsPayload {
	label: string;
	node_id: unknown;
	depth?: number;
	direction?: "outgoing" | "incoming" | "both";
	limit?: number;
}

export interface SubgraphPayload {
	seeds: { label: string; id: unknown }[];
	depth?: number;
	limit?: number;
}

export interface GraphSearchPayload {
	query: string;
	limit?: number;
}

// ─── State interface ───

export interface IGraphState {
	listOverlays(appId: string, userScoped?: boolean): Promise<GraphOverlay[]>;
	createOverlay(
		appId: string,
		payload: CreateOverlayPayload,
		userScoped?: boolean,
	): Promise<GraphOverlay>;
	getOverlay(
		appId: string,
		overlayId: string,
		userScoped?: boolean,
	): Promise<GraphOverlay>;
	updateOverlay(
		appId: string,
		overlayId: string,
		payload: UpdateOverlayPayload,
		userScoped?: boolean,
	): Promise<GraphOverlay>;
	deleteOverlay(
		appId: string,
		overlayId: string,
		userScoped?: boolean,
	): Promise<void>;
	getSchema(
		appId: string,
		overlayId: string,
		userScoped?: boolean,
	): Promise<GraphSchema>;
	validateOverlay(
		appId: string,
		overlayId: string,
		userScoped?: boolean,
	): Promise<ValidationResult>;
	cypher(
		appId: string,
		overlayId: string,
		payload: CypherPayload,
		userScoped?: boolean,
	): Promise<unknown[]>;
	sql(
		appId: string,
		overlayId: string,
		payload: SqlPayload,
		userScoped?: boolean,
	): Promise<unknown[]>;
	neighbors(
		appId: string,
		overlayId: string,
		payload: NeighborsPayload,
		userScoped?: boolean,
	): Promise<SubgraphResult>;
	subgraph(
		appId: string,
		overlayId: string,
		payload: SubgraphPayload,
		userScoped?: boolean,
	): Promise<SubgraphResult>;
	searchNodes(
		appId: string,
		overlayId: string,
		payload: GraphSearchPayload,
		userScoped?: boolean,
	): Promise<SubgraphNode[]>;
	sample(
		appId: string,
		overlayId: string,
		label: string,
		n?: number,
		userScoped?: boolean,
	): Promise<unknown[]>;
}
