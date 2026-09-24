import {
	type IMetadata,
	type IWidget,
	type IWidgetState,
	type Version,
	type VersionType,
	applyWidgetRename,
	normalizeWidgetForPersistence,
} from "@flow-like/flow-like-ui";
import {
	ApiResponseError,
	UPSTREAM_UNAVAILABLE_CODE,
	isHubUnavailable,
} from "@flow-like/flow-like-ui/lib/api-error";
import { isRecord } from "@flow-like/flow-like-ui/lib/response-shape";
import { invoke } from "@tauri-apps/api/core";
import { fetcher } from "../../lib/api";
import { HUB_REFRESH_TIMEOUT_MS } from "../../lib/request-deadline";
import { isMissingResourceError } from "../../lib/api-error";
import { withWidgetName } from "../../lib/widget-metadata";
import type { TauriBackend } from "../tauri-provider";

/** A 2xx body of the wrong shape reads as an unavailable hub, never as data. */
function malformedResponseError(path: string, expected: string) {
	return new ApiResponseError({
		status: 502,
		code: UPSTREAM_UNAVAILABLE_CODE,
		message: `Expected ${expected} but the API returned a malformed response`,
		path,
	});
}

function isWidgetPayload(value: unknown): value is IWidget {
	return (
		isRecord(value) &&
		typeof value.id === "string" &&
		Array.isArray(value.components)
	);
}

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
		// Unknown visibility must allow the read that populates a fresh device.
		return !(await this.backend.isLocalOnly(appId));
	}

	private async requiresRemoteWrite(appId: string): Promise<boolean> {
		if (await this.backend.isLocalOnly(appId)) return false;
		if (!this.hasRemote()) {
			throw new Error(
				"Updating a hosted widget requires an authenticated hub session",
			);
		}
		return true;
	}

	async syncWidgetsForExecution(appId: string): Promise<void> {
		if (await this.backend.isLocalOnly(appId)) return;
		// Without a session or a reachable hub, the untouched cache is the newest state this
		// device can run; a hub that answers with a refusal still fails the run.
		if (!this.hasRemote()) return;
		try {
			// Do not let fallback reads turn a failed sync into a run with old files.
			const inventory = await this.getWidgetsAuthoritative(appId);
			const widgets = await Promise.all(
				inventory.map(([, widgetId]) =>
					this.getWidgetAuthoritative(appId, widgetId),
				),
			);
			await invoke("cache_widgets", { appId, widgets });
		} catch (error) {
			if (!isHubUnavailable(error)) throw error;
			console.warn(
				"[WidgetState] Hub unreachable, running with cached widgets:",
				error,
			);
		}
	}

	private async pushWidgetRemote(
		appId: string,
		widget: IWidget,
	): Promise<void> {
		if (!this.backend.profile) return;
		const normalizedWidget = normalizeWidgetForPersistence(widget);
		await fetcher(
			this.backend.profile,
			`apps/${appId}/widgets/${widget.id}`,
			{
				method: "PUT",
				body: JSON.stringify({ widget: normalizedWidget }),
			},
			this.getRemoteAuth(),
		);
	}

	private async fetchRemoteWidget(
		appId: string,
		widgetId: string,
		version?: Version,
		timeoutMs?: number,
	): Promise<IWidget> {
		if (!this.backend.profile) {
			throw new Error("Profile not set. Cannot fetch remote widget.");
		}
		const versionQuery = version ? `?version=${version.join("_")}` : "";
		const path = `apps/${appId}/widgets/${widgetId}${versionQuery}`;
		const widget = await fetcher<unknown>(
			this.backend.profile,
			path,
			{ method: "GET", timeoutMs },
			this.getRemoteAuth(),
		);
		// Callers cache and render this payload, so a body that is no widget must not stand in.
		if (!isWidgetPayload(widget))
			throw malformedResponseError(path, "a Widget");
		return widget;
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
			result.push([appId, widget.id, withWidgetName(metadata, widget)]);
		}
		return result;
	}

	async getWidgets(
		appId: string,
		language?: string,
	): Promise<[string, string, IMetadata | undefined][]> {
		const profile = this.backend.profile;
		if (profile && (await this.canFetchRemoteWidget(appId))) {
			try {
				const params = language
					? `?language=${encodeURIComponent(language)}`
					: "";
				const path = `apps/${appId}/widgets${params}`;
				// A local cache entry absent from this inventory may have been deleted
				// on another device. Reading the inventory must never upload it again.
				const remote = await fetcher<[string, string, IMetadata | undefined][]>(
					profile,
					path,
					{ method: "GET" },
					this.getRemoteAuth(),
				);
				if (Array.isArray(remote)) return remote;
				throw malformedResponseError(path, "a Widget list");
			} catch (error) {
				if (isMissingResourceError(error)) throw error;
				console.warn(
					"[WidgetState] Falling back to local widgets list, remote fetch failed:",
					error,
				);
			}
		}

		const localWidgets = await invoke<IWidget[]>("get_widgets", { appId });
		return this.buildListResult(appId, localWidgets, language);
	}

	async getWidgetsAuthoritative(
		appId: string,
		language?: string,
	): Promise<[string, string, IMetadata | undefined][]> {
		if (await this.backend.isLocalOnly(appId)) {
			const widgets = await invoke<IWidget[]>("get_widgets", { appId });
			return widgets.map((widget) => [
				appId,
				widget.id,
				withWidgetName(undefined, widget),
			]);
		}
		if (
			!this.backend.profile ||
			!this.backend.auth?.isAuthenticated ||
			!this.backend.auth.user?.access_token
		) {
			throw new Error(
				"Hosted Widget inventory requires an authenticated hub session",
			);
		}
		const params = language ? `?language=${language}` : "";
		const path = `apps/${appId}/widgets${params}`;
		const inventory = await fetcher<[string, string, IMetadata | undefined][]>(
			this.backend.profile,
			path,
			{ method: "GET" },
			this.backend.auth,
		);
		// An empty inventory would read as "no widgets exist" and clear the execution cache.
		if (!Array.isArray(inventory)) {
			throw malformedResponseError(path, "a Widget list");
		}
		return inventory;
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
			// With a local copy to fall back on, a hung hub must not hold the render.
			const remote = await this.fetchRemoteWidget(
				appId,
				widgetId,
				version,
				local ? HUB_REFRESH_TIMEOUT_MS : undefined,
			);
			if (version && remote.version?.join("_") !== version.join("_")) {
				throw new Error(`Widget version mismatch: ${widgetId}`);
			}
			// Hosted reads follow the server even if a device clock makes its cache
			// look newer. Only explicit edits may write back to the server.
			try {
				await invoke(version ? "cache_widget_version" : "update_widget", {
					appId,
					widget: remote,
				});
			} catch (error) {
				console.warn(
					"[WidgetState] Failed to cache remote widget locally:",
					error,
				);
			}
			return remote;
		} catch (e) {
			if (isMissingResourceError(e)) throw e;
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

	async getWidgetAuthoritative(
		appId: string,
		widgetId: string,
		version?: Version,
	): Promise<IWidget> {
		if (await this.backend.isLocalOnly(appId)) {
			return invoke<IWidget>("get_widget", {
				appId,
				widgetId,
				version,
			});
		}
		if (
			!this.backend.profile ||
			!this.backend.auth?.isAuthenticated ||
			!this.backend.auth.user?.access_token
		) {
			throw new Error(
				"Hosted Widget read requires an authenticated hub session",
			);
		}
		return this.fetchRemoteWidget(appId, widgetId, version);
	}

	async createWidget(
		appId: string,
		widgetId: string,
		name: string,
		description?: string,
	): Promise<IWidget> {
		const writeRemote = await this.requiresRemoteWrite(appId);
		const widget = await invoke<IWidget>("create_widget", {
			appId,
			widgetId,
			name,
			description,
		});

		if (writeRemote) {
			try {
				await this.pushWidgetRemote(appId, widget);
			} catch (e) {
				console.warn("[WidgetState] Failed to push new widget to remote:", e);
				throw e;
			}
		}
		return widget;
	}

	async updateWidget(appId: string, widget: IWidget): Promise<void> {
		const normalizedWidget = normalizeWidgetForPersistence(widget);
		if (await this.requiresRemoteWrite(appId)) {
			try {
				await this.pushWidgetRemote(appId, normalizedWidget);
			} catch (e) {
				console.warn(
					"[WidgetState] Failed to push widget update to remote:",
					e,
				);
				throw e;
			}
		}
		await invoke("update_widget", { appId, widget: normalizedWidget });
	}

	async renameWidget(
		appId: string,
		widgetId: string,
		name: string,
		language?: string,
	): Promise<void> {
		await applyWidgetRename(this, appId, widgetId, name, language);
	}

	async deleteWidget(appId: string, widgetId: string): Promise<void> {
		const writeRemote = await this.requiresRemoteWrite(appId);
		if (writeRemote && this.backend.profile) {
			await fetcher(
				this.backend.profile,
				`apps/${appId}/widgets/${widgetId}`,
				{ method: "DELETE" },
				this.getRemoteAuth(),
			);
		}
		try {
			await invoke("delete_widget", { appId, widgetId });
		} catch (error) {
			if (!writeRemote) throw error;
			console.warn(
				"[WidgetState] Failed to remove deleted widget from cache:",
				error,
			);
		}
	}

	async createWidgetVersion(
		appId: string,
		widgetId: string,
		versionType: VersionType,
	): Promise<Version> {
		if (await this.backend.isLocalOnly(appId)) {
			return invoke<Version>("create_widget_version", {
				appId,
				widgetId,
				versionType,
			});
		}
		const profile = this.backend.profile;
		if (!profile || !this.hasRemote()) {
			throw new Error(
				"Publishing a hosted widget requires an authenticated hub session",
			);
		}

		// Publishing locally and PUTting the working copy never creates a server
		// snapshot. The hub must allocate and persist the shared version itself.
		const version = await fetcher<Version>(
			profile,
			`apps/${appId}/widgets/${widgetId}/versions`,
			{ method: "POST", body: JSON.stringify({ version_type: versionType }) },
			this.getRemoteAuth(),
		);
		try {
			await this.getWidget(appId, widgetId, version);
			await this.getWidget(appId, widgetId);
		} catch (error) {
			// The publish already succeeded. Cache refresh failure must not invite
			// the caller to retry publication and allocate another version.
			console.warn("[WidgetState] Failed to cache published widget:", error);
		}
		return version;
	}

	async getWidgetVersions(appId: string, widgetId: string): Promise<Version[]> {
		const profile = this.backend.profile;
		if (profile && (await this.canFetchRemoteWidget(appId))) {
			try {
				const path = `apps/${appId}/widgets/${widgetId}/versions`;
				const versions = await fetcher<Version[]>(
					profile,
					path,
					{ method: "GET" },
					this.getRemoteAuth(),
				);
				if (Array.isArray(versions)) return versions;
				throw malformedResponseError(path, "a Widget version list");
			} catch (error) {
				if (isMissingResourceError(error)) throw error;
				console.warn(
					"[WidgetState] Falling back to local widget versions:",
					error,
				);
			}
		}
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
		if (this.backend.profile && (await this.canFetchRemoteWidget(appId))) {
			try {
				const path = `apps/${appId}/meta?language=${encodeURIComponent(language ?? "en")}&widget_id=${encodeURIComponent(widgetId)}`;
				const metadata = await fetcher<IMetadata>(
					this.backend.profile,
					path,
					{ method: "GET" },
					this.getRemoteAuth(),
				);
				if (isRecord(metadata)) return metadata;
				throw malformedResponseError(path, "Widget metadata");
			} catch (error) {
				if (isMissingResourceError(error)) throw error;
				console.warn(
					"[WidgetState] Falling back to cached widget metadata:",
					error,
				);
			}
		}
		return invoke<IMetadata>("get_widget_meta", { appId, widgetId, language });
	}

	async pushWidgetMeta(
		appId: string,
		widgetId: string,
		metadata: IMetadata,
		language?: string,
	): Promise<void> {
		if ((await this.requiresRemoteWrite(appId)) && this.backend.profile) {
			await fetcher(
				this.backend.profile,
				`apps/${appId}/meta?language=${encodeURIComponent(language ?? "en")}&widget_id=${encodeURIComponent(widgetId)}`,
				{ method: "PUT", body: JSON.stringify(metadata) },
				this.getRemoteAuth(),
			);
		}
		return invoke("push_widget_meta", { appId, widgetId, metadata, language });
	}
}
