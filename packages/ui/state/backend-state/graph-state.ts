import type { ExecuteSqlResult } from "./query-state";

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
	id?: string;
	api_name?: string;
	label: string;
	table: string;
	id_column: string;
	display_column?: string;
	property_columns: PropertyColumn[];
	style: LabelStyle;
}

export interface EdgeLabelMapping {
	id?: string;
	api_name?: string;
	label: string;
	table: string;
	src_column: string;
	dst_column: string;
	src_label: string;
	dst_label: string;
	src_node_column?: string;
	dst_node_column?: string;
	/** Marks a hierarchy/drill-down spine edge (src_label = parent, dst_label = child). */
	containment?: boolean;
	/** Child objects live in another local overlay (its id). */
	dst_ontology?: string;
	/** Child objects live in an installed remote ontology (its import id). */
	dst_binding_id?: string;
	property_columns: PropertyColumn[];
	style: LabelStyle;
}

export interface GraphOverlay {
	id: string;
	name: string;
	description?: string;
	nodes: NodeLabelMapping[];
	edges: EdgeLabelMapping[];
	object_views: ObjectViewDefinition[];
	actions: OntologyActionDefinition[];
	exposed: boolean;
	bindings_enabled: boolean;
	default_limit: number;
	created_at: string;
	updated_at: string;
}

export interface RemoteOntologyImport {
	id: string;
	target_app_id: string;
	remote_ontology_id: string;
	contract: GraphOverlay;
	source_updated_at: string;
	bindings_enabled: boolean;
	installed_at: string;
	updated_at: string;
}

export interface ObjectViewDefinition {
	object_type: string;
	title_property?: string;
	prominent_properties: string[];
}

export interface OntologyActionDefinition {
	id: string;
	name: string;
	description?: string;
	object_type: string;
	board_id: string;
	board_version?: [number, number, number];
	start_node_id?: string;
	event_id?: string;
	enabled: boolean;
	allow_bulk: boolean;
	parameter_schema?: Record<string, unknown>;
	/** Per-action exposure to connected projects (default exposed). */
	exposed?: boolean;
}

export interface GraphEdgeLabelCount {
	label: string;
	count: number;
}

export interface SubgraphNodeStats {
	out_by_label: GraphEdgeLabelCount[];
	/** False when the counts were read off a sampling window, making them lower bounds. */
	exact: boolean;
}

export interface SubgraphNode {
	id: string;
	label: string;
	caption?: string;
	props: Record<string, unknown>;
	/** Only the seedless sampler knows a node's whole-population fan-out. */
	stats?: SubgraphNodeStats;
	/** Not sent by the server — resolved client-side from the overlay. */
	style?: LabelStyle;
}

export interface SubgraphEdge {
	id: string;
	source: string;
	target: string;
	label: string;
	props: Record<string, unknown>;
	/** Not sent by the server — resolved client-side from the overlay. */
	style?: LabelStyle;
}

export interface SubgraphResult {
	nodes: SubgraphNode[];
	edges: SubgraphEdge[];
	truncated: boolean;
	warnings?: string[];
}

export interface GraphPath {
	node_ids: string[];
	edge_ids: string[];
	length: number;
}

export interface GraphPathsResult {
	found: boolean;
	paths: GraphPath[];
	nodes: SubgraphNode[];
	edges: SubgraphEdge[];
	truncated: boolean;
	warnings?: string[];
}

export interface GraphLabelCount {
	label: string;
	nodes: number;
}

export interface GraphNodeMetric {
	id: string;
	label: string;
	caption?: string;
	degree_in: number;
	degree_out: number;
	pagerank: number;
	component: number;
}

export interface GraphAnalyticsResult {
	node_count: number;
	edge_count: number;
	truncated: boolean;
	label_counts: GraphLabelCount[];
	component_count: number;
	largest_components: number[];
	isolated_node_count: number;
	top_by_degree: GraphNodeMetric[];
	top_by_pagerank: GraphNodeMetric[];
	warnings?: string[];
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

export interface MappingValidation {
	kind: "node" | "edge";
	label: string;
	ok: boolean;
	issues: string[];
}

export interface ValidationResult {
	ok: boolean;
	issues: string[];
	mappings?: MappingValidation[];
}

// ─── Payloads ───

export interface CreateOverlayPayload {
	name: string;
	description?: string;
	nodes: NodeLabelMapping[];
	edges: EdgeLabelMapping[];
	object_views?: ObjectViewDefinition[];
	actions?: OntologyActionDefinition[];
	exposed?: boolean;
	bindings_enabled?: boolean;
	default_limit?: number;
}

export interface UpdateOverlayPayload {
	expected_updated_at?: string;
	name?: string;
	description?: string;
	nodes?: NodeLabelMapping[];
	edges?: EdgeLabelMapping[];
	object_views?: ObjectViewDefinition[];
	actions?: OntologyActionDefinition[];
	exposed?: boolean;
	bindings_enabled?: boolean;
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

export interface UpsertGraphElementsPayload {
	/** Node or edge label to write into; must exist in the overlay schema. */
	label: string;
	/** Rows to upsert. Node rows carry the id column; edge rows the source + target id columns. */
	rows: Record<string, unknown>[];
}

export interface UpsertGraphElementsResult {
	upserted: number;
}

export interface NeighborsPayload {
	label: string;
	node_id: unknown;
	depth?: number;
	direction?: "outgoing" | "incoming" | "both";
	limit?: number;
	/** Relationship labels to follow. Omit or leave empty to follow all of them. */
	edge_labels?: string[];
}

export interface OverlayChildrenPayload {
	label: string;
	node_id: unknown;
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

export interface PathsPayload {
	from_label: string;
	from_id: unknown;
	to_label: string;
	to_id: unknown;
	max_depth?: number;
	limit?: number;
}

export interface OntologyObjectRef {
	object_type: string;
	id: unknown;
}

export interface RemoteImportQueryPayload {
	sql: string;
	params?: Record<string, unknown>;
	limit?: number;
}

export interface InvokeOntologyActionPayload {
	object_refs: OntologyObjectRef[];
	parameters?: Record<string, unknown>;
	idempotency_key?: string;
	oauth_tokens?: Record<string, unknown>;
}

export interface OntologyActionRun {
	run_id: string;
	status: string;
	result?: unknown;
	error_message?: string;
}

export interface OntologyActionPrerun {
	oauth_requirements: Array<{ provider_id: string; scopes: string[] }>;
	signature: string;
}

export interface OntologyActionStreamEvent {
	event_type?: string;
	payload?: unknown;
}

export function applyOntologyActionStreamEvent(
	run: OntologyActionRun,
	event: OntologyActionStreamEvent,
): void {
	const payload =
		event.payload && typeof event.payload === "object"
			? (event.payload as Record<string, unknown>)
			: undefined;
	switch (event.event_type) {
		case "run_initiated":
			if (typeof payload?.run_id === "string") run.run_id = payload.run_id;
			run.status = "Running";
			break;
		case "generic_result":
			run.result = event.payload;
			break;
		case "completed":
			run.status =
				typeof payload?.status === "string" ? payload.status : "Completed";
			break;
		case "error":
			run.status = "Failed";
			run.error_message =
				typeof event.payload === "string"
					? event.payload
					: typeof payload?.message === "string"
						? payload.message
						: typeof payload?.error === "string"
							? payload.error
							: "The ontology action failed.";
			break;
	}
}

// ─── State interface ───

export interface IGraphState {
	listOverlays(appId: string, userScoped?: boolean): Promise<GraphOverlay[]>;
	listRemoteOntologyImports(appId: string): Promise<RemoteOntologyImport[]>;
	listRemoteOntologies(
		appId: string,
		targetAppId: string,
	): Promise<GraphOverlay[]>;
	installRemoteOntology(
		appId: string,
		targetAppId: string,
		ontologyId: string,
	): Promise<RemoteOntologyImport>;
	uninstallRemoteOntology(
		appId: string,
		targetAppId: string,
		ontologyId: string,
	): Promise<void>;
	/** Preview rows for an object of an installed remote ontology (read live from the source). */
	sampleRemoteImport(
		appId: string,
		importId: string,
		label: string,
		n?: number,
	): Promise<unknown[]>;
	/** Run a read-only query against an installed remote ontology's exposed tables. */
	queryRemoteImport(
		appId: string,
		importId: string,
		payload: RemoteImportQueryPayload,
	): Promise<ExecuteSqlResult>;
	invokeOntologyAction(
		appId: string,
		ontologyId: string,
		actionId: string,
		payload: InvokeOntologyActionPayload,
		onStatus?: (run: OntologyActionRun) => void,
	): Promise<OntologyActionRun>;
	prerunOntologyAction(
		appId: string,
		ontologyId: string,
		actionId: string,
	): Promise<OntologyActionPrerun>;
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
		draft?: GraphOverlay,
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
	children(
		appId: string,
		overlayId: string,
		payload: OverlayChildrenPayload,
		userScoped?: boolean,
	): Promise<SubgraphResult>;
	subgraph(
		appId: string,
		overlayId: string,
		payload: SubgraphPayload,
		userScoped?: boolean,
	): Promise<SubgraphResult>;
	paths(
		appId: string,
		overlayId: string,
		payload: PathsPayload,
		userScoped?: boolean,
	): Promise<GraphPathsResult>;
	analytics(
		appId: string,
		overlayId: string,
		limit?: number,
		userScoped?: boolean,
	): Promise<GraphAnalyticsResult>;
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
	upsertNodes(
		appId: string,
		overlayId: string,
		payload: UpsertGraphElementsPayload,
		userScoped?: boolean,
	): Promise<UpsertGraphElementsResult>;
	upsertEdges(
		appId: string,
		overlayId: string,
		payload: UpsertGraphElementsPayload,
		userScoped?: boolean,
	): Promise<UpsertGraphElementsResult>;
}
