import { invoke } from "@tauri-apps/api/core";
import {
	IAppVisibility,
	type IMetadata,
	type IWidget,
	type IWidgetState,
	type Version,
	type VersionType,
} from "@flow-like/flow-like-ui";
import { appsDB } from "../../lib/apps-db";
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

	private async canFetchRemoteWidget(appId: string): Promise<boolean> {
		if (!this.hasRemote()) return false;

		const cachedVisibility = await appsDB.visibility.get(appId);
		if (cachedVisibility) {
			return cachedVisibility.visibility !== IAppVisibility.Offline;
		}

		try {
			const app = await invoke<{ visibility?: IAppVisibility }>("get_app", {
				appId,
			});
			const visibility = app.visibility ?? IAppVisibility.Offline;
			await appsDB.visibility.put({ appId, visibility });
			return visibility !== IAppVisibility.Offline;
		} catch {
			// The widget can come from a remote app that has not been cached locally.
			return true;
		}
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

		const isOffline = await this.backend.isOffline(appId);
		const profile = this.backend.profile;
		if (isOffline || !profile || !this.hasRemote()) {
			return await this.buildListResult(appId, localWidgets, language);
		}

		try {
			const params = language ? `?language=${language}` : "";
			const remoteList = await fetcher<
				[string, string, IMetadata | undefined][]
			>(
				profile,
				`apps/${appId}/widgets${params}`,
				{ method: "GET" },
				this.getRemoteAuth(),
			);

			const remoteIds = new Set(remoteList.map(([, id]) => id));

			const localOnly = localWidgets.filter((w) => !remoteIds.has(w.id));
			const localOnlyMeta = await Promise.all(
				localOnly.map(async (w) => {
					try {
						return await invoke<IMetadata>("get_widget_meta", {
							appId,
							widgetId: w.id,
							language,
						});
					} catch {
						return undefined;
					}
				}),
			);

			const result: [string, string, IMetadata | undefined][] = [];
			for (const [, widgetId, metadata] of remoteList) {
				result.push([appId, widgetId, metadata]);
			}
			for (let i = 0; i < localOnly.length; i++) {
				result.push([appId, localOnly[i].id, localOnlyMeta[i]]);
			}

			const syncTask = (async () => {
				for (const [, widgetId, metadata] of remoteList) {
					if (metadata) {
						try {
							await invoke("push_widget_meta", {
								appId,
								widgetId,
								metadata,
								language,
							});
						} catch (e) {
							console.warn(
								"[WidgetState] Failed to persist remote widget metadata locally:",
								widgetId,
								e,
							);
						}
					}
					try {
						const remoteWidget = await this.fetchRemoteWidget(appId, widgetId);
						await invoke("update_widget", { appId, widget: remoteWidget });
					} catch (e) {
						console.warn(
							"[WidgetState] Failed to pull remote widget:",
							widgetId,
							e,
						);
					}
				}
				for (const local of localOnly) {
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
			})();
			this.backend.backgroundTaskHandler(syncTask);

			return result;
		} catch (e) {
			console.warn(
				"[WidgetState] Falling back to local widgets list, remote fetch failed:",
				e,
			);
			return await this.buildListResult(appId, localWidgets, language);
		}
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

		const canFetchRemote = await this.canFetchRemoteWidget(appId);

		if (!canFetchRemote) {
			if (local) {
				return local;
			}
			throw new Error(`Widget not found: ${widgetId}`);
		}

		try {
			const remote = await this.fetchRemoteWidget(appId, widgetId, version);
			if (!version) {
				try {
					await invoke("update_widget", { appId, widget: remote });
				} catch (e) {
					console.warn("[WidgetState] Failed to cache remote widget locally:", e);
				}
			}
			return remote;
		} catch (e) {
			if (local) {
				console.warn(
					"[WidgetState] Falling back to local widget, remote fetch failed:",
					widgetId,
					e,
				);
				return local;
			}
			throw e;
		}
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
