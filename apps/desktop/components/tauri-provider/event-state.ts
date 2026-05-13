import { Channel, invoke } from "@tauri-apps/api/core";
import {
	type IBoard,
	type IEvent,
	type IEventState,
	type IHub,
	type IIntercomEvent,
	type ILogMetadata,
	type INode,
	type IOAuthProvider,
	type IOAuthToken,
	type IPrerunEventResponse,
	type IRunPayload,
	type IVersionType,
	type ProgressToastData,
	checkOAuthTokens,
	extractOAuthRequirementsFromBoard,
	finishAllProgressToasts,
	injectDataFunction,
	isEqual,
	showProgressToast,
} from "@tm9657/flow-like-ui";
import { toast } from "sonner";
import { fetcher, streamFetcher } from "../../lib/api";
import {
	dispatchFlowNotificationEvent,
	dispatchFlowNotificationEvents,
} from "../../lib/flow-notification-events";
import { oauthConsentStore, oauthTokenStore } from "../../lib/oauth-db";
import { oauthService } from "../../lib/oauth-service";
import {
	ensureRpaSystemPermissions,
	requestRpaAutomationConsent,
} from "../rpa";
import type { TauriBackend } from "../tauri-provider";
import { resolveLocalFirstPrerun } from "./prerun-utils";

// Hub configuration cache (shared with board-state)
let hubCache: IHub | undefined;
let hubCachePromise: Promise<IHub | undefined> | undefined;

async function getHubConfig(profile?: { hub?: string }): Promise<
	IHub | undefined
> {
	if (hubCache) return hubCache;
	if (hubCachePromise) return hubCachePromise;

	const hubUrl = profile?.hub;
	if (!hubUrl) return undefined;

	hubCachePromise = fetch(`https://${hubUrl}/api/v1`)
		.then((res) => res.json() as Promise<IHub>)
		.then((hub) => {
			hubCache = hub;
			return hub;
		})
		.catch((e) => {
			console.warn("[OAuth] Failed to fetch Hub config:", e);
			return undefined;
		});

	return hubCachePromise;
}

export class EventState implements IEventState {
	constructor(private readonly backend: TauriBackend) {}

	private async ensureRpaApprovalForEvent(
		appId: string,
		event: IEvent,
		board: IBoard,
		context: "execution" | "event_registration",
	): Promise<void> {
		if (event.execution_mode === "Remote") return;
		if (context === "event_registration" && event.active === false) return;

		const { requires_local_execution } =
			extractOAuthRequirementsFromBoard(board);
		if (!requires_local_execution) return;

		const approved = await requestRpaAutomationConsent({
			appId,
			boardId: event.board_id,
			context,
			eventId: event.id,
		});
		if (!approved) {
			const error = new Error(
				"Computer automation was not approved for this event.",
			) as Error & { isRpaConsentError?: boolean };
			error.isRpaConsentError = true;
			throw error;
		}

		const permissionsGranted = await ensureRpaSystemPermissions({
			appId,
			boardId: event.board_id,
			eventId: event.id,
		});
		if (!permissionsGranted) {
			const error = new Error(
				"RPA system permissions were not granted.",
			) as Error & { isRpaPermissionDeclined?: boolean };
			error.isRpaPermissionDeclined = true;
			throw error;
		}
	}

	async getEvent(
		appId: string,
		eventId: string,
		version?: [number, number, number],
	): Promise<IEvent> {
		let event: IEvent;
		try {
			event = await invoke<IEvent>("get_event", {
				appId: appId,
				eventId: eventId,
				version: version,
			});
		} catch {
			const isOffline = await this.backend.isOffline(appId);
			if (isOffline || !this.backend.profile || !this.backend.auth) {
				throw new Error(`Event not found: ${eventId}`);
			}
			let url = `apps/${appId}/events/${eventId}`;
			if (version) {
				url += `?version=${version.join("_")}`;
			}
			const remoteData = await fetcher<IEvent>(
				this.backend.profile,
				url,
				{ method: "GET" },
				this.backend.auth,
			);
			await invoke("upsert_event", {
				appId: appId,
				event: remoteData,
				enforceId: true,
				offline: false,
			}).catch(() => {});
			return remoteData;
		}

		const isOffline = await this.backend.isOffline(appId);
		if (
			isOffline ||
			!this.backend.profile ||
			!this.backend.auth ||
			!this.backend.queryClient
		) {
			return event;
		}

		const promise = injectDataFunction(
			async () => {
				let url = `apps/${appId}/events/${eventId}`;
				if (version) {
					url += `?version=${version.join("_")}`;
				}
				const remoteData = await fetcher<IEvent>(
					this.backend.profile!,
					url,
					{
						method: "GET",
					},
					this.backend.auth,
				);

				if (!remoteData) {
					throw new Error("Failed to fetch event data");
				}

				if (!isEqual(remoteData, event) && typeof version === "undefined") {
					await invoke("upsert_event", {
						appId: appId,
						event: remoteData,
						enforceId: true,
						offline: isOffline,
					});
				}

				return remoteData;
			},
			this,
			this.backend.queryClient,
			this.getEvent,
			[appId, eventId, version],
			[],
			event,
		);

		this.backend.backgroundTaskHandler(promise);
		return event;
	}
	async getEvents(appId: string, force?: boolean): Promise<IEvent[]> {
		const events = await invoke<IEvent[]>("get_events", {
			appId: appId,
		});
		const isOffline = await this.backend.isOffline(appId);
		if (
			isOffline ||
			!this.backend.profile ||
			!this.backend.auth ||
			!this.backend.queryClient
		) {
			return events;
		}

		const syncRemote = async () => {
			const remoteData = await fetcher<IEvent[]>(
				this.backend.profile!,
				`apps/${appId}/events`,
				{
					method: "GET",
				},
				this.backend.auth,
			);

			for (const event of remoteData) {
				await invoke("upsert_event", {
					appId: appId,
					event: event,
					enforceId: true,
					offline: isOffline,
				});
			}

			return remoteData;
		};

		if (force) {
			try {
				const remoteData = await syncRemote();
				const queryKey = [this.getEvents.name || "backendFn", appId, true];
				this.backend.queryClient.setQueryData(queryKey, remoteData);
				return remoteData;
			} catch (error) {
				if (events.length === 0) throw error;
				console.warn(
					"[EventSync] Forced event fetch failed, falling back to local events:",
					error,
				);
				return events;
			}
		}

		if (events.length === 0) {
			try {
				const remoteData = await syncRemote();
				const queryKey = [this.getEvents.name || "backendFn", appId];
				this.backend.queryClient.setQueryData(queryKey, remoteData);
				return remoteData;
			} catch {
				return events;
			}
		}

		const promise = injectDataFunction(
			syncRemote,
			this,
			this.backend.queryClient,
			this.getEvents,
			[appId],
			[],
			events,
		);

		this.backend.backgroundTaskHandler(promise);
		return events;
	}
	async getEventVersions(
		appId: string,
		eventId: string,
	): Promise<[number, number, number][]> {
		const versions = await invoke<[number, number, number][]>(
			"get_event_versions",
			{
				appId: appId,
				eventId: eventId,
			},
		);

		const isOffline = await this.backend.isOffline(appId);
		if (
			isOffline ||
			!this.backend.profile ||
			!this.backend.auth ||
			!this.backend.queryClient
		) {
			return versions;
		}

		const promise = injectDataFunction(
			async () => {
				const remoteData = await fetcher<[number, number, number][]>(
					this.backend.profile!,
					`apps/${appId}/events/${eventId}/versions`,
					{
						method: "GET",
					},
					this.backend.auth,
				);

				return remoteData;
			},
			this,
			this.backend.queryClient,
			this.getEventVersions,
			[appId, eventId],
			[],
			versions,
		);

		this.backend.backgroundTaskHandler(promise);
		return versions;
	}
	async upsertEvent(
		appId: string,
		event: IEvent,
		versionType?: IVersionType,
		personalAccessToken?: string,
		oauthTokens?: Record<string, IOAuthToken>,
	): Promise<IEvent> {
		if (event.board_id && event.execution_mode !== "Remote") {
			const board = await this.backend.boardState.getBoard(
				appId,
				event.board_id,
				event.board_version as [number, number, number] | undefined,
				true,
			);
			await this.ensureRpaApprovalForEvent(
				appId,
				event,
				board,
				"event_registration",
			);
		}

		const isOffline = await this.backend.isOffline(appId);
		if (isOffline) {
			return await invoke("upsert_event", {
				appId: appId,
				event: event,
				versionType: versionType,
				offline: isOffline,
				pat: personalAccessToken,
				oauthTokens: oauthTokens,
			});
		}
		if (
			!this.backend.profile ||
			!this.backend.auth ||
			!this.backend.queryClient
		) {
			throw new Error(
				"Profile, auth or query client not set. Cannot upsert event.",
			);
		}
		const response = await fetcher<IEvent>(
			this.backend.profile,
			`apps/${appId}/events/${event.id}`,
			{
				method: "PUT",
				body: JSON.stringify({
					event: event,
					version_type: versionType,
					profile_id: this.backend.profile.id,
					pat: personalAccessToken,
					oauth_tokens: oauthTokens,
				}),
			},
			this.backend.auth,
		);
		await invoke("upsert_event", {
			appId: appId,
			event: response,
			versionType: versionType,
			enforceId: true,
			offline: isOffline,
			pat: personalAccessToken,
			oauthTokens: oauthTokens,
		});
		return response;
	}
	async deleteEvent(appId: string, eventId: string): Promise<void> {
		const isOffline = await this.backend.isOffline(appId);

		if (!isOffline) {
			if (
				!this.backend.profile ||
				!this.backend.auth ||
				!this.backend.queryClient
			) {
				throw new Error(
					"Profile, auth or query client not set. Cannot delete event.",
				);
			}

			await fetcher(
				this.backend.profile,
				`apps/${appId}/events/${eventId}`,
				{
					method: "DELETE",
				},
				this.backend.auth,
			);
		}

		try {
			await invoke("delete_event", {
				appId: appId,
				eventId: eventId,
			});
		} catch (e) {
			if (isOffline) throw e;
			console.warn("[EventState] Local event deletion failed (non-fatal):", e);
		}
	}
	async validateEvent(
		appId: string,
		eventId: string,
		version?: [number, number, number],
	): Promise<void> {
		const isOffline = await this.backend.isOffline(appId);
		if (isOffline) {
			return await invoke("validate_event", {
				appId: appId,
				eventId: eventId,
				version: version,
			});
		}

		if (
			!this.backend.profile ||
			!this.backend.auth ||
			!this.backend.queryClient
		) {
			throw new Error(
				"Profile, auth or query client not set. Cannot validate event.",
			);
		}

		return await fetcher(
			this.backend.profile,
			`apps/${appId}/events/${eventId}/validate`,
			{
				method: "POST",
				body: JSON.stringify({
					version: version,
				}),
			},
			this.backend.auth,
		);
	}
	async upsertEventFeedback(
		appId: string,
		eventId: string,
		feedbackId: string,
		feedback: {
			rating: number;
			history?: any[];
			globalState?: Record<string, any>;
			localState?: Record<string, any>;
			comment?: string;
		},
	): Promise<string> {
		const isOffline = await this.backend.isOffline(appId);
		if (isOffline) {
			try {
				const now = Math.floor(Date.now() / 1000);
				await invoke("upsert_offline_feedback", {
					appId,
					feedback: {
						id: feedbackId,
						app_id: appId,
						event_id: eventId,
						message_id: feedbackId,
						session_id: "",
						rating: feedback.rating,
						comment: feedback.comment ?? null,
						include_chat_history: !!feedback.history,
						can_contact: false,
						created_at: now,
						updated_at: now,
					},
				});
			} catch (e) {
				console.warn("[Feedback] Failed to save offline feedback:", e);
			}
			return feedbackId;
		}

		if (
			!this.backend.profile ||
			!this.backend.auth ||
			!this.backend.queryClient
		) {
			throw new Error(
				"Profile, auth or query client not set. Cannot upsert event feedback.",
			);
		}

		const response = await fetcher<{ feedback_id: string }>(
			this.backend.profile,
			`apps/${appId}/events/${eventId}/feedback`,
			{
				method: "PUT",
				body: JSON.stringify({
					rating: feedback.rating,
					context: {
						history: feedback.history,
						global_state: feedback.globalState,
						local_state: feedback.localState,
					},
					comment: feedback.comment ?? "",
					feedback_id: feedbackId,
				}),
			},
			this.backend.auth,
		);

		return response.feedback_id;
	}

	async executeEvent(
		appId: string,
		eventId: string,
		payload: IRunPayload,
		streamState?: boolean,
		onEventId?: (id: string) => void,
		cb?: (event: IIntercomEvent[]) => void,
		skipConsentCheck?: boolean,
	): Promise<ILogMetadata | undefined> {
		const channel = new Channel<IIntercomEvent[]>();
		let closed = false;
		let foundRunId = false;

		const isOffline = await this.backend.isOffline(appId);
		let credentials = undefined;

		if (!isOffline && this.backend.auth && this.backend.profile) {
			try {
				credentials = await fetcher(
					this.backend.profile,
					`apps/${appId}/invoke/presign`,
					{
						method: "GET",
					},
					this.backend.auth,
				);
			} catch (e) {
				console.warn(e);
			}
		}

		// Collect OAuth tokens from event's board using shared helper
		let oauthTokens:
			| Record<
					string,
					{
						access_token: string;
						refresh_token?: string;
						expires_at?: number;
						token_type?: string;
					}
			  >
			| undefined;
		const event = await this.getEvent(appId, eventId);
		const board: IBoard = await this.backend.boardState.getBoard(
			appId,
			event.board_id,
			(event.board_version as [number, number, number]) ?? undefined,
			true,
		);
		await this.ensureRpaApprovalForEvent(appId, event, board, "execution");
		const hub = await getHubConfig(this.backend.profile);
		const oauthResult = await checkOAuthTokens(board, oauthTokenStore, hub, {
			refreshToken: oauthService.refreshToken.bind(oauthService),
		});

		// Check consent for providers that have tokens but might not have consent for this app
		const consentedIds = await oauthConsentStore.getConsentedProviderIds(appId);
		const providersNeedingConsent: IOAuthProvider[] = [];

		// Add providers that are missing tokens
		providersNeedingConsent.push(...oauthResult.missingProviders);

		// Also add providers that have tokens but no consent for this specific app
		for (const provider of oauthResult.requiredProviders) {
			const hasToken = oauthResult.tokens[provider.id] !== undefined;
			const hasConsent = consentedIds.has(provider.id);

			if (hasToken && !hasConsent) {
				console.log(
					`[OAuth] Provider ${provider.id} has token but no consent for app ${appId}`,
				);
				providersNeedingConsent.push(provider);
			}
		}

		if (providersNeedingConsent.length > 0 && !skipConsentCheck) {
			const error = new Error(
				`Missing OAuth authorization for: ${providersNeedingConsent.map((p) => p.name).join(", ")}`,
			);
			(error as any).missingProviders = providersNeedingConsent;
			(error as any).isOAuthError = true;
			throw error;
		}

		if (Object.keys(oauthResult.tokens).length > 0) {
			oauthTokens = oauthResult.tokens;
		}

		channel.onmessage = (events: IIntercomEvent[]) => {
			if (closed) {
				console.warn("Channel closed, ignoring events");
				return;
			}

			if (!foundRunId && events.length > 0 && eventId) {
				const runId_event = events.find(
					(event) => event.event_type === "run_initiated",
				);

				if (runId_event) {
					const runId = runId_event.payload.run_id;
					onEventId?.(runId);
					foundRunId = true;
				}
			}

			dispatchFlowNotificationEvents(events, appId);

			if (cb) cb(events);
		};

		const token = this.backend.auth?.user?.access_token;
		console.log("Using token:", token);

		const metadata: ILogMetadata | undefined = await invoke("execute_event", {
			appId: appId,
			eventId: eventId,
			payload: payload,
			events: channel,
			streamState: streamState,
			credentials,
			token,
			oauthTokens,
		});

		closed = true;

		return metadata;
	}

	async executeEventRemote(
		appId: string,
		eventId: string,
		payload: IRunPayload,
		streamState?: boolean,
		onEventId?: (id: string) => void,
		cb?: (event: IIntercomEvent[]) => void,
	): Promise<ILogMetadata | undefined> {
		if (!this.backend.profile || !this.backend.auth) {
			throw new Error("Profile and auth required for remote execution");
		}

		let closed = false;
		let foundRunId = false;

		await streamFetcher<IIntercomEvent>(
			this.backend.profile,
			`apps/${appId}/events/${eventId}/invoke`,
			{
				method: "POST",
				body: JSON.stringify({
					payload: payload.payload,
					token: this.backend.auth.user?.access_token,
					stream_state: streamState ?? false,
					oauth_tokens: undefined,
					runtime_variables: payload.runtime_variables,
					profile_id: this.backend.profile?.id,
				}),
			},
			this.backend.auth,
			(event: IIntercomEvent) => {
				if (closed) return;

				if (!foundRunId && onEventId && event.event_type === "run_initiated") {
					const runId = (event.payload as { run_id?: string })?.run_id;
					if (runId) {
						onEventId(runId);
						foundRunId = true;
					}
				}

				if (event.event_type === "toast") {
					const payload = event.payload as {
						message: string;
						level: "success" | "error" | "info" | "warning";
					};
					if (payload?.message) {
						switch (payload.level) {
							case "success":
								toast.success(payload.message);
								break;
							case "error":
								toast.error(payload.message);
								break;
							case "warning":
								toast.warning(payload.message);
								break;
							default:
								toast.info(payload.message);
						}
					}
				}

				if (event.event_type === "progress") {
					showProgressToast(event.payload as ProgressToastData);
				}

				if (event.event_type === "flow_notification") {
					dispatchFlowNotificationEvent(event, appId);
				}

				if (event.event_type === "completed") {
					finishAllProgressToasts(true);
				} else if (event.event_type === "error") {
					finishAllProgressToasts(false);
				}

				if (cb) cb([event]);
			},
		);

		closed = true;
		finishAllProgressToasts(true);
		return undefined;
	}

	async cancelExecution(runId: string): Promise<void> {
		await invoke("cancel_execution", {
			runId: runId,
		});
	}

	async isEventSinkActive(eventId: string): Promise<boolean> {
		return await invoke<boolean>("is_event_sink_active", {
			eventId: eventId,
		});
	}

	async checkEventOAuth(
		appId: string,
		event: IEvent,
	): Promise<{
		tokens?: Record<string, IOAuthToken>;
		missingProviders: IOAuthProvider[];
	}> {
		// Get the board for this event
		const board: IBoard = await invoke("get_board", {
			appId: appId,
			boardId: event.board_id,
			version: event.board_version,
		});

		const hub = await getHubConfig(this.backend.profile);
		const oauthResult = await checkOAuthTokens(board, oauthTokenStore, hub, {
			refreshToken: oauthService.refreshToken.bind(oauthService),
		});

		console.log("[checkEventOAuth] oauthResult:", {
			requiredProviders: oauthResult.requiredProviders?.map((p) => p.id),
			missingProviders: oauthResult.missingProviders?.map((p) => p.id),
			tokens: Object.keys(oauthResult.tokens || {}),
		});

		// Check consent for providers that have tokens but might not have consent for this app
		const consentedIds = await oauthConsentStore.getConsentedProviderIds(appId);
		console.log("[checkEventOAuth] consentedIds:", [...consentedIds]);
		const providersNeedingConsent: IOAuthProvider[] = [];

		// Add providers that are missing tokens
		providersNeedingConsent.push(...oauthResult.missingProviders);

		// Also add providers that have tokens but no consent for this specific app
		for (const provider of oauthResult.requiredProviders) {
			const hasToken = oauthResult.tokens[provider.id] !== undefined;
			const hasConsent = consentedIds.has(provider.id);

			if (hasToken && !hasConsent) {
				providersNeedingConsent.push(provider);
			}
		}

		if (providersNeedingConsent.length > 0) {
			return {
				tokens: undefined,
				missingProviders: providersNeedingConsent,
			};
		}

		return {
			tokens:
				Object.keys(oauthResult.tokens).length > 0
					? oauthResult.tokens
					: undefined,
			missingProviders: [],
		};
	}

	async prerunEvent(
		appId: string,
		eventId: string,
		version?: [number, number, number],
	): Promise<IPrerunEventResponse> {
		const isOffline = await this.backend.isOffline(appId);

		// Helper to build prerun response from local event/board
		const buildLocalPrerun = async (): Promise<IPrerunEventResponse> => {
			const event: IEvent = await invoke("get_event", {
				appId,
				eventId,
				version,
			});
			const board: IBoard = await invoke("get_board", {
				appId,
				boardId: event.board_id,
				version: event.board_version,
			});

			const runtimeVariables = Object.values(board.variables)
				.filter((v) => v.runtime_configured)
				.map((v) => ({
					id: v.id,
					name: v.name,
					description: v.description ?? undefined,
					data_type: v.data_type,
					value_type: v.value_type,
					secret: v.secret,
					schema: v.schema ?? undefined,
				}));

			const {
				oauth_requirements,
				requires_local_execution,
				execution_mode,
				can_execute_locally,
			} = extractOAuthRequirementsFromBoard(board);

			// Collect all WASM (external) node package_ids and permissions
			const wasmPackageIds = new Set<string>();
			const wasmPackagePermissions: Record<string, string[]> = {};
			const collectWasm = (node: INode) => {
				if (node.wasm?.package_id) {
					wasmPackageIds.add(node.wasm.package_id);
					if (node.wasm.permissions?.length) {
						const existing = wasmPackagePermissions[node.wasm.package_id] ?? [];
						for (const perm of node.wasm.permissions) {
							if (!existing.includes(perm)) existing.push(perm);
						}
						wasmPackagePermissions[node.wasm.package_id] = existing;
					}
				}
			};
			for (const node of Object.values(board.nodes)) collectWasm(node);
			for (const layer of Object.values(board.layers)) {
				for (const node of Object.values(layer.nodes)) collectWasm(node);
			}

			return {
				board_id: event.board_id,
				runtime_variables: runtimeVariables,
				oauth_requirements,
				requires_local_execution,
				execution_mode,
				can_execute_locally,
				has_wasm_nodes: wasmPackageIds.size > 0,
				wasm_package_ids: Array.from(wasmPackageIds),
				wasm_package_permissions: wasmPackagePermissions,
			};
		};

		// Offline apps: always use local data
		if (isOffline) {
			return buildLocalPrerun();
		}

		return resolveLocalFirstPrerun({
			label: "prerunEvent",
			buildLocal: buildLocalPrerun,
			fetchRemote:
				this.backend.profile && this.backend.auth
					? async () => {
							let url = `apps/${appId}/events/${eventId}/prerun`;
							if (version) {
								url += `?version=${version.join("_")}`;
							}

							return fetcher<IPrerunEventResponse>(
								this.backend.profile!,
								url,
								{ method: "GET" },
								this.backend.auth!,
							);
						}
					: undefined,
		});
	}
}
