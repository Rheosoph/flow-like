import {
	type IBoard,
	type IEvent,
	type IEventState,
	type IEventVariant,
	type IHub,
	type IIntercomEvent,
	type ILogMetadata,
	type IOAuthProvider,
	type IOAuthToken,
	type IRunPayload,
	type IVersionType,
	type PageTrigger,
	type ProgressToastData,
	checkOAuthTokens,
	checkOAuthTokensFromPrerun,
	classifyPageContractError,
	finishAllProgressToasts,
	getCurrentPageContext,
	notifyPageContractRejected,
	serializePageTrigger,
	showProgressToast,
	withCurrentManifestRevision,
} from "@flow-like/flow-like-ui";
import {
	apiResponseError,
	isMissingResourceError,
} from "@flow-like/flow-like-ui/lib/api-error";
import type { IOAuthCheckResult } from "@flow-like/flow-like-ui/state/backend-state/event-state";
import type {
	ICanaryExplainResult,
	ICanaryPromoteResult,
	IEventAlias,
	IEventCorpusResult,
	IEventRunsResult,
	IEventSetupInfo,
	IEventTimeline,
	IEventTimelineRun,
	IEventVariantSharePatch,
	IEventVariantStatsResult,
	IEventVariantStatsWindow,
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
import type { IPrerunEventResponse } from "@flow-like/flow-like-ui/state/backend-state/types";
import { toast } from "sonner";
import { oauthConsentStore, oauthTokenStore } from "../oauth-db";
import { getOAuthApiBaseUrl, getOAuthService } from "../oauth-service";
import {
	type WebBackendRef,
	apiDelete,
	apiGet,
	apiPatch,
	apiPost,
	apiPut,
	getApiBaseUrl,
} from "./api-utils";

// Hub configuration cache
let hubCache: IHub | undefined;
let hubCachePromise: Promise<IHub | undefined> | undefined;

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

	const url =
		hubUrl.startsWith("http://") || hubUrl.startsWith("https://")
			? `${hubUrl}/api/v1`
			: `https://${hubUrl}/api/v1`;

	hubCachePromise = fetch(url)
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

interface ToastEventPayload {
	message: string;
	level: "success" | "error" | "info" | "warning";
}

function handleToastEvent(event: IIntercomEvent): void {
	const payload = event.payload as ToastEventPayload;
	if (!payload?.message) return;

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

function handleProgressEvent(event: IIntercomEvent): void {
	const payload = event.payload as ProgressToastData;
	if (!payload?.id) return;
	showProgressToast(payload);
}

/**
 * Tell any mounted Page that the authority refused this trigger for a reason a
 * refreshed contract can cure. Only classified contract failures publish — a
 * routing or permission refusal is not something a refetch fixes, and treating
 * it as one is how a refetch loop starts. Never called on success.
 */
function reportPageContractRejection(
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

export class WebEventState implements IEventState {
	readonly alwaysRemote = true;

	constructor(private readonly backend: WebBackendRef) {}

	async getEvent(
		appId: string,
		eventId: string,
		version?: [number, number, number],
	): Promise<IEvent> {
		const params = version ? `?version=${version.join("_")}` : "";
		return apiGet<IEvent>(
			`apps/${appId}/events/${eventId}${params}`,
			this.backend.auth,
		);
	}

	// Errors propagate: returning [] here makes a failed fetch look like an app
	// with no events, which callers cannot distinguish from the real thing.
	async getEvents(appId: string, _force?: boolean): Promise<IEvent[]> {
		return await apiGet<IEvent[]>(`apps/${appId}/events`, this.backend.auth);
	}

	async getEventVersions(
		appId: string,
		eventId: string,
	): Promise<[number, number, number][]> {
		try {
			return await apiGet<[number, number, number][]>(
				`apps/${appId}/events/${eventId}/versions`,
				this.backend.auth,
			);
		} catch {
			return [];
		}
	}

	async upsertEvent(
		appId: string,
		event: IEvent,
		versionType?: IVersionType,
		personalAccessToken?: string,
		oauthTokens?: Record<string, IOAuthToken>,
	): Promise<IEvent> {
		return apiPut<IEvent>(
			`apps/${appId}/events/${event.id}`,
			{
				event,
				version_type: versionType,
				pat: personalAccessToken,
				oauth_tokens: oauthTokens,
				profile_id: this.backend.profile?.id,
			},
			this.backend.auth,
		);
	}

	async checkEventOAuth(
		appId: string,
		event: IEvent,
	): Promise<IOAuthCheckResult> {
		try {
			// Get the board for this event
			const boardParams = event.board_version
				? `?version=${event.board_version.join("_")}`
				: "";
			const board = await apiGet<IBoard>(
				`apps/${appId}/board/${event.board_id}${boardParams}`,
				this.backend.auth,
			);

			const hub = await getHubConfig(this.backend.profile);
			const oauthService = getOAuthService(
				getOAuthApiBaseUrl(this.backend.profile?.hub),
			);
			const oauthResult = await checkOAuthTokens(board, oauthTokenStore, hub, {
				refreshToken: oauthService.refreshToken.bind(oauthService),
			});

			console.log("[checkEventOAuth] oauthResult:", {
				requiredProviders: oauthResult.requiredProviders?.map((p) => p.id),
				missingProviders: oauthResult.missingProviders?.map((p) => p.id),
				tokens: Object.keys(oauthResult.tokens || {}),
			});

			// Check consent for providers that have tokens but might not have consent for this app
			const consentedIds =
				await oauthConsentStore.getConsentedProviderIds(appId);
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
		} catch (error) {
			console.error("[checkEventOAuth] Error:", error);
			return { missingProviders: [] };
		}
	}

	async checkOAuthRequirements(
		appId: string,
		requirements: Array<{ provider_id: string; scopes: string[] }>,
	): Promise<IOAuthCheckResult> {
		const hub = await getHubConfig(this.backend.profile);
		const oauthService = getOAuthService(
			getOAuthApiBaseUrl(this.backend.profile?.hub),
		);
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

	async deleteEvent(appId: string, eventId: string): Promise<void> {
		await apiDelete(`apps/${appId}/events/${eventId}`, this.backend.auth);
	}

	async validateEvent(
		appId: string,
		eventId: string,
		version?: [number, number, number],
	): Promise<void> {
		const params = version ? `?version=${version.join("_")}` : "";
		await apiPost(
			`apps/${appId}/events/${eventId}/validate${params}`,
			undefined,
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
		const localState = withFeedbackPageContext(feedback.localState);
		const context =
			feedback.history || feedback.globalState || localState
				? {
						history: feedback.history,
						global_state: feedback.globalState,
						local_state: localState,
					}
				: undefined;

		const result = await apiPut<{ feedback_id: string }>(
			`apps/${appId}/events/${eventId}/feedback`,
			{
				rating: feedback.rating,
				context,
				comment: feedback.comment ?? "",
				feedback_id: feedbackId,
			},
			this.backend.auth,
		);
		return result.feedback_id;
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
		const hub = await getHubConfig(this.backend.profile);
		const oauthService = getOAuthService(
			getOAuthApiBaseUrl(this.backend.profile?.hub),
		);
		const oauthOptions = {
			refreshToken: oauthService.refreshToken.bind(oauthService),
		};
		// The prerun resolves the trigger server-side, so it both answers with the
		// revision the server currently holds AND is where a removed action is
		// refused — before the invoke is ever built. Substitute the fresh revision
		// so the click runs with current authority; on a refusal, tell the mounted
		// Page to refetch instead of letting the reason die here.
		let activeTrigger = pageTrigger;
		let pagePrerun: IPrerunEventResponse | undefined;
		if (pageTrigger) {
			try {
				pagePrerun = await this.prerunEvent(
					appId,
					eventId,
					undefined,
					pageTrigger,
				);
			} catch (error) {
				reportPageContractRejection(appId, eventId, pageTrigger, error);
				throw error;
			}
			activeTrigger = withCurrentManifestRevision(
				pageTrigger,
				pagePrerun.manifest_revision,
			);
		}

		const oauthResult = pagePrerun
			? await checkOAuthTokensFromPrerun(
					pagePrerun.oauth_requirements,
					oauthTokenStore,
					hub,
					oauthOptions,
				)
			: await (async () => {
					const event = await this.getEvent(appId, eventId);
					const boardParams = event.board_version
						? `?version=${event.board_version.join("_")}`
						: "";
					const board = await apiGet<IBoard>(
						`apps/${appId}/board/${event.board_id}${boardParams}`,
						this.backend.auth,
					);
					return checkOAuthTokens(board, oauthTokenStore, hub, oauthOptions);
				})();

		console.log("[OAuth] Event check result:", {
			requiredProviders: oauthResult.requiredProviders.map((p) => p.id),
			missingProviders: oauthResult.missingProviders.map((p) => p.id),
			hasTokens: Object.keys(oauthResult.tokens),
			skipConsentCheck,
		});

		if (!skipConsentCheck) {
			const consentedIds =
				await oauthConsentStore.getConsentedProviderIds(appId);
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

			if (providersNeedingConsent.length > 0) {
				const error = new Error(
					`Missing OAuth authorization for: ${providersNeedingConsent.map((p) => p.name).join(", ")}`,
				);
				(error as any).missingProviders = providersNeedingConsent;
				(error as any).isOAuthError = true;
				throw error;
			}
		} else {
			// Still need to check for missing tokens even if skipping consent
			if (oauthResult.missingProviders.length > 0) {
				const error = new Error(
					`Missing OAuth tokens for: ${oauthResult.missingProviders.map((p) => p.name).join(", ")}`,
				);
				(error as any).missingProviders = oauthResult.missingProviders;
				(error as any).isOAuthError = true;
				throw error;
			}
		}

		// Collect OAuth tokens to pass to execution
		const oauthTokens =
			Object.keys(oauthResult.tokens).length > 0
				? oauthResult.tokens
				: undefined;

		const baseUrl = getApiBaseUrl();
		const url = `${baseUrl}/api/v1/apps/${appId}/events/${eventId}/invoke`;

		const headers: HeadersInit = {
			"Content-Type": "application/json",
		};
		if (this.backend.auth?.user?.access_token) {
			headers["Authorization"] =
				`Bearer ${this.backend.auth.user.access_token}`;
		}

		console.log("[OAuth] Sending event execution with tokens:", {
			hasOAuthTokens: !!oauthTokens,
			tokenProviders: oauthTokens ? Object.keys(oauthTokens) : [],
		});

		let executionFinished = false;
		try {
			const response = await fetch(url, {
				method: "POST",
				headers,
				body: JSON.stringify({
					payload: payload.payload,
					token: this.backend.auth?.user?.access_token,
					oauth_tokens: oauthTokens,
					runtime_variables: payload.runtime_variables,
					profile_id: this.backend.profile?.id,
					page_trigger: activeTrigger
						? serializePageTrigger(activeTrigger)
						: undefined,
				}),
			});

			if (!response.ok) {
				// The status alone destroys the server's reason, which is the only
				// thing that tells a Page contract failure apart from any other 400.
				const body = await response.text().catch(() => "");
				const error = apiResponseError(response, body, url);
				reportPageContractRejection(appId, eventId, activeTrigger, error);
				throw error;
			}

			// Always consume the SSE stream - the API always returns one
			if (response.body) {
				const reader = response.body.getReader();
				const decoder = new TextDecoder();
				let buffer = "";
				let foundRunId = false;

				while (true) {
					const { done, value } = await reader.read();
					if (done) break;

					buffer += decoder.decode(value, { stream: true });

					// SSE events are separated by double newlines
					const parts = buffer.split("\n\n");
					buffer = parts.pop() ?? "";

					for (const part of parts) {
						if (!part.trim()) continue;

						// Parse SSE format: "event: xxx\ndata: {...}"
						let eventName = "message";
						const dataLines: string[] = [];

						for (const line of part.split("\n")) {
							if (line.startsWith("event:")) {
								eventName = line.slice(6).trim();
							} else if (line.startsWith("data:")) {
								// Strip only the optional single leading space —
								// trimming destroys whitespace-only tokens.
								const value = line.slice(5);
								dataLines.push(value.startsWith(" ") ? value.slice(1) : value);
							} else if (line.startsWith(":")) {
								continue;
							}
						}

						const eventData = dataLines.join("\n");

						if (!eventData.trim() || eventData === "keep-alive") continue;

						try {
							const event = JSON.parse(eventData) as IIntercomEvent;

							// Handle run_initiated to get run ID
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

							// Handle toast events
							if (event.event_type === "toast") {
								handleToastEvent(event);
							}

							// Handle progress events
							if (event.event_type === "progress") {
								handleProgressEvent(event);
							}

							// Forward event to callback
							if (cb) {
								cb([event]);
							}

							// Note the terminal event but keep delivering the rest of
							// the decoded batch — the executor coalesces trailing
							// chat_out/usage events into the same network chunk.
							if (
								eventName === "done" ||
								eventName === "completed" ||
								event.event_type === "completed"
							) {
								executionFinished = true;
								finishAllProgressToasts(true);
							} else if (event.event_type === "error") {
								executionFinished = true;
								finishAllProgressToasts(false);
							}
						} catch (error) {
							console.warn(
								"[SSE] Dropping unparseable event frame:",
								error,
								eventData.slice(0, 200),
							);
						}
					}
					if (executionFinished) break;
				}
			}

			// Ensure progress toasts are finished when stream ends
			if (!executionFinished) {
				finishAllProgressToasts(true);
			}

			return undefined;
		} catch (error) {
			finishAllProgressToasts(false);
			throw error;
		}
	}

	async cancelExecution(runId: string): Promise<void> {
		await apiPost(`runs/${runId}/cancel`, undefined, this.backend.auth);
	}

	async isEventSinkActive(eventId: string): Promise<boolean> {
		try {
			const result = await apiGet<{ active: boolean }>(
				`sinks/${eventId}/status`,
				this.backend.auth,
			);
			return result?.active ?? false;
		} catch {
			return false;
		}
	}

	async listEventRegistrations(
		appId: string,
		eventId: string,
		version?: string,
		variant?: string,
	): Promise<IListRegistrationsResponse> {
		const params = new URLSearchParams();
		if (version) params.set("version", version);
		if (variant) params.set("variant", variant);
		const qs = params.size > 0 ? `?${params.toString()}` : "";
		return apiGet<IListRegistrationsResponse>(
			`apps/${appId}/events/${eventId}/registrations${qs}`,
			this.backend.auth,
		);
	}

	async listEventAliases(
		appId: string,
		eventId: string,
	): Promise<IEventAlias[]> {
		return apiGet<IEventAlias[]>(
			`apps/${appId}/events/${eventId}/alias`,
			this.backend.auth,
		);
	}

	async setupEvent(
		appId: string,
		eventId: string,
		force = false,
		variant?: string,
	): Promise<ISetupEventResponse> {
		return apiPost<ISetupEventResponse>(
			`apps/${appId}/events/${eventId}/setup`,
			{ force, variant },
			this.backend.auth,
		);
	}

	async upsertEventAlias(
		appId: string,
		eventId: string,
		slug: string,
	): Promise<IEventAlias> {
		return apiPut<IEventAlias>(
			`apps/${appId}/events/${eventId}/alias/${encodeURIComponent(slug)}`,
			{},
			this.backend.auth,
		);
	}

	async deleteEventAlias(
		appId: string,
		eventId: string,
		slug: string,
	): Promise<void> {
		await apiDelete<void>(
			`apps/${appId}/events/${eventId}/alias/${encodeURIComponent(slug)}`,
			this.backend.auth,
		);
	}

	async getEventTimeline(
		appId: string,
		eventId: string,
	): Promise<IEventTimeline> {
		return apiGet<IEventTimeline>(
			`apps/${appId}/events/${eventId}/timeline`,
			this.backend.auth,
		);
	}

	async listEventRuns(
		appId: string,
		eventId: string,
		boardIds: string[],
		options?: { limit?: number; offset?: number },
	): Promise<IEventTimelineRun[]> {
		const params = new URLSearchParams();
		for (const boardId of boardIds) {
			params.append("board_id", boardId);
		}
		if (options?.limit !== undefined) {
			params.set("limit", String(options.limit));
		}
		if (options?.offset !== undefined) {
			params.set("offset", String(options.offset));
		}
		const query = params.toString();
		const result = await apiGet<IEventRunsResult>(
			`apps/${appId}/events/${eventId}/runs${query ? `?${query}` : ""}`,
			this.backend.auth,
		);
		return result.runs;
	}

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
		return apiPost<IRestorePlanResult>(
			`apps/${appId}/events/${eventId}/restore`,
			{
				version,
				version_type: options?.versionType,
				dry_run: options?.dryRun ?? true,
				restore_route: options?.restoreRoute ?? false,
				drop_canary: options?.dropCanary ?? false,
				accept_blank_secrets: options?.acceptBlankSecrets ?? false,
			},
			this.backend.auth,
		);
	}

	async getCanaryStats(
		appId: string,
		eventId: string,
		window: IEventVariantStatsWindow = "24h",
	): Promise<IEventVariantStatsResult> {
		return apiGet<IEventVariantStatsResult>(
			`apps/${appId}/events/${eventId}/canary/stats?window=${window}`,
			this.backend.auth,
		);
	}

	// Share edits go through the dedicated PATCH so they never cut an event
	// version and never re-run the REST/MCP setup (slider path).
	async patchCanary(
		appId: string,
		eventId: string,
		patch: IEventVariantSharePatch,
	): Promise<IEvent> {
		return apiPatch<IEvent>(
			`apps/${appId}/events/${eventId}/canary`,
			patch,
			this.backend.auth,
		);
	}

	async putEventVariants(
		appId: string,
		eventId: string,
		variants: IEventVariant[],
	): Promise<IEvent> {
		return apiPut<IEvent>(
			`apps/${appId}/events/${eventId}/variants`,
			{ variants },
			this.backend.auth,
		);
	}

	// Both stores are written server-side in the review-verified order; the
	// response carries the promoted event plus the non-fatal setup outcome.
	async promoteCanary(
		appId: string,
		eventId: string,
		variant: string,
		versionType?: IVersionType,
	): Promise<ICanaryPromoteResult> {
		return apiPost<ICanaryPromoteResult>(
			`apps/${appId}/events/${eventId}/canary/promote`,
			{ variant, version_type: versionType },
			this.backend.auth,
		);
	}

	async abortCanary(
		appId: string,
		eventId: string,
		variant: string,
	): Promise<IEvent> {
		const result = await apiPost<{ event: IEvent }>(
			`apps/${appId}/events/${eventId}/canary/abort`,
			{ variant },
			this.backend.auth,
		);
		return result.event;
	}

	async listEventSetups(
		appId: string,
		eventId: string,
	): Promise<IEventSetupInfo[]> {
		return apiGet<IEventSetupInfo[]>(
			`apps/${appId}/events/${eventId}/setups`,
			this.backend.auth,
		);
	}

	async explainCanary(
		appId: string,
		eventId: string,
		key: string,
		source?: string,
	): Promise<ICanaryExplainResult> {
		const params = new URLSearchParams({ key });
		if (source) params.set("source", source);
		return apiGet<ICanaryExplainResult>(
			`apps/${appId}/events/${eventId}/canary/explain?${params.toString()}`,
			this.backend.auth,
		);
	}

	async getEventCorpus(
		appId: string,
		eventId: string,
		limit?: number,
	): Promise<IEventCorpusResult> {
		const qs = limit !== undefined ? `?limit=${limit}` : "";
		return apiGet<IEventCorpusResult>(
			`apps/${appId}/events/${eventId}/corpus${qs}`,
			this.backend.auth,
		);
	}

	async getCorpusPayload(
		appId: string,
		eventId: string,
		runId: string,
	): Promise<IRegressionCorpusPayload> {
		return apiGet<IRegressionCorpusPayload>(
			`apps/${appId}/events/${eventId}/corpus/${runId}/payload`,
			this.backend.auth,
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
		return apiPost<IRegressionFixtureSummary>(
			`apps/${appId}/events/${eventId}/regression/fixtures`,
			{
				run_id: runId,
				expectation: options?.expectation,
				acknowledge_rejected: options?.acknowledgeRejected ?? false,
			},
			this.backend.auth,
		);
	}

	async deleteRegressionFixture(
		appId: string,
		eventId: string,
		fixtureId: string,
	): Promise<void> {
		await apiDelete<void>(
			`apps/${appId}/events/${eventId}/regression/fixtures/${fixtureId}`,
			this.backend.auth,
		);
	}

	// A 404 means "no suite saved yet", which the Quality section treats as an
	// empty state rather than an error.
	async getRegressionSuite(
		appId: string,
		eventId: string,
	): Promise<IRegressionSuiteResult | null> {
		try {
			return await apiGet<IRegressionSuiteResult>(
				`apps/${appId}/events/${eventId}/regression/suite`,
				this.backend.auth,
			);
		} catch (error) {
			if (isMissingResourceError(error)) return null;
			throw error;
		}
	}

	async putRegressionSuite(
		appId: string,
		eventId: string,
		config: IPutRegressionSuiteRequest,
	): Promise<IRegressionSuiteResult> {
		return apiPut<IRegressionSuiteResult>(
			`apps/${appId}/events/${eventId}/regression/suite`,
			{
				trigger_on_publish: config.trigger_on_publish,
				schedule: config.schedule ?? null,
				gate_mode: config.gate_mode,
				allow_live_side_effects: config.allow_live_side_effects,
			},
			this.backend.auth,
		);
	}

	async runRegressionSuite(
		appId: string,
		eventId: string,
		options?: {
			boardVersion?: [number, number, number];
			allowDraft?: boolean;
		},
	): Promise<IRegressionRunAccepted> {
		return apiPost<IRegressionRunAccepted>(
			`apps/${appId}/events/${eventId}/regression/run`,
			{
				board_version: options?.boardVersion,
				allow_draft: options?.allowDraft ?? false,
			},
			this.backend.auth,
		);
	}

	async listRegressionRuns(
		appId: string,
		eventId: string,
	): Promise<IRegressionSuiteRunSummary[]> {
		return apiGet<IRegressionSuiteRunSummary[]>(
			`apps/${appId}/events/${eventId}/regression/runs`,
			this.backend.auth,
		);
	}

	async getRegressionRun(
		appId: string,
		eventId: string,
		suiteRunId: string,
	): Promise<IRegressionSuiteRunDetail> {
		return apiGet<IRegressionSuiteRunDetail>(
			`apps/${appId}/events/${eventId}/regression/runs/${suiteRunId}`,
			this.backend.auth,
		);
	}

	async prerunEvent(
		appId: string,
		eventId: string,
		version?: [number, number, number],
		pageTrigger?: PageTrigger,
	): Promise<IPrerunEventResponse> {
		const params = version ? `?version=${version.join("_")}` : "";
		if (pageTrigger) {
			return apiPost<IPrerunEventResponse>(
				`apps/${appId}/events/${eventId}/prerun${params}`,
				{ page_trigger: serializePageTrigger(pageTrigger) },
				this.backend.auth,
			);
		}
		return apiGet<IPrerunEventResponse>(
			`apps/${appId}/events/${eventId}/prerun${params}`,
			this.backend.auth,
		);
	}
}
