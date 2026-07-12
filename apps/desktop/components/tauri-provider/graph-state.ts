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
	OntologyActionStreamEvent,
	RemoteOntologyImport,
	SqlPayload,
	SubgraphNode,
	SubgraphPayload,
	SubgraphResult,
	UpdateOverlayPayload,
	ValidationResult,
} from "@flow-like/flow-like-ui";
import { applyOntologyActionStreamEvent } from "@flow-like/flow-like-ui";
import { invoke } from "@tauri-apps/api/core";
import { fetcher } from "../../lib/api";
import type { TauriBackend } from "../tauri-provider";

function scopeQuery(userScoped?: boolean): string {
	return userScoped ? "?scope=user" : "";
}

export class GraphState implements IGraphState {
	constructor(private readonly backend: TauriBackend) {}

	private requireProfile() {
		const profile = this.backend.profile;
		if (!profile)
			throw new Error("An active profile is required for this request.");
		return profile;
	}

	async listOverlays(
		appId: string,
		userScoped?: boolean,
	): Promise<GraphOverlay[]> {
		const isOffline = await this.backend.isOffline(appId);

		if (!isOffline) {
			return fetcher<GraphOverlay[]>(
				this.requireProfile(),
				`apps/${appId}/graph${scopeQuery(userScoped)}`,
				{ method: "GET" },
				this.backend.auth,
			);
		}

		return invoke("graph_list_overlays", {
			appId,
			userScoped: userScoped ?? false,
		});
	}

	async listRemoteOntologies(
		appId: string,
		targetAppId: string,
	): Promise<GraphOverlay[]> {
		const isOffline = await this.backend.isOffline(appId);
		if (isOffline || !this.backend.profile || !this.backend.auth) return [];
		return fetcher<GraphOverlay[]>(
			this.backend.profile,
			`apps/${appId}/connections/${targetAppId}/ontologies`,
			{ method: "GET" },
			this.backend.auth,
		);
	}

	async listRemoteOntologyImports(
		appId: string,
	): Promise<RemoteOntologyImport[]> {
		const isOffline = await this.backend.isOffline(appId);
		if (isOffline || !this.backend.profile || !this.backend.auth) return [];
		return fetcher<RemoteOntologyImport[]>(
			this.backend.profile,
			`apps/${appId}/graph/imports`,
			{ method: "GET" },
			this.backend.auth,
		);
	}

	async installRemoteOntology(
		appId: string,
		targetAppId: string,
		ontologyId: string,
	): Promise<RemoteOntologyImport> {
		const isOffline = await this.backend.isOffline(appId);
		if (isOffline || !this.backend.profile || !this.backend.auth) {
			throw new Error("Remote ontology imports require an online connection.");
		}
		return fetcher<RemoteOntologyImport>(
			this.backend.profile,
			`apps/${appId}/connections/${targetAppId}/ontologies/${ontologyId}/install`,
			{ method: "PUT" },
			this.backend.auth,
		);
	}

	async uninstallRemoteOntology(
		appId: string,
		targetAppId: string,
		ontologyId: string,
	): Promise<void> {
		const isOffline = await this.backend.isOffline(appId);
		if (isOffline || !this.backend.profile || !this.backend.auth) {
			throw new Error("Remote ontology imports require an online connection.");
		}
		await fetcher<void>(
			this.backend.profile,
			`apps/${appId}/connections/${targetAppId}/ontologies/${ontologyId}/install`,
			{ method: "DELETE" },
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
		const isOffline = await this.backend.isOffline(appId);
		if (isOffline) {
			const prepared = await invoke<{
				event_id: string;
				payload: unknown;
			}>("graph_prepare_ontology_action", {
				appId,
				ontologyId,
				actionId,
				payload,
			});
			const run: OntologyActionRun = { run_id: "", status: "Submitting" };
			onStatus?.({ ...run });
			const metadata = await this.backend.eventState.executeEvent(
				appId,
				prepared.event_id,
				{ id: prepared.event_id, payload: prepared.payload },
				false,
				(runId) => {
					run.run_id = runId;
					run.status = "Running";
					onStatus?.({ ...run });
				},
				(events) => {
					for (const event of events) {
						applyOntologyActionStreamEvent(run, event);
						onStatus?.({ ...run });
					}
				},
			);
			if (!run.run_id && metadata?.run_id) run.run_id = metadata.run_id;
			if (
				(run.status === "Submitting" || run.status === "Running") &&
				metadata
			) {
				if ((metadata.log_level ?? 0) >= 3) {
					run.status = "Failed";
					run.error_message =
						"The action run reported an error. Open the run for details.";
				} else {
					run.status = "Completed";
				}
			} else if (run.status === "Submitting" || run.status === "Running") {
				run.status = "Interrupted";
				run.error_message =
					"The local action ended without completion metadata. Check the run before retrying.";
			}
			onStatus?.({ ...run });
			return run;
		}
		if (!this.backend.profile || !this.backend.auth) {
			throw new Error(
				"An authenticated profile is required to invoke an action.",
			);
		}
		const run: OntologyActionRun = { run_id: "", status: "Submitting" };
		onStatus?.({ ...run });
		try {
			const requestPayload = {
				...payload,
				token: this.backend.auth.user?.access_token,
				profile_id: this.backend.profile.id,
			};
			await this.backend.apiState.stream<OntologyActionStreamEvent>(
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
		if (await this.backend.isOffline(appId)) {
			throw new Error("Offline actions use the local execution preflight.");
		}
		return fetcher<OntologyActionPrerun>(
			this.requireProfile(),
			`apps/${appId}/graph/${ontologyId}/actions/${actionId}/prerun`,
			{ method: "GET" },
			this.backend.auth,
		);
	}

	async createOverlay(
		appId: string,
		payload: CreateOverlayPayload,
		userScoped?: boolean,
	): Promise<GraphOverlay> {
		const isOffline = await this.backend.isOffline(appId);

		if (!isOffline) {
			return fetcher<GraphOverlay>(
				this.requireProfile(),
				`apps/${appId}/graph${scopeQuery(userScoped)}`,
				{ method: "POST", body: JSON.stringify(payload) },
				this.backend.auth,
			);
		}

		return invoke("graph_create_overlay", {
			appId,
			payload,
			userScoped: userScoped ?? false,
		});
	}

	async getOverlay(
		appId: string,
		overlayId: string,
		userScoped?: boolean,
	): Promise<GraphOverlay> {
		const isOffline = await this.backend.isOffline(appId);

		if (!isOffline) {
			return fetcher<GraphOverlay>(
				this.requireProfile(),
				`apps/${appId}/graph/${overlayId}${scopeQuery(userScoped)}`,
				{ method: "GET" },
				this.backend.auth,
			);
		}

		return invoke("graph_get_overlay", {
			appId,
			overlayId,
			userScoped: userScoped ?? false,
		});
	}

	async updateOverlay(
		appId: string,
		overlayId: string,
		payload: UpdateOverlayPayload,
		userScoped?: boolean,
	): Promise<GraphOverlay> {
		const isOffline = await this.backend.isOffline(appId);

		if (!isOffline) {
			return fetcher<GraphOverlay>(
				this.requireProfile(),
				`apps/${appId}/graph/${overlayId}${scopeQuery(userScoped)}`,
				{ method: "PUT", body: JSON.stringify(payload) },
				this.backend.auth,
			);
		}

		return invoke("graph_update_overlay", {
			appId,
			overlayId,
			payload,
			userScoped: userScoped ?? false,
		});
	}

	async deleteOverlay(
		appId: string,
		overlayId: string,
		userScoped?: boolean,
	): Promise<void> {
		const isOffline = await this.backend.isOffline(appId);

		if (!isOffline) {
			await fetcher<void>(
				this.requireProfile(),
				`apps/${appId}/graph/${overlayId}${scopeQuery(userScoped)}`,
				{ method: "DELETE" },
				this.backend.auth,
			);
			return;
		}

		await invoke("graph_delete_overlay", {
			appId,
			overlayId,
			userScoped: userScoped ?? false,
		});
	}

	async getSchema(
		appId: string,
		overlayId: string,
		userScoped?: boolean,
	): Promise<GraphSchema> {
		const isOffline = await this.backend.isOffline(appId);

		if (!isOffline) {
			return fetcher<GraphSchema>(
				this.requireProfile(),
				`apps/${appId}/graph/${overlayId}/schema${scopeQuery(userScoped)}`,
				{ method: "GET" },
				this.backend.auth,
			);
		}

		return invoke("graph_get_schema", {
			appId,
			overlayId,
			userScoped: userScoped ?? false,
		});
	}

	async validateOverlay(
		appId: string,
		overlayId: string,
		userScoped?: boolean,
	): Promise<ValidationResult> {
		const isOffline = await this.backend.isOffline(appId);

		if (!isOffline) {
			return fetcher<ValidationResult>(
				this.requireProfile(),
				`apps/${appId}/graph/${overlayId}/validate${scopeQuery(userScoped)}`,
				{ method: "POST" },
				this.backend.auth,
			);
		}

		return invoke("graph_validate_overlay", {
			appId,
			overlayId,
			userScoped: userScoped ?? false,
		});
	}

	async cypher(
		appId: string,
		overlayId: string,
		payload: CypherPayload,
		userScoped?: boolean,
	): Promise<unknown[]> {
		const isOffline = await this.backend.isOffline(appId);

		if (!isOffline) {
			return fetcher<unknown[]>(
				this.requireProfile(),
				`apps/${appId}/graph/${overlayId}/cypher${scopeQuery(userScoped)}`,
				{ method: "POST", body: JSON.stringify(payload) },
				this.backend.auth,
			);
		}

		return invoke("graph_cypher", {
			appId,
			overlayId,
			payload,
			userScoped: userScoped ?? false,
		});
	}

	async sql(
		appId: string,
		overlayId: string,
		payload: SqlPayload,
		userScoped?: boolean,
	): Promise<unknown[]> {
		const isOffline = await this.backend.isOffline(appId);

		if (!isOffline) {
			return fetcher<unknown[]>(
				this.requireProfile(),
				`apps/${appId}/graph/${overlayId}/sql${scopeQuery(userScoped)}`,
				{ method: "POST", body: JSON.stringify(payload) },
				this.backend.auth,
			);
		}

		return invoke("graph_sql", {
			appId,
			overlayId,
			payload,
			userScoped: userScoped ?? false,
		});
	}

	async neighbors(
		appId: string,
		overlayId: string,
		payload: NeighborsPayload,
		userScoped?: boolean,
	): Promise<SubgraphResult> {
		const isOffline = await this.backend.isOffline(appId);

		if (!isOffline) {
			return fetcher<SubgraphResult>(
				this.requireProfile(),
				`apps/${appId}/graph/${overlayId}/neighbors${scopeQuery(userScoped)}`,
				{ method: "POST", body: JSON.stringify(payload) },
				this.backend.auth,
			);
		}

		return invoke("graph_neighbors", {
			appId,
			overlayId,
			payload,
			userScoped: userScoped ?? false,
		});
	}

	async subgraph(
		appId: string,
		overlayId: string,
		payload: SubgraphPayload,
		userScoped?: boolean,
	): Promise<SubgraphResult> {
		const isOffline = await this.backend.isOffline(appId);

		if (!isOffline) {
			return fetcher<SubgraphResult>(
				this.requireProfile(),
				`apps/${appId}/graph/${overlayId}/subgraph${scopeQuery(userScoped)}`,
				{ method: "POST", body: JSON.stringify(payload) },
				this.backend.auth,
			);
		}

		return invoke("graph_subgraph", {
			appId,
			overlayId,
			payload,
			userScoped: userScoped ?? false,
		});
	}

	async searchNodes(
		appId: string,
		overlayId: string,
		payload: GraphSearchPayload,
		userScoped?: boolean,
	): Promise<SubgraphNode[]> {
		const isOffline = await this.backend.isOffline(appId);

		if (!isOffline) {
			return fetcher<SubgraphNode[]>(
				this.requireProfile(),
				`apps/${appId}/graph/${overlayId}/search${scopeQuery(userScoped)}`,
				{ method: "POST", body: JSON.stringify(payload) },
				this.backend.auth,
			);
		}

		return invoke("graph_search_nodes", {
			appId,
			overlayId,
			payload,
			userScoped: userScoped ?? false,
		});
	}

	async sample(
		appId: string,
		overlayId: string,
		label: string,
		n?: number,
		userScoped?: boolean,
	): Promise<unknown[]> {
		const isOffline = await this.backend.isOffline(appId);

		if (!isOffline) {
			const params = new URLSearchParams();
			if (userScoped) params.set("scope", "user");
			params.set("label", label);
			if (n !== undefined) params.set("n", String(n));
			const qs = params.toString();
			return fetcher<unknown[]>(
				this.requireProfile(),
				`apps/${appId}/graph/${overlayId}/sample${qs ? `?${qs}` : ""}`,
				{ method: "GET" },
				this.backend.auth,
			);
		}

		return invoke("graph_sample", {
			appId,
			overlayId,
			label,
			n,
			userScoped: userScoped ?? false,
		});
	}
}
