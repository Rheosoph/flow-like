import { invoke } from "@tauri-apps/api/core";
import {
	type IMetadata,
	type IWidget,
	type IWidgetState,
	type Version,
	type VersionType,
	injectDataFunction,
} from "@tm9657/flow-like-ui";
import { fetcher } from "../../lib/api";
import type { TauriBackend } from "../tauri-provider";

export class WidgetState implements IWidgetState {
	constructor(private readonly backend: TauriBackend) {}

	private getRemoteAuth() {
		return this.backend.auth?.isAuthenticated ? this.backend.auth : undefined;
	}

	private hasRemote(): boolean {
		return !!(this.backend.profile && this.backend.auth?.isAuthenticated);
	}

	private async pushWidgetRemote(
		appId: string,
		widget: IWidget,
	): Promise<void> {
		if (!this.backend.profile) return;
		await fetcher(
			this.backend.profile,
			`apps/${appId}/widgets/${widget.id}`,
			{
				method: "PUT",
				body: JSON.stringify({ widget }),
			},
			this.getRemoteAuth(),
		);
	}

	private async fetchRemoteWidget(
		appId: string,
		widgetId: string,
		version?: Version,
	): Promise<IWidget> {
		if (!this.backend.profile) {
			throw new Error("Profile not set. Cannot fetch remote widget.");
		}
		const versionQuery = version ? `?version=${version.join(".")}` : "";
		return fetcher<IWidget>(
			this.backend.profile,
			`apps/${appId}/widgets/${widgetId}${versionQuery}`,
			{ method: "GET" },
			this.getRemoteAuth(),
		);
	}

	private async buildListResult(
		appId: string,
		widgets: IWidget[],
		language?: string,
	): Promise<[string, string, IMetadata | undefined][]> {
		const result: [string, string, IMetadata | undefined][] = [];
		for (const widget of widgets) {
			let metadata: IMetadata | undefined;
			try {
				metadata = await invoke<IMetadata>("get_widget_meta", {
					appId,
					widgetId: widget.id,
					language,
				});
			} catch {
				metadata = undefined;
			}
			result.push([appId, widget.id, metadata]);
		}
		return result;
	}

	async getWidgets(
		appId: string,
		language?: string,
	): Promise<[string, string, IMetadata | undefined][]> {
		const localWidgets = await invoke<IWidget[]>("get_widgets", { appId });
		const localResult = await this.buildListResult(
			appId,
			localWidgets,
			language,
		);

		const isOffline = await this.backend.isOffline(appId);
		const profile = this.backend.profile;
		if (
			isOffline ||
			!this.backend.queryClient ||
			!profile ||
			!this.hasRemote()
		) {
			return localResult;
		}

		const syncRemote = async () => {
			const params = language ? `?language=${language}` : "";
			const remoteList = await fetcher<
				[string, string, IMetadata | undefined][]
			>(
				profile,
				`apps/${appId}/widgets${params}`,
				{ method: "GET" },
				this.getRemoteAuth(),
			);

			const localById = new Map(localWidgets.map((w) => [w.id, w]));
			const remoteIds = new Set(remoteList.map(([, id]) => id));

			// Pull remote widgets that don't exist locally; refresh ones that do
			for (const [, widgetId] of remoteList) {
				try {
					const remoteWidget = await this.fetchRemoteWidget(appId, widgetId);
					const local = localById.get(widgetId);
					if (
						!local ||
						new Date(remoteWidget.updatedAt).getTime() >
							new Date(local.updatedAt).getTime()
					) {
						await invoke("update_widget", { appId, widget: remoteWidget });
						localById.set(widgetId, remoteWidget);
					}
				} catch (e) {
					console.warn(
						"[WidgetState] Failed to pull remote widget:",
						widgetId,
						e,
					);
				}
			}

			// Push local widgets the remote does not yet know about (e.g. created offline)
			for (const local of localWidgets) {
				if (!remoteIds.has(local.id)) {
					try {
						await this.pushWidgetRemote(appId, local);
					} catch (e) {
						console.warn(
							"[WidgetState] Failed to push local widget to remote:",
							local.id,
							e,
						);
					}
				}
			}

			return this.buildListResult(
				appId,
				Array.from(localById.values()),
				language,
			);
		};

		const promise = injectDataFunction(
			syncRemote,
			this,
			this.backend.queryClient,
			this.getWidgets,
			[appId, language],
			[],
			localResult,
		);
		this.backend.backgroundTaskHandler(promise);

		return localResult;
	}

	async getWidget(
		appId: string,
		widgetId: string,
		version?: Version,
	): Promise<IWidget> {
		let local: IWidget | undefined;
		try {
			local = await invoke<IWidget>("get_widget", {
				appId,
				widgetId,
				version,
			});
		} catch {
			local = undefined;
		}

		const isOffline = await this.backend.isOffline(appId);

		if (local) {
			if (isOffline || !this.backend.queryClient || !this.hasRemote()) {
				return local;
			}

			const localSnapshot = local;
			const syncRemote = async () => {
				const remote = await this.fetchRemoteWidget(appId, widgetId, version);
				if (
					!version &&
					new Date(remote.updatedAt).getTime() >
						new Date(localSnapshot.updatedAt).getTime()
				) {
					await invoke("update_widget", { appId, widget: remote });
					return remote;
				}
				return localSnapshot;
			};
			const promise = injectDataFunction(
				syncRemote,
				this,
				this.backend.queryClient,
				this.getWidget,
				[appId, widgetId, version],
				[],
				local,
			);
			this.backend.backgroundTaskHandler(promise);
			return local;
		}

		if (isOffline || !this.hasRemote()) {
			throw new Error(`Widget not found: ${widgetId}`);
		}

		const remote = await this.fetchRemoteWidget(appId, widgetId, version);
		if (!version) {
			try {
				await invoke("update_widget", { appId, widget: remote });
			} catch (e) {
				console.warn("[WidgetState] Failed to cache remote widget locally:", e);
			}
		}
		return remote;
	}

	async createWidget(
		appId: string,
		widgetId: string,
		name: string,
		description?: string,
	): Promise<IWidget> {
		const widget = await invoke<IWidget>("create_widget", {
			appId,
			widgetId,
			name,
			description,
		});

		const isOffline = await this.backend.isOffline(appId);
		if (!isOffline && this.hasRemote()) {
			try {
				await this.pushWidgetRemote(appId, widget);
			} catch (e) {
				console.warn("[WidgetState] Failed to push new widget to remote:", e);
			}
		}
		return widget;
	}

	async updateWidget(appId: string, widget: IWidget): Promise<void> {
		await invoke("update_widget", { appId, widget });

		const isOffline = await this.backend.isOffline(appId);
		if (!isOffline && this.hasRemote()) {
			try {
				await this.pushWidgetRemote(appId, widget);
			} catch (e) {
				console.warn(
					"[WidgetState] Failed to push widget update to remote:",
					e,
				);
			}
		}
	}

	async deleteWidget(appId: string, widgetId: string): Promise<void> {
		await invoke("delete_widget", { appId, widgetId });

		const isOffline = await this.backend.isOffline(appId);
		if (!isOffline && this.backend.profile && this.hasRemote()) {
			try {
				await fetcher(
					this.backend.profile,
					`apps/${appId}/widgets/${widgetId}`,
					{ method: "DELETE" },
					this.getRemoteAuth(),
				);
			} catch (e) {
				console.warn("[WidgetState] Failed to delete widget on remote:", e);
			}
		}
	}

	async createWidgetVersion(
		appId: string,
		widgetId: string,
		versionType: VersionType,
	): Promise<Version> {
		const version = await invoke<Version>("create_widget_version", {
			appId,
			widgetId,
			versionType,
		});

		const isOffline = await this.backend.isOffline(appId);
		if (!isOffline && this.hasRemote()) {
			try {
				const widget = await invoke<IWidget>("get_widget", {
					appId,
					widgetId,
				});
				await this.pushWidgetRemote(appId, widget);
			} catch (e) {
				console.warn("[WidgetState] Failed to push version bump to remote:", e);
			}
		}
		return version;
	}

	async getWidgetVersions(appId: string, widgetId: string): Promise<Version[]> {
		return invoke<Version[]>("get_widget_versions", { appId, widgetId });
	}

	async getOpenWidgets(): Promise<[string, string, string][]> {
		return invoke<[string, string, string][]>("get_open_widgets");
	}

	async closeWidget(widgetId: string): Promise<void> {
		return invoke("close_widget", { widgetId });
	}

	async getWidgetMeta(
		appId: string,
		widgetId: string,
		language?: string,
	): Promise<IMetadata> {
		return invoke<IMetadata>("get_widget_meta", { appId, widgetId, language });
	}

	async pushWidgetMeta(
		appId: string,
		widgetId: string,
		metadata: IMetadata,
		language?: string,
	): Promise<void> {
		return invoke("push_widget_meta", { appId, widgetId, metadata, language });
	}
}
