import type {
	CreateOverlayPayload,
	CypherPayload,
	GraphOverlay,
	GraphSchema,
	GraphSearchPayload,
	IGraphState,
	NeighborsPayload,
	SqlPayload,
	SubgraphNode,
	SubgraphPayload,
	SubgraphResult,
	UpdateOverlayPayload,
	ValidationResult,
} from "@flow-like/flow-like-ui";
import {
	type WebBackendRef,
	apiDelete,
	apiGet,
	apiPost,
	apiPut,
} from "./api-utils";

function scopeQuery(userScoped?: boolean): string {
	return userScoped ? "?scope=user" : "";
}

export class WebGraphState implements IGraphState {
	constructor(private readonly backend: WebBackendRef) {}

	async listOverlays(
		appId: string,
		userScoped?: boolean,
	): Promise<GraphOverlay[]> {
		return apiGet<GraphOverlay[]>(
			`apps/${appId}/graph${scopeQuery(userScoped)}`,
			this.backend.auth,
		);
	}

	async listRemoteOntologies(
		appId: string,
		targetAppId: string,
	): Promise<GraphOverlay[]> {
		return apiGet<GraphOverlay[]>(
			`apps/${appId}/connections/${targetAppId}/ontologies`,
			this.backend.auth,
		);
	}

	async createOverlay(
		appId: string,
		payload: CreateOverlayPayload,
		userScoped?: boolean,
	): Promise<GraphOverlay> {
		return apiPost<GraphOverlay>(
			`apps/${appId}/graph${scopeQuery(userScoped)}`,
			payload,
			this.backend.auth,
		);
	}

	async getOverlay(
		appId: string,
		overlayId: string,
		userScoped?: boolean,
	): Promise<GraphOverlay> {
		return apiGet<GraphOverlay>(
			`apps/${appId}/graph/${overlayId}${scopeQuery(userScoped)}`,
			this.backend.auth,
		);
	}

	async updateOverlay(
		appId: string,
		overlayId: string,
		payload: UpdateOverlayPayload,
		userScoped?: boolean,
	): Promise<GraphOverlay> {
		return apiPut<GraphOverlay>(
			`apps/${appId}/graph/${overlayId}${scopeQuery(userScoped)}`,
			payload,
			this.backend.auth,
		);
	}

	async deleteOverlay(
		appId: string,
		overlayId: string,
		userScoped?: boolean,
	): Promise<void> {
		await apiDelete<void>(
			`apps/${appId}/graph/${overlayId}${scopeQuery(userScoped)}`,
			this.backend.auth,
		);
	}

	async getSchema(
		appId: string,
		overlayId: string,
		userScoped?: boolean,
	): Promise<GraphSchema> {
		return apiGet<GraphSchema>(
			`apps/${appId}/graph/${overlayId}/schema${scopeQuery(userScoped)}`,
			this.backend.auth,
		);
	}

	async validateOverlay(
		appId: string,
		overlayId: string,
		userScoped?: boolean,
	): Promise<ValidationResult> {
		return apiPost<ValidationResult>(
			`apps/${appId}/graph/${overlayId}/validate${scopeQuery(userScoped)}`,
			undefined,
			this.backend.auth,
		);
	}

	async cypher(
		appId: string,
		overlayId: string,
		payload: CypherPayload,
		userScoped?: boolean,
	): Promise<unknown[]> {
		return apiPost<unknown[]>(
			`apps/${appId}/graph/${overlayId}/cypher${scopeQuery(userScoped)}`,
			payload,
			this.backend.auth,
		);
	}

	async sql(
		appId: string,
		overlayId: string,
		payload: SqlPayload,
		userScoped?: boolean,
	): Promise<unknown[]> {
		return apiPost<unknown[]>(
			`apps/${appId}/graph/${overlayId}/sql${scopeQuery(userScoped)}`,
			payload,
			this.backend.auth,
		);
	}

	async neighbors(
		appId: string,
		overlayId: string,
		payload: NeighborsPayload,
		userScoped?: boolean,
	): Promise<SubgraphResult> {
		return apiPost<SubgraphResult>(
			`apps/${appId}/graph/${overlayId}/neighbors${scopeQuery(userScoped)}`,
			payload,
			this.backend.auth,
		);
	}

	async subgraph(
		appId: string,
		overlayId: string,
		payload: SubgraphPayload,
		userScoped?: boolean,
	): Promise<SubgraphResult> {
		return apiPost<SubgraphResult>(
			`apps/${appId}/graph/${overlayId}/subgraph${scopeQuery(userScoped)}`,
			payload,
			this.backend.auth,
		);
	}

	async searchNodes(
		appId: string,
		overlayId: string,
		payload: GraphSearchPayload,
		userScoped?: boolean,
	): Promise<SubgraphNode[]> {
		return apiPost<SubgraphNode[]>(
			`apps/${appId}/graph/${overlayId}/search${scopeQuery(userScoped)}`,
			payload,
			this.backend.auth,
		);
	}

	async sample(
		appId: string,
		overlayId: string,
		label: string,
		n?: number,
		userScoped?: boolean,
	): Promise<unknown[]> {
		const params = new URLSearchParams();
		if (userScoped) params.set("scope", "user");
		params.set("label", label);
		if (n !== undefined) params.set("n", String(n));
		const qs = params.toString();
		return apiGet<unknown[]>(
			`apps/${appId}/graph/${overlayId}/sample${qs ? `?${qs}` : ""}`,
			this.backend.auth,
		);
	}
}
