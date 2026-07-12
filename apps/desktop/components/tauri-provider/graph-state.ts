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
import { invoke } from "@tauri-apps/api/core";
import { fetcher } from "../../lib/api";
import type { TauriBackend } from "../tauri-provider";

function scopeQuery(userScoped?: boolean): string {
	return userScoped ? "?scope=user" : "";
}

export class GraphState implements IGraphState {
	constructor(private readonly backend: TauriBackend) {}

	async listOverlays(
		appId: string,
		userScoped?: boolean,
	): Promise<GraphOverlay[]> {
		const isOffline = await this.backend.isOffline(appId);

		if (!isOffline) {
			return fetcher<GraphOverlay[]>(
				this.backend.profile!,
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

	async createOverlay(
		appId: string,
		payload: CreateOverlayPayload,
		userScoped?: boolean,
	): Promise<GraphOverlay> {
		const isOffline = await this.backend.isOffline(appId);

		if (!isOffline) {
			return fetcher<GraphOverlay>(
				this.backend.profile!,
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
				this.backend.profile!,
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
				this.backend.profile!,
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
				this.backend.profile!,
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
				this.backend.profile!,
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
				this.backend.profile!,
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
				this.backend.profile!,
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
				this.backend.profile!,
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
				this.backend.profile!,
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
				this.backend.profile!,
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
				this.backend.profile!,
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
				this.backend.profile!,
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
