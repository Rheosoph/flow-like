import {
	type IBoard,
	type IMetadata,
	type ITemplatePreview,
	type ITemplateSearchHit,
	type ITemplateSearchQuery,
	type ITemplateState,
	type IVersionType,
	injectDataFunction,
} from "@flow-like/flow-like-ui";
import { isRecord } from "@flow-like/flow-like-ui/lib/response-shape";
import {
	MAX_OWNED_TEMPLATE_METADATA,
	type TemplateReadOptions,
	readTemplateMetadataSnapshot,
} from "@flow-like/flow-like-ui/state/backend-state/template-read";
import { invoke } from "@tauri-apps/api/core";
import { isEqual } from "lodash-es";
import { type FetcherOptions, fetcher } from "../../lib/api";
import type { TauriBackend } from "../tauri-provider";

type RemoteTemplateEntry = [string, string, IMetadata];

function shapeOf(value: unknown): string {
	if (value === null) return "null";
	return Array.isArray(value) ? "array" : typeof value;
}

function expectArray(value: unknown, route: string): unknown[] {
	if (Array.isArray(value)) return value;
	throw new Error(`${route} returned ${shapeOf(value)} instead of an array`);
}

function expectRecord<T>(value: T, route: string): T {
	if (isRecord(value)) return value;
	throw new Error(`${route} returned ${shapeOf(value)} instead of an object`);
}

function isRemoteTemplateEntry(entry: unknown): entry is RemoteTemplateEntry {
	return (
		Array.isArray(entry) &&
		typeof entry[0] === "string" &&
		typeof entry[1] === "string" &&
		isRecord(entry[2])
	);
}

/** A garbled listing is an error, never an empty one: callers merge it into local state. */
async function fetchRemoteTemplateEntries(
	backend: TauriBackend,
	route: string,
	options?: FetcherOptions,
): Promise<RemoteTemplateEntry[]> {
	if (!backend.profile) throw new Error(`No profile set to fetch ${route}`);
	const response = await fetcher<unknown>(
		backend.profile,
		route,
		options,
		backend.auth,
	);
	return expectArray(response, route).filter(isRemoteTemplateEntry);
}

export class TemplateState implements ITemplateState {
	constructor(private readonly backend: TauriBackend) {}

	/**
	 * Store-wide template search is a remote-only surface: local templates come
	 * from `getTemplates`, which already walks every app in the profile.
	 */
	async searchTemplates(
		query: ITemplateSearchQuery,
		options?: TemplateReadOptions,
	): Promise<ITemplateSearchHit[]> {
		if (!this.backend.profile) {
			if (options?.strict)
				throw new Error("Profile not set. Cannot search public templates.");
			return [];
		}

		const params = new URLSearchParams();
		params.set("query", query.query);
		if (query.language) params.set("language", query.language);
		if (query.category) params.set("category", query.category);
		if (query.tag) params.set("tag", query.tag);
		if (query.forkable_only) params.set("forkable_only", "true");
		if (query.limit !== undefined) params.set("limit", String(query.limit));
		if (query.offset !== undefined) params.set("offset", String(query.offset));

		try {
			const route = `apps/templates/search?${params}`;
			const hits = await fetcher<unknown>(this.backend.profile, route, {
				method: "GET",
			});
			return expectArray(hits, route) as ITemplateSearchHit[];
		} catch (error) {
			if (options?.strict) throw error;
			return [];
		}
	}

	async getTemplatePreview(
		appId: string,
		templateId: string,
	): Promise<ITemplatePreview> {
		if (!this.backend.profile) {
			throw new Error("Profile not set. Cannot preview a template.");
		}
		const route = `apps/${appId}/templates/${templateId}/preview`;
		return expectRecord(
			await fetcher<ITemplatePreview>(this.backend.profile, route, {
				method: "GET",
			}),
			route,
		);
	}
	async getTemplates(
		appId?: string,
		language?: string,
		options?: TemplateReadOptions,
	): Promise<[string, string, IMetadata | undefined][]> {
		if (options?.readOnly) {
			const profile = this.backend.profile;
			const remote =
				profile && (!appId || !(await this.backend.isOffline(appId)));
			const params = new URLSearchParams({ limit: "100", offset: "0" });
			if (language) params.set("language", language);
			return readTemplateMetadataSnapshot(
				[
					{
						label: "Local template metadata",
						read: () =>
							invoke("get_templates", {
								appId,
								language,
								metadataOnly: true,
								limit: MAX_OWNED_TEMPLATE_METADATA,
							}),
					},
					...(profile && remote
						? [
								{
									label: "Owned remote template metadata",
									read: () =>
										fetchRemoteTemplateEntries(
											this.backend,
											appId
												? `apps/${appId}/templates?${params}`
												: `user/templates?${params}`,
											{ method: "GET" },
										),
								},
							]
						: []),
				],
				options,
				{
					complete: false,
					scope: remote
						? "local_cache_and_owned_remote_metadata"
						: "local_cached_template_metadata",
					warning:
						remote && !appId
							? "Remote metadata covers the first 100 memberships without exhaustion metadata. Local metadata covers at most 100 profile apps and 1000 template IDs and may omit unavailable records."
							: "Local metadata covers at most 100 profile apps and 1000 template IDs and may omit unavailable records, so it cannot certify exhaustive coverage.",
				},
			);
		}
		const templates = await invoke<[string, string, IMetadata | undefined][]>(
			"get_templates",
			{
				appId: appId,
				language: language,
			},
		);

		if (appId) {
			const isOffline = await this.backend.isOffline(appId);
			if (isOffline) {
				return templates;
			}

			if (!this.backend.profile || !this.backend.queryClient) {
				console.warn(
					"No profile set for Tauri backend, returning local templates",
				);
				return templates;
			}

			const promise = injectDataFunction(
				async () => {
					const remoteData = await fetchRemoteTemplateEntries(
						this.backend,
						`apps/${appId}/templates`,
					);

					const mergedData = new Map<string, [string, string, IMetadata]>();

					for (const [id, templateId, meta] of templates) {
						const key = `${id}:${templateId}`;
						if (!mergedData.has(key) && meta) {
							mergedData.set(key, [id, templateId, meta]);
						}
					}

					for (const [appId, templateId, metadata] of remoteData) {
						const key = `${appId}:${templateId}`;
						const found = mergedData.get(key);
						if (found) {
							if (isEqual(found[2], metadata)) {
								// If metadata is the same, skip adding it again
								continue;
							}
						}
						mergedData.set(key, [appId, templateId, metadata]);
						await invoke("push_template_meta", {
							appId: appId,
							templateId: templateId,
							metadata: metadata,
						});
						await this.getTemplate(appId, templateId);
					}

					return Array.from(mergedData.values());
				},
				this,
				this.backend.queryClient,
				this.getTemplates,
				[appId, language],
				[],
				templates,
			);
			this.backend.backgroundTaskHandler(promise);

			return templates;
		}

		if (!this.backend.profile || !this.backend.queryClient) {
			return templates;
		}

		// Local templates render first; a slow or hung hub only delays the merge.
		const promise = injectDataFunction(
			async () => {
				const limit = 100;
				let offset = 0;
				let foundAmount = 0;
				const mergedData = new Map<string, [string, string, IMetadata]>();
				for (const [id, templateId, meta] of templates) {
					const key = `${id}:${templateId}`;
					if (!mergedData.has(key) && meta) {
						mergedData.set(key, [id, templateId, meta]);
					}
				}

				try {
					do {
						const remoteData = await fetchRemoteTemplateEntries(
							this.backend,
							`user/templates?limit=${limit}&offset=${offset}`,
						);

						foundAmount = remoteData.length;
						offset += 100;

						for (const [appId, templateId, metadata] of remoteData) {
							const key = `${appId}:${templateId}`;
							const found = mergedData.get(key);
							if (found) {
								if (isEqual(found[2], metadata)) {
									// If metadata is the same, skip adding it again
									continue;
								}
							}
							mergedData.set(key, [appId, templateId, metadata]);
							await invoke("push_template_meta", {
								appId: appId,
								templateId: templateId,
								metadata: metadata,
							});
						}
					} while (foundAmount > 0);
				} catch (error) {
					console.error("Failed to fetch templates from remote:", error);
				}

				return Array.from(mergedData.values());
			},
			this,
			this.backend.queryClient,
			this.getTemplates,
			[appId, language],
			[],
			templates,
		);
		this.backend.backgroundTaskHandler(promise);

		return templates;
	}

	async getTemplate(
		appId: string,
		templateId: string,
		version?: [number, number, number],
	): Promise<IBoard> {
		let template: IBoard | undefined = undefined;
		const isOffline = await this.backend.isOffline(appId);

		try {
			template = await invoke<IBoard>("get_template", {
				appId: appId,
				templateId: templateId,
				version: version,
			});
			if (isOffline) {
				return template;
			}
		} catch (error) {
			console.error("Error fetching template:", error);
			if (await this.backend.isLocalOnly(appId)) throw error;
		}

		if (!this.backend.profile || !this.backend.queryClient) {
			if (template) {
				return template;
			}
			throw new Error(
				"No profile set for Tauri backend and no local template available",
			);
		}

		const profile = this.backend.profile;
		const route = `apps/${appId}/templates/${templateId}`;
		if (template) {
			const promise = injectDataFunction(
				async () => {
					const remoteData = expectRecord(
						await fetcher<IBoard>(profile, route, undefined, this.backend.auth),
						route,
					);

					if (!isEqual(template, remoteData)) {
						await invoke("push_template_data", {
							appId: appId,
							templateId: templateId,
							data: remoteData,
							version: version,
						});

						return remoteData;
					}

					return template;
				},
				this,
				this.backend.queryClient,
				this.getTemplate,
				[appId, templateId, version],
				[],
				template,
			);
			this.backend.backgroundTaskHandler(promise);

			return template;
		}

		try {
			const remoteData = expectRecord(
				await fetcher<IBoard>(profile, route, undefined, this.backend.auth),
				route,
			);

			await invoke("push_template_data", {
				appId: appId,
				templateId: templateId,
				data: remoteData,
				version: version,
			});

			return remoteData;
		} catch (error) {
			console.error("Failed to fetch template from remote:", error);
			if (template) {
				return template;
			}
			throw new Error(
				"Failed to fetch template: no local cache available and remote fetch failed.",
			);
		}
	}

	async upsertTemplate(
		appId: string,
		boardId: string,
		templateId?: string,
		boardVersion?: [number, number, number],
		versionType?: IVersionType,
	): Promise<[string, [number, number, number]]> {
		const isOffline = await this.backend.isOffline(appId);

		if (isOffline) {
			return await invoke("upsert_template", {
				appId: appId,
				boardId: boardId,
				templateId: templateId,
				boardVersion: boardVersion,
				versionType: versionType,
			});
		}

		if (!this.backend.profile || !this.backend.queryClient) {
			throw new Error("No profile set for Tauri backend");
		}

		const route = `apps/${appId}/templates/${templateId ?? "new"}`;
		const result = await fetcher<[string, [number, number, number]]>(
			this.backend.profile,
			route,
			{
				method: "PUT",
				body: JSON.stringify({
					board_id: boardId,
					board_version: boardVersion,
					version_type: versionType,
				}),
			},
			this.backend.auth,
		);
		// Callers key the new template's metadata by result[0].
		if (!Array.isArray(result) || typeof result[0] !== "string") {
			throw new Error(
				`${route} returned ${shapeOf(result)} instead of [templateId, version]`,
			);
		}

		await invoke("upsert_template", {
			appId: appId,
			boardId: boardId,
			templateId: templateId,
			boardVersion: boardVersion,
			versionType: versionType,
		});

		return result;
	}

	async deleteTemplate(appId: string, templateId: string): Promise<void> {
		const isOffline = await this.backend.isOffline(appId);

		if (isOffline) {
			await invoke("delete_template", {
				appId: appId,
				templateId: templateId,
			});
			return;
		}

		if (!this.backend.profile || !this.backend.queryClient) {
			throw new Error("No profile set for Tauri backend");
		}

		await fetcher(
			this.backend.profile,
			`apps/${appId}/templates/${templateId}`,
			{
				method: "DELETE",
			},
			this.backend.auth,
		);

		await invoke("delete_template", {
			appId: appId,
			templateId: templateId,
		});
	}

	async getTemplateMeta(
		appId: string,
		templateId: string,
		language?: string,
	): Promise<IMetadata> {
		const isOffline = await this.backend.isOffline(appId);

		let meta: IMetadata | undefined = undefined;

		try {
			meta = await invoke<IMetadata>("get_template_meta", {
				appId: appId,
				templateId: templateId,
				language: language,
			});
			if (isOffline) {
				return meta;
			}
		} catch (error) {
			console.error("Error fetching template meta:", error);
			if (isOffline) {
				throw new Error(
					"Cannot fetch template meta while offline. Please try again later.",
				);
			}
		}

		if (
			!this.backend.profile ||
			!this.backend.auth ||
			!this.backend.queryClient
		) {
			if (meta) {
				return meta;
			}
			throw new Error(
				"Profile, auth or query client not set. Cannot get template meta.",
			);
		}

		const profile = this.backend.profile;
		const route = `apps/${appId}/meta?language=${language ?? "en"}&template_id=${templateId}`;
		if (meta) {
			const promise = injectDataFunction(
				async () => {
					const remoteMeta = expectRecord(
						await fetcher<IMetadata>(
							profile,
							route,
							undefined,
							this.backend.auth,
						),
						route,
					);

					await invoke("push_template_meta", {
						appId: appId,
						templateId: templateId,
						metadata: remoteMeta,
						language,
					});

					return remoteMeta;
				},
				this,
				this.backend.queryClient,
				this.getTemplateMeta,
				[appId, templateId, language],
				[],
				meta,
			);
			this.backend.backgroundTaskHandler(promise);

			return meta;
		}

		try {
			const remoteMeta = expectRecord(
				await fetcher<IMetadata>(profile, route, undefined, this.backend.auth),
				route,
			);

			await invoke("push_template_meta", {
				appId: appId,
				templateId: templateId,
				metadata: remoteMeta,
				language,
			});

			return remoteMeta;
		} catch (error) {
			console.error("Failed to fetch template meta from remote:", error);
			if (meta) {
				return meta;
			}
			throw new Error(
				"Failed to fetch template meta: no local cache available and remote fetch failed.",
			);
		}
	}

	async pushTemplateMeta(
		appId: string,
		templateId: string,
		metadata: IMetadata,
		language?: string,
	): Promise<void> {
		const isOffline = await this.backend.isOffline(appId);

		await invoke("push_template_meta", {
			appId: appId,
			templateId: templateId,
			metadata: metadata,
			language: language,
		});

		if (isOffline) {
			return;
		}

		if (
			!this.backend.profile ||
			!this.backend.auth ||
			!this.backend.queryClient
		) {
			throw new Error(
				"Profile, auth or query client not set. Cannot push app meta.",
			);
		}
		await fetcher(
			this.backend.profile,
			`apps/${appId}/meta?language=${language ?? "en"}&template_id=${templateId}`,
			{
				method: "PUT",
				body: JSON.stringify(metadata),
			},
			this.backend.auth,
		);

		await invoke("push_template_meta", {
			appId: appId,
			templateId: templateId,
			metadata: metadata,
			language: language,
		});
	}
}
