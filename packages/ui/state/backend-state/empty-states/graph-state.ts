import type {
	CreateOverlayPayload,
	CypherPayload,
	GraphAnalyticsResult,
	GraphOverlay,
	GraphPathsResult,
	GraphSchema,
	GraphSearchPayload,
	IGraphState,
	InvokeOntologyActionPayload,
	NeighborsPayload,
	OntologyActionPrerun,
	OntologyActionRun,
	OverlayChildrenPayload,
	PathsPayload,
	RemoteOntologyImport,
	SqlPayload,
	SubgraphNode,
	SubgraphPayload,
	SubgraphResult,
	UpdateOverlayPayload,
	UpsertGraphElementsResult,
	ValidationResult,
} from "../graph-state";
import type { ExecuteSqlResult } from "../query-state";

export class EmptyGraphState implements IGraphState {
	listOverlays(): Promise<GraphOverlay[]> {
		throw new Error("Method not implemented.");
	}
	listRemoteOntologies(): Promise<GraphOverlay[]> {
		throw new Error("Method not implemented.");
	}
	listRemoteOntologyImports(): Promise<RemoteOntologyImport[]> {
		throw new Error("Method not implemented.");
	}
	installRemoteOntology(): Promise<RemoteOntologyImport> {
		throw new Error("Method not implemented.");
	}
	uninstallRemoteOntology(): Promise<void> {
		throw new Error("Method not implemented.");
	}
	sampleRemoteImport(): Promise<unknown[]> {
		throw new Error("Method not implemented.");
	}
	queryRemoteImport(): Promise<ExecuteSqlResult> {
		throw new Error("Method not implemented.");
	}
	invokeOntologyAction(
		_appId: string,
		_ontologyId: string,
		_actionId: string,
		_payload: InvokeOntologyActionPayload,
		_onStatus?: (run: OntologyActionRun) => void,
	): Promise<OntologyActionRun> {
		throw new Error("Method not implemented.");
	}
	prerunOntologyAction(): Promise<OntologyActionPrerun> {
		return Promise.resolve({ oauth_requirements: [], signature: "" });
	}
	createOverlay(
		_appId: string,
		_payload: CreateOverlayPayload,
	): Promise<GraphOverlay> {
		throw new Error("Method not implemented.");
	}
	getOverlay(): Promise<GraphOverlay> {
		throw new Error("Method not implemented.");
	}
	updateOverlay(
		_appId: string,
		_overlayId: string,
		_payload: UpdateOverlayPayload,
	): Promise<GraphOverlay> {
		throw new Error("Method not implemented.");
	}
	deleteOverlay(): Promise<void> {
		throw new Error("Method not implemented.");
	}
	getSchema(): Promise<GraphSchema> {
		throw new Error("Method not implemented.");
	}
	validateOverlay(): Promise<ValidationResult> {
		throw new Error("Method not implemented.");
	}
	cypher(
		_appId: string,
		_overlayId: string,
		_payload: CypherPayload,
	): Promise<unknown[]> {
		throw new Error("Method not implemented.");
	}
	sql(
		_appId: string,
		_overlayId: string,
		_payload: SqlPayload,
	): Promise<unknown[]> {
		throw new Error("Method not implemented.");
	}
	neighbors(
		_appId: string,
		_overlayId: string,
		_payload: NeighborsPayload,
	): Promise<SubgraphResult> {
		throw new Error("Method not implemented.");
	}
	children(
		_appId: string,
		_overlayId: string,
		_payload: OverlayChildrenPayload,
	): Promise<SubgraphResult> {
		throw new Error("Method not implemented.");
	}
	subgraph(
		_appId: string,
		_overlayId: string,
		_payload: SubgraphPayload,
	): Promise<SubgraphResult> {
		throw new Error("Method not implemented.");
	}
	paths(
		_appId: string,
		_overlayId: string,
		_payload: PathsPayload,
	): Promise<GraphPathsResult> {
		throw new Error("Method not implemented.");
	}
	analytics(
		_appId: string,
		_overlayId: string,
		_limit?: number,
	): Promise<GraphAnalyticsResult> {
		throw new Error("Method not implemented.");
	}
	searchNodes(
		_appId: string,
		_overlayId: string,
		_payload: GraphSearchPayload,
	): Promise<SubgraphNode[]> {
		throw new Error("Method not implemented.");
	}
	sample(): Promise<unknown[]> {
		throw new Error("Method not implemented.");
	}
	upsertNodes(): Promise<UpsertGraphElementsResult> {
		throw new Error("Method not implemented.");
	}
	upsertEdges(): Promise<UpsertGraphElementsResult> {
		throw new Error("Method not implemented.");
	}
}
