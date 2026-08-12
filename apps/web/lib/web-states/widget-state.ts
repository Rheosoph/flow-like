import {
	type IMetadata,
	type IWidgetState,
	normalizeWidgetForPersistence,
} from "@flow-like/flow-like-ui";
import type {
	IWidget,
	Version,
	VersionType,
} from "@flow-like/flow-like-ui/state/backend-state/widget-state";
import {
	type WebBackendRef,
	apiDelete,
	apiGet,
	apiPost,
	apiPut,
} from "./api-utils";

export class WebWidgetState implements IWidgetState {
	constructor(private readonly backend: WebBackendRef) {}

	async getWidgets(
		appId: string,
		language?: string,
	): Promise<[string, string, IMetadata | undefined][]> {
		const params = language ? `?language=${language}` : "";
		try {
			return await apiGet<[string, string, IMetadata | undefined][]>(
				`apps/${appId}/widgets${params}`,
				this.backend.auth,
			);
		} catch {
			return [];
		}
	}

	async getWidget(
		appId: string,
		widgetId: string,
		version?: Version,
	): Promise<IWidget> {
		const params = version ? `?version=${version.join(".")}` : "";
		return apiGet<IWidget>(
			`apps/${appId}/widgets/${widgetId}${params}`,
			this.backend.auth,
		);
	}

	async createWidget(
		appId: string,
		widgetId: string,
		name: string,
		description?: string,
	): Promise<IWidget> {
		const now = new Date().toISOString();
		const newWidget: IWidget = {
			id: widgetId,
			name,
			description,
			rootComponentId: "root",
			components: [],
			dataModel: [],
			customizationOptions: [],
			exposedProps: [],
			tags: [],
			version: [0, 0, 1],
			createdAt: now,
			updatedAt: now,
			actions: [],
		};
		return apiPut<IWidget>(
			`apps/${appId}/widgets/${widgetId}`,
			{ widget: newWidget },
			this.backend.auth,
		);
	}

	async updateWidget(appId: string, widget: IWidget): Promise<void> {
		const normalizedWidget = normalizeWidgetForPersistence(widget);
		await apiPut(
			`apps/${appId}/widgets/${widget.id}`,
			{ widget: normalizedWidget },
			this.backend.auth,
		);
	}

	async deleteWidget(appId: string, widgetId: string): Promise<void> {
		await apiDelete(`apps/${appId}/widgets/${widgetId}`, this.backend.auth);
	}

	async createWidgetVersion(
		appId: string,
		widgetId: string,
		versionType: VersionType,
	): Promise<Version> {
		return apiPost<Version>(
			`apps/${appId}/widgets/${widgetId}/versions`,
			{ version_type: versionType },
			this.backend.auth,
		);
	}

	async getWidgetVersions(appId: string, widgetId: string): Promise<Version[]> {
		try {
			return await apiGet<Version[]>(
				`apps/${appId}/widgets/${widgetId}/versions`,
				this.backend.auth,
			);
		} catch {
			return [];
		}
	}

	async getOpenWidgets(): Promise<[string, string, string][]> {
		// In web mode, we don't track open widgets locally
		return [];
	}

	async closeWidget(widgetId: string): Promise<void> {
		// No-op in web mode
	}

	async getWidgetMeta(
		appId: string,
		widgetId: string,
		language?: string,
	): Promise<IMetadata> {
		return apiGet<IMetadata>(
			`apps/${appId}/meta?language=${language ?? "en"}&widget_id=${widgetId}`,
			this.backend.auth,
		);
	}

	async pushWidgetMeta(
		appId: string,
		widgetId: string,
		metadata: IMetadata,
		language?: string,
	): Promise<void> {
		await apiPut(
			`apps/${appId}/meta?language=${language ?? "en"}&widget_id=${widgetId}`,
			metadata,
			this.backend.auth,
		);
	}
}
