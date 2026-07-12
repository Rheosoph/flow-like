import type {
	CreateOverlayPayload,
	CypherPayload,
	GraphOverlay,
	GraphSchema,
	GraphSearchPayload,
	IGraphState,
	InvokeOntologyActionPayload,
	NeighborsPayload,
	OntologyActionPrerun,
	OntologyActionRun,
	RemoteOntologyImport,
	SqlPayload,
	SubgraphNode,
	SubgraphPayload,
	SubgraphResult,
	UpdateOverlayPayload,
	ValidationResult,
} from "../graph-state";

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
	subgraph(
		_appId: string,
		_overlayId: string,
		_payload: SubgraphPayload,
	): Promise<SubgraphResult> {
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
}
