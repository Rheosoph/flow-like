import type {
	CreateOverlayPayload,
	CypherPayload,
	GraphSearchPayload,
	GraphOverlay,
	GraphSchema,
	IGraphState,
	NeighborsPayload,
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
