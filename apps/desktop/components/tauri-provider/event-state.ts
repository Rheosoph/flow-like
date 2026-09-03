import {
	type IBoard,
	type IEvent,
	IEventExecutionMode,
	type IEventState,
	IExecutionMode,
	type IHub,
	type IIntercomEvent,
	type ILogMetadata,
	type INode,
	type IOAuthProvider,
	type IOAuthToken,
	type IPrerunEventResponse,
	type IRunPayload,
	type IVersionType,
	type PageTrigger,
	type ProgressToastData,
	checkOAuthTokens,
	checkOAuthTokensFromPrerun,
	classifyPageContractError,
	extractOAuthRequirementsFromBoard,
	finishAllProgressToasts,
	getCurrentPageContext,
	injectDataFunction,
	notifyPageContractRejected,
	serializePageTrigger,
	showProgressToast,
	withCurrentManifestRevision,
} from "@flow-like/flow-like-ui";
import type {
	IEventAlias,
	IEventCorpusResult,
	IEventRunsResult,
	IEventTimeline,
	IEventTimelineRun,
	IListRegistrationsResponse,
	IPutRegressionSuiteRequest,
	IRegressionCorpusPayload,
	IRegressionFixtureSummary,
	IRegressionRunAccepted,
	IRegressionSuiteResult,
	IRegressionSuiteRunDetail,
	IRegressionSuiteRunSummary,
	IRestorePlanResult,
	ISetupEventResponse,
} from "@flow-like/flow-like-ui/state/backend-state/event-state";
import { Channel, invoke } from "@tauri-apps/api/core";
import { toast } from "sonner";
import { fetcher, streamFetcher } from "../../lib/api";
import { isMissingResourceError } from "../../lib/api-error";
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
import { startRegressionSuiteRun } from "./regression-runner";

// Hub configuration cache (shared with board-state)
let hubCache: IHub | undefined;
let hubCachePromise: Promise<IHub | undefined> | undefined;

const LOCAL_DYNAMIC_PAGE_ACTION_ID_PREFIX = "lda1_";
const SERVER_DYNAMIC_PAGE_ACTION_ID_PREFIX = "da1_";

function isLocalDynamicPageTrigger(trigger?: PageTrigger): boolean {
	return (
		trigger?.kind === "action" &&
		trigger.actionId.startsWith(LOCAL_DYNAMIC_PAGE_ACTION_ID_PREFIX)
	);
}

function isServerDynamicPageTrigger(trigger?: PageTrigger): boolean {
	return (
		trigger?.kind === "action" &&
		!trigger.actionId.startsWith(LOCAL_DYNAMIC_PAGE_ACTION_ID_PREFIX) &&
		(Boolean(trigger.capabilityJwt) ||
			trigger.actionId.startsWith(SERVER_DYNAMIC_PAGE_ACTION_ID_PREFIX))
	);
}

function withFeedbackPageContext(localState?: Record<string, any>) {
	if (
		localState &&
		Object.prototype.hasOwnProperty.call(localState, "pageContext")
	) {
		return localState;
	}

	const pageContext = getCurrentPageContext(undefined, { mode: "path" });
	return {
		...(localState ?? {}),
		...(pageContext ? { pageContext } : {}),
	};
}

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

/**
 * Events this client has seen acknowledged by the server, keyed
 * `${appId}:${eventId}`.
 *
 * An authoritative 404/410 is only a revocation for an event the server has
 * previously served to this device — an event created offline for a hosted app
 * and never uploaded 404s the same way, and deleting it would destroy the only
 * copy. Persistence mirrors the page-etag cache: a lost marker merely keeps a
 * local copy alive on a later 404, never the other way around.
 */
const REMOTE_KNOWN_EVENTS_KEY = "flow-like:remote-known-events";
const MAX_REMOTE_KNOWN_EVENTS = 2000;

type RemoteKnownEventMap = Record<string, number>;

function remoteKnownEventKey(appId: string, eventId: string): string {
	return `${appId}:${eventId}`;
}

function readRemoteKnownEvents(): RemoteKnownEventMap {
	if (typeof localStorage === "undefined") return {};
	try {
		const raw = localStorage.getItem(REMOTE_KNOWN_EVENTS_KEY);
		if (!raw) return {};
		const parsed = JSON.parse(raw);
		return parsed && typeof parsed === "object"
			? (parsed as RemoteKnownEventMap)
			: {};
	} catch {
		return {};
	}
}

function writeRemoteKnownEvents(map: RemoteKnownEventMap): void {
	if (typeof localStorage === "undefined") return;
	try {
		const entries = Object.entries(map);
		const bounded =
			entries.length <= MAX_REMOTE_KNOWN_EVENTS
				? map
				: Object.fromEntries(
						entries
							.toSorted((a, b) => b[1] - a[1])
							.slice(0, MAX_REMOTE_KNOWN_EVENTS),
					);
		localStorage.setItem(REMOTE_KNOWN_EVENTS_KEY, JSON.stringify(bounded));
	} catch {
		// Quota or private-mode failures only keep a local copy alive longer.
	}
}

function markEventsRemoteKnown(appId: string, eventIds: string[]): void {
	if (eventIds.length === 0) return;
	const map = readRemoteKnownEvents();
	const now = Date.now();
	for (const eventId of eventIds) {
		map[remoteKnownEventKey(appId, eventId)] = now;
	}
	writeRemoteKnownEvents(map);
}

function isEventRemoteKnown(appId: string, eventId: string): boolean {
	return remoteKnownEventKey(appId, eventId) in readRemoteKnownEvents();
}

function clearEventRemoteKnown(appId: string, eventId: string): void {
	const map = readRemoteKnownEvents();
	const key = remoteKnownEventKey(appId, eventId);
	if (!(key in map)) return;
	delete map[key];
	writeRemoteKnownEvents(map);
}

function eventUpdatedAtMs(event?: IEvent): number {
	const updatedAt = event?.updated_at;
	if (!updatedAt) return Number.NaN;
	return (
		updatedAt.secs_since_epoch * 1000 + updatedAt.nanos_since_epoch / 1_000_000
	);
}

function isLocalEventNewer(
	localEvent: IEvent | undefined,
	remoteEvent: IEvent,
): localEvent is IEvent {
	const localUpdated = eventUpdatedAtMs(localEvent);
	const remoteUpdated = eventUpdatedAtMs(remoteEvent);
	return (
		localEvent !== undefined &&
		!Number.isNaN(localUpdated) &&
		!Number.isNaN(remoteUpdated) &&
		localUpdated > remoteUpdated
	);
}

/**
 * Merge an online event snapshot with the events available on this device.
 *
 * A strictly newer local record wins when both sides contain the same event,
 * matching getEvent's freshness policy. With equal or unavailable timestamps,
 * the remote record remains authoritative. A Local event may legitimately be
 * usable on this device while the server's DB mirror is incomplete, so a forced
 * refresh must not make it disappear. Cached Remote-only events are not retained:
 * the server remains authoritative for events that execute there.
 */
export function mergeLocalAndRemoteEvents(
	localEvents: IEvent[],
	remoteEvents: IEvent[],
): IEvent[] {
	const merged = new Map<string, IEvent>();
	const localById = new Map(localEvents.map((event) => [event.id, event]));

	for (const event of localEvents) {
		if (event.execution_mode !== "Remote") {
			merged.set(event.id, event);
		}
	}

	for (const remoteEvent of remoteEvents) {
		const localEvent = localById.get(remoteEvent.id);
		merged.set(
			remoteEvent.id,
			isLocalEventNewer(localEvent, remoteEvent) ? localEvent : remoteEvent,
		);
	}

	return Array.from(merged.values()).toSorted(
		(a, b) => a.priority - b.priority,
	);
}

export class EventState implements IEventState {
	private readonly remoteEventSyncs = new Map<string, Promise<IEvent[]>>();
	private readonly remoteEventFailures = new Map<
		string,
		{ attempts: number; retryAt: number; error: unknown }
	>();

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
		let event: IEvent | undefined;
		try {
			event = await invoke<IEvent>("get_event", {
				appId: appId,
				eventId: eventId,
				version: version,
			});
		} catch {
			event = undefined;
		}

		const isOffline = await this.backend.isOffline(appId);
		if (isOffline || !this.backend.profile || !this.backend.auth) {
			if (event) return event;
			throw new Error(`Event not found: ${eventId}`);
		}

		let url = `apps/${appId}/events/${eventId}`;
		if (version) {
			url += `?version=${version.join("_")}`;
		}

		try {
			const remoteData = await fetcher<IEvent>(
				this.backend.profile,
				url,
				{ method: "GET" },
				this.backend.auth,
			);

			if (!remoteData) {
				throw new Error("Failed to fetch event data");
			}

			markEventsRemoteKnown(appId, [eventId]);

			const localUpdated = eventUpdatedAtMs(event);
			const remoteUpdated = eventUpdatedAtMs(remoteData);
			const shouldUseRemote =
				!event ||
				typeof version !== "undefined" ||
				Number.isNaN(localUpdated) ||
				Number.isNaN(remoteUpdated) ||
				remoteUpdated >= localUpdated;

			if (!shouldUseRemote) {
				return event!;
			}

			if (typeof version === "undefined") {
				await invoke("upsert_event", {
					appId: appId,
					event: remoteData,
					enforceId: true,
					offline: isOffline,
				}).catch(() => {});
			}

			if (this.backend.queryClient) {
				this.backend.queryClient.setQueryData(
					[this.getEvent.name || "backendFn", appId, eventId, version].filter(
						(arg) => typeof arg !== "undefined",
					),
					remoteData,
				);
			}

			return remoteData;
		} catch (error) {
			// An authoritative 404/410 means the hosted Event was removed. A
			// local Hybrid run may operate without a cloud execution hop, but it
			// must never revive an Event the hub explicitly revoked. That only
			// holds for an Event the server has acknowledged before: one created
			// offline and never uploaded 404s identically, and this device holds
			// its only copy.
			if (isMissingResourceError(error)) {
				if (event && typeof version === "undefined") {
					if (!isEventRemoteKnown(appId, eventId)) {
						console.warn(
							"[EventSync] Event unknown to the server but never synced from it; keeping the local copy:",
							error,
						);
						return event;
					}
					await invoke("delete_event", {
						appId,
						eventId,
					})
						.then(() => clearEventRemoteKnown(appId, eventId))
						.catch((deleteError) => {
							console.warn(
								"[EventSync] Failed to remove a revoked local Event:",
								deleteError,
							);
						});
				}
				throw error;
			}
			if (event) {
				console.warn(
					"[EventSync] Event fetch failed, falling back to local event:",
					error,
				);
				return event;
			}
			const isOffline = await this.backend.isOffline(appId);
			if (isOffline || !this.backend.profile || !this.backend.auth) {
				throw new Error(`Event not found: ${eventId}`);
			}
			throw error;
		}
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

		const syncRemote = () => {
			const active = this.remoteEventSyncs.get(appId);
			if (active) return active;

			let task: Promise<IEvent[]>;
			task = (async () => {
				const remoteData = await fetcher<IEvent[]>(
					this.backend.profile!,
					`apps/${appId}/events`,
					{
						method: "GET",
					},
					this.backend.auth,
				);
				markEventsRemoteKnown(
					appId,
					remoteData.map((event) => event.id),
				);
				const localById = new Map(events.map((event) => [event.id, event]));
				const toPersist = remoteData.filter(
					(event) => !isLocalEventNewer(localById.get(event.id), event),
				);

				// The fetched snapshot is already usable for Remote execution and UI resolution,
				// and `getEvent` reaches the server on its own when a local copy is missing. The
				// cache writes are one IPC round trip per event, so running them ahead of the
				// caller made every app open wait on work nothing was blocked by.
				this.backend.backgroundTaskHandler(
					Promise.allSettled(
						toPersist.map((event) =>
							invoke("upsert_event", {
								appId: appId,
								event: event,
								enforceId: true,
								offline: isOffline,
							}).catch((error) => {
								// A local write can fail for reasons the refresh does not share (a
								// board that has not downloaded yet), and must not turn a successful
								// refresh into "Event not found".
								console.warn(
									`[EventSync] Failed to persist remote event ${event.id} locally:`,
									error,
								);
							}),
						),
					).then(() => undefined),
				);

				this.remoteEventFailures.delete(appId);
				return mergeLocalAndRemoteEvents(events, remoteData);
			})()
				.catch((error) => {
					const previous = this.remoteEventFailures.get(appId)?.attempts ?? 0;
					const attempts = Math.min(previous + 1, 6);
					const delayMs = Math.min(2_000 * 2 ** (attempts - 1), 60_000);
					this.remoteEventFailures.set(appId, {
						attempts,
						retryAt: Date.now() + delayMs,
						error,
					});
					throw error;
				})
				.finally(() => {
					if (this.remoteEventSyncs.get(appId) === task) {
						this.remoteEventSyncs.delete(appId);
					}
				});

			this.remoteEventSyncs.set(appId, task);
			return task;
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

		const failure = this.remoteEventFailures.get(appId);
		if (failure && failure.retryAt > Date.now()) {
			// Preserve the distinction between "the app has no Events" and "we could
			// not load its Events", while still preventing render/query retries from
			// hammering a failing endpoint during the backoff window.
			if (events.length === 0) throw failure.error;
			return events;
		}

		if (events.length === 0) {
			try {
				const remoteData = await syncRemote();
				const queryKey = [this.getEvents.name || "backendFn", appId];
				this.backend.queryClient.setQueryData(queryKey, remoteData);
				return remoteData;
			} catch (error) {
				// An empty local cache is not a valid fallback: returning [] here makes an
				// authentication or server failure look like an app with no Events.
				console.error(
					"[EventSync] Remote event fetch failed with no local fallback:",
					error,
				);
				throw error;
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

	/**
	 * Timeline and run telemetry are assembled from the device's own event
	 * archive and Lance run tables — local-only by design: the local and cloud
	 * version counters diverge once edits happen on both sides, so no
	 * remote-first merge is attempted here.
	 */
	async getEventTimeline(
		appId: string,
		eventId: string,
	): Promise<IEventTimeline> {
		return await invoke<IEventTimeline>("get_event_timeline", {
			appId,
			eventId,
		});
	}

	async listEventRuns(
		appId: string,
		eventId: string,
		boardIds: string[],
		options?: { limit?: number; offset?: number },
	): Promise<IEventTimelineRun[]> {
		const result = await invoke<IEventRunsResult>("list_event_runs", {
			appId,
			eventId,
			boardIds,
			limit: options?.limit,
			offset: options?.offset,
		});
		return result.runs;
	}

	/**
	 * Restore is local-only like the timeline it addresses into: version
	 * tuples are not comparable across transports, so the plan and the write
	 * both target the device's own archive and live event. The command refuses
	 * a non-dry run on Blocking issues — on synced apps local secrets are
	 * already server-filtered, so `SecretUnrecoverable` fires often and needs
	 * `acceptBlankSecrets` to proceed.
	 */
	async restoreEvent(
		appId: string,
		eventId: string,
		version: [number, number, number],
		options?: {
			dryRun?: boolean;
			versionType?: string;
			restoreRoute?: boolean;
			dropCanary?: boolean;
			acceptBlankSecrets?: boolean;
		},
	): Promise<IRestorePlanResult> {
		return await invoke<IRestorePlanResult>("restore_event", {
			appId,
			eventId,
			version,
			versionType: options?.versionType,
			dryRun: options?.dryRun,
			restoreRoute: options?.restoreRoute,
			dropCanary: options?.dropCanary,
			acceptBlankSecrets: options?.acceptBlankSecrets,
			offline: await this.backend.isOffline(appId),
		});
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
			if (!isOffline) clearEventRemoteKnown(appId, eventId);
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

		const localState = withFeedbackPageContext(feedback.localState);
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
						local_state: localState,
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
		pageTrigger?: PageTrigger,
	): Promise<ILogMetadata | undefined> {
		// Substitution is per dispatch path, never shared: the native command
		// re-derives the revision from the LOCAL board, while the server answers
		// for its own contract, and on a hosted app both are live at once.
		// Stamping one path with the other's revision would only move the drift.
		let localTrigger = pageTrigger;
		let remoteTrigger = pageTrigger;

		const runRemotely = () =>
			this.executeEventRemote(
				appId,
				eventId,
				payload,
				streamState,
				onEventId,
				cb,
				remoteTrigger,
			);
		const localDynamicPageAction = isLocalDynamicPageTrigger(pageTrigger);

		if (pageTrigger) {
			if (isServerDynamicPageTrigger(pageTrigger)) {
				return runRemotely();
			}

			let prerun: IPrerunEventResponse;
			try {
				prerun = await this.prerunEvent(appId, eventId, undefined, pageTrigger);
			} catch (error) {
				// The server prerun resolves the trigger, so a removed action is
				// refused HERE and the run is never built. Publishing before the
				// rethrow is the only chance the mounted Page gets to heal.
				this.reportPageContractRejection(appId, eventId, pageTrigger, error);
				throw error;
			}

			// The judging authority just told us the revision it holds. Send that
			// rather than the one the surface was rendered with — a compiled
			// action keeps its identity across a Board edit, so the click the user
			// just made can run with current authority instead of being refused.
			remoteTrigger = withCurrentManifestRevision(
				pageTrigger,
				prerun.manifest_revision,
			);

			if (!prerun.can_execute_locally) {
				if (localDynamicPageAction) {
					throw new Error(
						"This local Page action cannot execute on this device; reload the Page",
					);
				}
				return runRemotely();
			}

			// The server can establish permission and mode, but only the device
			// can prove it holds a Page contract for this Event at all. It must
			// not be an *exact* revision match: the revision hashes the whole
			// Board, so every unrelated edit supersedes what an already-rendered
			// Page carries, and demanding equality here sent a perfectly runnable
			// action to the server — or, offline, failed it outright. The native
			// command re-resolves the trigger against the current local contract
			// and re-checks the entry node, so a superseded revision is safe.
			if (!localDynamicPageAction) {
				let holdsLocalContract = false;
				let deviceRevision: string | undefined;
				try {
					const localBootstrap = await invoke<{
						executionRevision?: string;
					}>("get_local_page_bootstrap", {
						appId,
						eventId,
					});
					deviceRevision = localBootstrap.executionRevision ?? undefined;
					holdsLocalContract = Boolean(
						pageTrigger.manifestRevision && deviceRevision,
					);
				} catch (error) {
					console.warn(
						"[executeEvent] No local Page contract for this Event:",
						error,
					);
				}
				if (!holdsLocalContract) {
					if (!(await this.canReachServer(appId))) {
						notifyPageContractRejected({
							appId,
							eventId,
							renderedRevision: pageTrigger.manifestRevision,
							reason: "missing_contract",
						});
						throw new Error(
							"This device holds no Page contract for this Event and the server is unreachable; reload the Page",
						);
					}
					return runRemotely();
				}
				// The native command judges against the device's own board, so the
				// device revision — not the server's — is the current one for the
				// invoke below.
				localTrigger = withCurrentManifestRevision(pageTrigger, deviceRevision);
			}
		}

		const event = await this.getEvent(appId, eventId);

		// An event pinned to Remote has no board on this device. Reading one only
		// fails on the way to a run that belongs on the server anyway, so the
		// dispatch happens here rather than after a "board not found".
		if (event.execution_mode === IEventExecutionMode.Remote) {
			if (localDynamicPageAction) {
				throw new Error(
					"A local Page action cannot be sent to a Remote Event; reload the Page",
				);
			}
			if (await this.canReachServer(appId)) {
				return runRemotely();
			}
		}

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
		let board: IBoard;
		try {
			board = await this.backend.boardState.getBoard(
				appId,
				event.board_id,
				(event.board_version as [number, number, number]) ?? undefined,
				true,
			);
		} catch (error) {
			// Everything below reads the flow to prepare a local run: packages,
			// RPA consent, OAuth tokens. A user who may run an event but not read
			// its board — the normal shape of a published app — gets nothing back
			// from any of it, and the server can run it instead: it holds the
			// board, and resolves permissions, secrets and OAuth on its own.
			if (localDynamicPageAction || !(await this.canReachServer(appId))) {
				throw error;
			}
			console.warn(
				"[executeEvent] Board unavailable for local execution, running on the server:",
				error,
			);
			return runRemotely();
		}

		await this.backend.boardState.ensureAppPackagesInstalledForExecution?.(
			appId,
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

		let metadata: ILogMetadata | undefined;
		try {
			metadata = await invoke("execute_event", {
				appId: appId,
				eventId: eventId,
				payload: payload,
				events: channel,
				streamState: streamState,
				credentials,
				token,
				oauthTokens,
				pageTrigger: localTrigger
					? serializePageTrigger(localTrigger)
					: undefined,
			});
		} catch (error) {
			this.reportPageContractRejection(appId, eventId, localTrigger, error);
			throw error;
		}

		closed = true;

		return metadata;
	}

	/**
	 * Tell any mounted Page that the authority refused this trigger for a reason
	 * a refreshed contract can cure, so it can refetch itself. Only classified
	 * contract failures publish — a routing or permission refusal is not
	 * something a refetch fixes, and treating it as one is how a refetch loop
	 * starts. Never called on success: a run that completed has already rewritten
	 * the surface through its A2UI messages.
	 */
	private reportPageContractRejection(
		appId: string,
		eventId: string,
		trigger: PageTrigger | undefined,
		error: unknown,
	): void {
		if (!trigger) return;
		const failure = classifyPageContractError(error);
		if (!failure) return;
		notifyPageContractRejected({
			appId,
			eventId,
			renderedRevision: trigger.manifestRevision,
			reason: failure,
		});
	}

	async executeEventRemote(
		appId: string,
		eventId: string,
		payload: IRunPayload,
		streamState?: boolean,
		onEventId?: (id: string) => void,
		cb?: (event: IIntercomEvent[]) => void,
		pageTrigger?: PageTrigger,
	): Promise<ILogMetadata | undefined> {
		if (isLocalDynamicPageTrigger(pageTrigger)) {
			throw new Error(
				"A local Page action cannot be sent to the server; reload the Page",
			);
		}
		if (!this.backend.profile || !this.backend.auth) {
			throw new Error("Profile and auth required for remote execution");
		}

		let closed = false;
		let foundRunId = false;

		try {
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
						page_trigger: pageTrigger
							? serializePageTrigger(pageTrigger)
							: undefined,
					}),
				},
				this.backend.auth,
				(event: IIntercomEvent) => {
					if (closed) return;

					if (
						!foundRunId &&
						onEventId &&
						event.event_type === "run_initiated"
					) {
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
		} catch (error) {
			this.reportPageContractRejection(appId, eventId, pageTrigger, error);
			throw error;
		}

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

	async listEventRegistrations(
		appId: string,
		eventId: string,
		version?: string,
		variant?: string,
	): Promise<IListRegistrationsResponse> {
		if (!this.backend.profile || !this.backend.auth) {
			return {
				event_id: eventId,
				event_version: null,
				variant: variant ?? "stable",
				registrations: [],
			};
		}
		const params = new URLSearchParams();
		if (version) params.set("version", version);
		if (variant) params.set("variant", variant);
		const qs = params.size > 0 ? `?${params.toString()}` : "";
		return await fetcher<IListRegistrationsResponse>(
			this.backend.profile,
			`apps/${appId}/events/${eventId}/registrations${qs}`,
			{ method: "GET" },
			this.backend.auth,
		);
	}

	async listEventAliases(
		appId: string,
		eventId: string,
	): Promise<IEventAlias[]> {
		if (!this.backend.profile || !this.backend.auth) {
			return [];
		}
		return await fetcher<IEventAlias[]>(
			this.backend.profile,
			`apps/${appId}/events/${eventId}/alias`,
			{ method: "GET" },
			this.backend.auth,
		);
	}

	async setupEvent(
		appId: string,
		eventId: string,
		force = false,
		variant?: string,
	): Promise<ISetupEventResponse> {
		if (!this.backend.profile || !this.backend.auth) {
			throw new Error("Remote setup requires an online profile");
		}
		return await fetcher<ISetupEventResponse>(
			this.backend.profile,
			`apps/${appId}/events/${eventId}/setup`,
			{ method: "POST", body: JSON.stringify({ force, variant }) },
			this.backend.auth,
		);
	}

	async upsertEventAlias(
		appId: string,
		eventId: string,
		slug: string,
	): Promise<IEventAlias> {
		if (!this.backend.profile || !this.backend.auth) {
			throw new Error("Alias setup requires an online profile");
		}
		return await fetcher<IEventAlias>(
			this.backend.profile,
			`apps/${appId}/events/${eventId}/alias/${encodeURIComponent(slug)}`,
			{ method: "PUT", body: "{}" },
			this.backend.auth,
		);
	}

	async deleteEventAlias(
		appId: string,
		eventId: string,
		slug: string,
	): Promise<void> {
		if (!this.backend.profile || !this.backend.auth) {
			throw new Error("Alias setup requires an online profile");
		}
		await fetcher<void>(
			this.backend.profile,
			`apps/${appId}/events/${eventId}/alias/${encodeURIComponent(slug)}`,
			{ method: "DELETE" },
			this.backend.auth,
		);
	}

	async checkEventOAuth(
		appId: string,
		event: IEvent,
	): Promise<{
		tokens?: Record<string, IOAuthToken>;
		missingProviders: IOAuthProvider[];
	}> {
		// An event pinned to Remote has no board on this device, and the server
		// resolves OAuth on its own for runs it hosts.
		if (event.execution_mode === IEventExecutionMode.Remote) {
			return { missingProviders: [] };
		}

		let board: IBoard;
		try {
			board = await this.backend.boardState.getBoard(
				appId,
				event.board_id,
				(event.board_version as [number, number, number]) ?? undefined,
				true,
			);
		} catch (error) {
			// A user who may run an event but not read its board — the normal
			// shape of a published app — cannot resolve OAuth here and does not
			// need to: the run is handed to the server, which holds the board.
			// A local run re-checks OAuth inside executeEvent, so skipping this
			// preflight never skips the gate.
			console.warn(
				"[checkEventOAuth] Board unavailable, skipping local OAuth preflight:",
				error,
			);
			return { missingProviders: [] };
		}

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

	async checkOAuthRequirements(
		appId: string,
		requirements: Array<{ provider_id: string; scopes: string[] }>,
	): Promise<{
		tokens?: Record<string, IOAuthToken>;
		missingProviders: IOAuthProvider[];
	}> {
		const hub = await getHubConfig(this.backend.profile);
		const oauthResult = await checkOAuthTokensFromPrerun(
			requirements,
			oauthTokenStore,
			hub,
			{ refreshToken: oauthService.refreshToken.bind(oauthService) },
		);
		const consentedIds = await oauthConsentStore.getConsentedProviderIds(appId);
		const missingProviders = [...oauthResult.missingProviders];
		for (const provider of oauthResult.requiredProviders) {
			if (
				oauthResult.tokens[provider.id] !== undefined &&
				!consentedIds.has(provider.id)
			) {
				missingProviders.push(provider);
			}
		}
		if (missingProviders.length > 0) {
			return { tokens: undefined, missingProviders };
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
		pageTrigger?: PageTrigger,
	): Promise<IPrerunEventResponse> {
		const loadLocalEvent = async (): Promise<IEvent> =>
			invoke<IEvent>("get_event", { appId, eventId, version });

		// Helper to build prerun response from local event/board
		const buildLocalPrerun = async (): Promise<IPrerunEventResponse> => {
			const event: IEvent = await loadLocalEvent();
			if (pageTrigger && (!event.active || !event.default_page_id)) {
				throw new Error(
					"Page triggers require an active Event with a configured Page",
				);
			}
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
				event_execution_mode: event.execution_mode ?? IEventExecutionMode.Local,
				can_execute_locally,
				has_wasm_nodes: wasmPackageIds.size > 0,
				wasm_package_ids: Array.from(wasmPackageIds),
				wasm_package_permissions: wasmPackagePermissions,
			};
		};

		const fetchRemotePrerun =
			this.backend.profile && this.backend.auth
				? async () => {
						let url = `apps/${appId}/events/${eventId}/prerun`;
						if (version) {
							url += `?version=${version.join("_")}`;
						}

						return fetcher<IPrerunEventResponse>(
							this.backend.profile!,
							url,
							pageTrigger
								? {
										method: "POST",
										body: JSON.stringify({
											page_trigger: serializePageTrigger(pageTrigger),
										}),
									}
								: { method: "GET" },
							this.backend.auth!,
						);
					}
				: undefined;

		if (pageTrigger) {
			if (isLocalDynamicPageTrigger(pageTrigger)) {
				return buildLocalPrerun();
			}
			const dynamic = isServerDynamicPageTrigger(pageTrigger);
			const localOnly = await this.backend
				.isLocalOnly(appId)
				.catch(() => false);

			if (localOnly && !dynamic) {
				return buildLocalPrerun();
			}
			if (!fetchRemotePrerun) {
				throw new Error(
					"Page action authorization requires a remote prerun endpoint",
				);
			}

			// Hosted Page execution always obtains a fresh governed server
			// decision first. The native command then independently rechecks the
			// same caller and exact local contract before starting the run.
			return fetchRemotePrerun();
		}

		// An event pinned to Remote never runs on this device, so its board is not
		// expected to be here. Answering that preflight from a local board would
		// report on a machine the run never touches — and usually just fails,
		// sending the caller down the local path it must not take.
		const remoteEvent = await this.resolveRemotePinnedEvent(
			appId,
			eventId,
			loadLocalEvent,
		);
		if (remoteEvent) {
			if (fetchRemotePrerun) {
				try {
					const remoteResult = await fetchRemotePrerun();
					if (remoteResult) return remoteResult;
				} catch (error) {
					console.warn(
						"[prerunEvent] API prerun failed for a Remote event:",
						error,
					);
				}
			}

			return {
				board_id: remoteEvent.board_id,
				runtime_variables: [],
				oauth_requirements: [],
				requires_local_execution: false,
				execution_mode: IExecutionMode.Remote,
				event_execution_mode: IEventExecutionMode.Remote,
				can_execute_locally: false,
				has_wasm_nodes: false,
				wasm_package_ids: [],
				wasm_package_permissions: {},
			};
		}

		// Local-only apps have no server answer to ask for. An app whose
		// visibility is merely uncached is not one of them — treating it as one
		// is what left this preflight with only a board it does not have.
		if (await this.backend.isLocalOnly(appId).catch(() => false)) {
			return buildLocalPrerun();
		}

		return resolveLocalFirstPrerun({
			label: "prerunEvent",
			buildLocal: buildLocalPrerun,
			fetchRemote: fetchRemotePrerun,
		});
	}

	/* -------------------------------------------- regression suites (Track D) */
	// Local-only by design, like the timeline: corpus, fixtures, suite config
	// and run archives all live on this device (the same core bucket layout as
	// cloud, over the local meta store). Desktop suites have no schedule and no
	// publish gate; the runner is client-side (regression-runner.ts) and its
	// replays are fully LIVE runs — no shadow isolation exists locally.

	async getEventCorpus(
		appId: string,
		eventId: string,
		limit?: number,
	): Promise<IEventCorpusResult> {
		return await invoke<IEventCorpusResult>("list_regression_corpus", {
			appId,
			eventId,
			limit,
		});
	}

	async getCorpusPayload(
		appId: string,
		eventId: string,
		runId: string,
	): Promise<IRegressionCorpusPayload> {
		return await invoke<IRegressionCorpusPayload>(
			"get_regression_corpus_payload",
			{ appId, eventId, runId },
		);
	}

	async promoteRegressionFixture(
		appId: string,
		eventId: string,
		runId: string,
		options?: {
			expectation?: "pass" | "fail";
			acknowledgeRejected?: boolean;
		},
	): Promise<IRegressionFixtureSummary> {
		return await invoke<IRegressionFixtureSummary>(
			"promote_regression_fixture",
			{
				appId,
				eventId,
				runId,
				expectation: options?.expectation,
				acknowledgeRejected: options?.acknowledgeRejected ?? false,
			},
		);
	}

	async deleteRegressionFixture(
		appId: string,
		eventId: string,
		fixtureId: string,
	): Promise<void> {
		await invoke("delete_regression_fixture", { appId, eventId, fixtureId });
	}

	async getRegressionSuite(
		appId: string,
		eventId: string,
	): Promise<IRegressionSuiteResult | null> {
		return await invoke<IRegressionSuiteResult | null>("get_regression_suite", {
			appId,
			eventId,
		});
	}

	async putRegressionSuite(
		appId: string,
		eventId: string,
		config: IPutRegressionSuiteRequest,
	): Promise<IRegressionSuiteResult> {
		return await invoke<IRegressionSuiteResult>("upsert_regression_suite", {
			appId,
			eventId,
			triggerOnPublish: config.trigger_on_publish,
			schedule: config.schedule ?? null,
			gateMode: config.gate_mode,
			allowLiveSideEffects: config.allow_live_side_effects,
		});
	}

	async runRegressionSuite(
		appId: string,
		eventId: string,
		options?: {
			boardVersion?: [number, number, number];
			allowDraft?: boolean;
		},
	): Promise<IRegressionRunAccepted> {
		const suiteResult = await this.getRegressionSuite(appId, eventId);
		if (!suiteResult) {
			throw new Error(
				"No regression suite is configured for this event — save one first from the event's Quality section",
			);
		}
		return startRegressionSuiteRun(
			this.backend,
			appId,
			eventId,
			suiteResult.suite,
			options,
		);
	}

	async listRegressionRuns(
		appId: string,
		eventId: string,
	): Promise<IRegressionSuiteRunSummary[]> {
		return await invoke<IRegressionSuiteRunSummary[]>(
			"list_regression_suite_runs",
			{ appId, eventId },
		);
	}

	async getRegressionRun(
		appId: string,
		eventId: string,
		suiteRunId: string,
	): Promise<IRegressionSuiteRunDetail> {
		return await invoke<IRegressionSuiteRunDetail>("get_regression_suite_run", {
			appId,
			eventId,
			suiteRunId,
		});
	}

	/**
	 * Whether a run can be handed to the server for this app. `isOffline` also
	 * reports true when the app's visibility has simply never been cached, which
	 * is not a reason to give up on the server — only an app this device
	 * positively knows is local-only is.
	 */
	private async canReachServer(appId: string): Promise<boolean> {
		if (!this.backend.profile || !this.backend.auth) return false;
		return !(await this.backend.isLocalOnly(appId).catch(() => false));
	}

	/**
	 * Returns the event when it is pinned to Remote execution, otherwise
	 * undefined. Reads the device copy first and only asks the hub when there is
	 * none, so the common case costs one IPC call.
	 */
	private async resolveRemotePinnedEvent(
		appId: string,
		eventId: string,
		loadLocalEvent: () => Promise<IEvent>,
	): Promise<IEvent | undefined> {
		let event = await loadLocalEvent().catch(() => undefined);

		if (!event && this.backend.profile && this.backend.auth) {
			event = await this.getEvent(appId, eventId).catch(() => undefined);
		}

		return event?.execution_mode === IEventExecutionMode.Remote
			? event
			: undefined;
	}
}
