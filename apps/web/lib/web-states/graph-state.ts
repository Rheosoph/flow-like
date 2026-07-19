import type {
	CreateOverlayPayload,
	CypherPayload,
	ExecuteSqlResult,
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
	OntologyActionStreamEvent,
	OverlayChildrenPayload,
	PathsPayload,
	RemoteImportQueryPayload,
	RemoteOntologyImport,
	SqlPayload,
	SubgraphNode,
	SubgraphPayload,
	SubgraphResult,
	UpdateOverlayPayload,
	UpsertGraphElementsPayload,
	UpsertGraphElementsResult,
	ValidationResult,
} from "@flow-like/flow-like-ui";
import { applyOntologyActionStreamEvent } from "@flow-like/flow-like-ui";
import { WebApiState } from "./api-state";
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

	async listRemoteOntologyImports(
		appId: string,
	): Promise<RemoteOntologyImport[]> {
		return apiGet<RemoteOntologyImport[]>(
			`apps/${appId}/graph/imports`,
			this.backend.auth,
		);
	}

	async installRemoteOntology(
		appId: string,
		targetAppId: string,
		ontologyId: string,
	): Promise<RemoteOntologyImport> {
		return apiPut<RemoteOntologyImport>(
			`apps/${appId}/connections/${targetAppId}/ontologies/${ontologyId}/install`,
			undefined,
			this.backend.auth,
		);
	}

	async uninstallRemoteOntology(
		appId: string,
		targetAppId: string,
		ontologyId: string,
	): Promise<void> {
		await apiDelete<void>(
			`apps/${appId}/connections/${targetAppId}/ontologies/${ontologyId}/install`,
			this.backend.auth,
		);
	}

	async invokeOntologyAction(
		appId: string,
		ontologyId: string,
		actionId: string,
		payload: InvokeOntologyActionPayload,
		onStatus?: (run: OntologyActionRun) => void,
	): Promise<OntologyActionRun> {
		if (!this.backend.profile) {
			throw new Error("An active profile is required to invoke an action.");
		}
		const run: OntologyActionRun = { run_id: "", status: "Submitting" };
		onStatus?.({ ...run });
		try {
			const requestPayload = {
				...payload,
				token: this.backend.auth?.user?.access_token,
				profile_id: this.backend.profile.id,
			};
			await new WebApiState(this.backend).stream<OntologyActionStreamEvent>(
				this.backend.profile,
				`apps/${appId}/graph/${ontologyId}/actions/${actionId}/invoke`,
				{
					method: "POST",
					body: JSON.stringify(requestPayload),
					headers: { Accept: "text/event-stream" },
				},
				(event) => {
					applyOntologyActionStreamEvent(run, event);
					onStatus?.({ ...run });
				},
			);
		} catch (error) {
			if (!run.run_id) throw error;
			run.status = "Failed";
			run.error_message ??=
				error instanceof Error ? error.message : "The ontology action failed.";
		}
		if (run.status === "Submitting" || run.status === "Running") {
			run.status = "Interrupted";
			run.error_message =
				"The action stream ended before a terminal status was received. Check the run before retrying.";
		}
		onStatus?.({ ...run });
		return run;
	}

	async prerunOntologyAction(
		appId: string,
		ontologyId: string,
		actionId: string,
	): Promise<OntologyActionPrerun> {
		return apiGet<OntologyActionPrerun>(
			`apps/${appId}/graph/${ontologyId}/actions/${actionId}/prerun`,
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
		draft?: GraphOverlay,
	): Promise<ValidationResult> {
		return apiPost<ValidationResult>(
			`apps/${appId}/graph/${overlayId}/validate${scopeQuery(userScoped)}`,
			draft,
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

	async children(
		appId: string,
		overlayId: string,
		payload: OverlayChildrenPayload,
		userScoped?: boolean,
	): Promise<SubgraphResult> {
		return apiPost<SubgraphResult>(
			`apps/${appId}/graph/${overlayId}/children${scopeQuery(userScoped)}`,
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

	async paths(
		appId: string,
		overlayId: string,
		payload: PathsPayload,
		userScoped?: boolean,
	): Promise<GraphPathsResult> {
		return apiPost<GraphPathsResult>(
			`apps/${appId}/graph/${overlayId}/paths${scopeQuery(userScoped)}`,
			payload,
			this.backend.auth,
		);
	}

	async analytics(
		appId: string,
		overlayId: string,
		limit?: number,
		userScoped?: boolean,
	): Promise<GraphAnalyticsResult> {
		const params = new URLSearchParams();
		if (userScoped) params.set("scope", "user");
		if (limit !== undefined) params.set("limit", String(limit));
		const qs = params.toString();
		return apiGet<GraphAnalyticsResult>(
			`apps/${appId}/graph/${overlayId}/analytics${qs ? `?${qs}` : ""}`,
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

	async upsertNodes(
		appId: string,
		overlayId: string,
		payload: UpsertGraphElementsPayload,
		userScoped?: boolean,
	): Promise<UpsertGraphElementsResult> {
		return apiPost<UpsertGraphElementsResult>(
			`apps/${appId}/graph/${overlayId}/nodes${scopeQuery(userScoped)}`,
			payload,
			this.backend.auth,
		);
	}

	async upsertEdges(
		appId: string,
		overlayId: string,
		payload: UpsertGraphElementsPayload,
		userScoped?: boolean,
	): Promise<UpsertGraphElementsResult> {
		return apiPost<UpsertGraphElementsResult>(
			`apps/${appId}/graph/${overlayId}/edges${scopeQuery(userScoped)}`,
			payload,
			this.backend.auth,
		);
	}

	async sampleRemoteImport(
		appId: string,
		importId: string,
		label: string,
		n?: number,
	): Promise<unknown[]> {
		const params = new URLSearchParams({ label });
		if (n !== undefined) params.set("n", String(n));
		return apiGet<unknown[]>(
			`apps/${appId}/graph/imports/${encodeURIComponent(importId)}/sample?${params.toString()}`,
			this.backend.auth,
		);
	}

	async queryRemoteImport(
		appId: string,
		importId: string,
		payload: RemoteImportQueryPayload,
	): Promise<ExecuteSqlResult> {
		return apiPost<ExecuteSqlResult>(
			`apps/${appId}/graph/imports/${encodeURIComponent(importId)}/query`,
			payload,
			this.backend.auth,
		);
	}
}
