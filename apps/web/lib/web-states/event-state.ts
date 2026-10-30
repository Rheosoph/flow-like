import {
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
	checkOAuthTokensFromPrerun,
	classifyPageContractError,
	getCurrentPageContext,
	notifyPageContractRejected,
	serializePageTrigger,
	withCurrentManifestRevision,
} from "@flow-like/flow-like-ui";
import {
	apiResponseError,
	isMissingResourceError,
} from "@flow-like/flow-like-ui/lib/api-error";
import { getApiOrigin } from "@flow-like/flow-like-ui/lib/api-url";
import { cancelDeviceCommands } from "@flow-like/flow-like-ui/lib/device-bridge";
import { getExecutionRuntime } from "@flow-like/flow-like-ui/lib/execution/runtime-client";
import {
	getExecutionMetadata,
	isExecutionFinished,
	type ExecutionMetadata,
} from "@flow-like/flow-like-ui/lib/execution/types";
import {
	isDeviceEventSource,
	resolveEventSourceIntent,
} from "@flow-like/flow-like-ui/lib/event-source";
import { ensureDeviceEventCreation } from "@flow-like/flow-like-ui/lib/event-source-capability";
import {
	requestTimeoutMs,
	withRequestDeadline,
} from "@flow-like/flow-like-ui/lib/request-deadline";
import {
	recordRunStep,
	runTimingNow,
	timeRunStep,
} from "@flow-like/flow-like-ui/lib/run-timing";
import type {
	TeamsBotAccess,
	TeamsBotConnection,
	TeamsBotPackage,
	TeamsBotSetup,
} from "@flow-like/flow-like-ui/lib/teams-bot";
import type {
	IEventUpsertOptions,
	IOAuthCheckResult,
} from "@flow-like/flow-like-ui/state/backend-state/event-state";
import type {
	ICanaryExplainResult,
	ICanaryPromoteResult,
	IEventAlias,
	IEventCorpusResult,
	IEventRunsResult,
	IEventSetupInfo,
	IEventSinkStatus,
	IEventSinkStatusContext,
	IEventTimeline,
	IEventTimelineRun,
	IEventVariantSharePatch,
	IEventVariantStatsResult,
	IEventVariantStatsWindow,
	IInboundEmailAddress,
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
	IUserSchedules,
} from "@flow-like/flow-like-ui/state/backend-state/event-state";
import type { IPrerunEventResponse } from "@flow-like/flow-like-ui/state/backend-state/types";
import { oauthConsentStore, oauthTokenStore } from "../oauth-db";
import { prepareSessionRequest } from "../session-request";
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

const PAGE_TRIGGER_PRERUN_REUSE_MS = 15_000;

function missingOAuthError(
	reason: string,
	missingProviders: IOAuthProvider[],
): Error {
	return Object.assign(
		new Error(`${reason}: ${missingProviders.map((p) => p.name).join(", ")}`),
		{ missingProviders, isOAuthError: true },
	);
}

function withFeedbackPageContext(localState?: Record<string, unknown>) {
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

	// Only a success is cached; a failed fetch clears the slot so the next call
	// retries instead of running every later event without its OAuth providers.
	hubCachePromise = withRequestDeadline(url, async ({ signal }) => {
		const res = await fetch(url, { signal });
		if (!res.ok) {
			throw apiResponseError(res, await res.text().catch(() => ""), url);
		}
		return (await res.json()) as IHub;
	})
		.then((hub) => {
			hubCache = hub;
			return hub;
		})
		.catch((e) => {
			console.warn("[OAuth] Failed to fetch Hub config:", e);
			return undefined;
		})
		.finally(() => {
			hubCachePromise = undefined;
		});

	return hubCachePromise;
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
	private readonly runScopes = new Map<string, string>();
	// Response identity keeps simultaneous dispatches from consuming each other's prerun.
	private readonly pagePreruns = new WeakMap<
		IPrerunEventResponse,
		{ scope: string; started: number }
	>();

	constructor(private readonly backend: WebBackendRef) {}

	private pagePrerunScope(
		appId: string,
		eventId: string,
		version: [number, number, number] | undefined,
		trigger: PageTrigger,
	): string {
		return JSON.stringify([
			getApiBaseUrl(),
			this.backend.profile?.hub ?? null,
			this.backend.profile?.id ?? null,
			this.backend.auth?.user?.profile?.sub ?? null,
			this.backend.auth?.user?.access_token ?? null,
			appId,
			eventId,
			version ?? null,
			serializePageTrigger(trigger),
		]);
	}

	private takePagePrerun(
		response: IPrerunEventResponse | undefined,
		appId: string,
		eventId: string,
		trigger: PageTrigger,
	): IPrerunEventResponse | undefined {
		if (!response) return undefined;
		const entry = this.pagePreruns.get(response);
		this.pagePreruns.delete(response);
		if (
			entry?.scope ===
				this.pagePrerunScope(appId, eventId, undefined, trigger) &&
			runTimingNow() - entry.started <= PAGE_TRIGGER_PRERUN_REUSE_MS
		) {
			return response;
		}
		return undefined;
	}

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

	async getEventAuthoritative(
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

	async getEventsAuthoritative(appId: string): Promise<IEvent[]> {
		return apiGet<IEvent[]>(`apps/${appId}/events`, this.backend.auth);
	}

	async getUserSchedules(limit = 100, appId?: string): Promise<IUserSchedules> {
		const params = new URLSearchParams({ limit: String(limit) });
		if (appId) params.set("app_id", appId);
		return apiGet<IUserSchedules>(
			`user/schedules?${params}`,
			this.backend.auth,
		);
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
		inputEvent: IEvent,
		versionType?: IVersionType,
		_personalAccessToken?: string,
		oauthTokens?: Record<string, IOAuthToken>,
		options?: IEventUpsertOptions,
	): Promise<IEvent> {
		const { event, intent } = resolveEventSourceIntent(
			inputEvent,
			options?.source,
		);
		if (options?.source === "device") {
			await ensureDeviceEventCreation(
				`${this.backend.profile?.hub ?? ""}|${appId}`,
				() =>
					apiGet<{ device_event_creation?: boolean }>(
						`apps/${appId}/device-placements`,
						this.backend.auth,
					),
			);
		}
		return apiPut<IEvent>(
			`apps/${appId}/events/${event.id}`,
			{
				event,
				version_type: versionType,
				oauth_tokens: oauthTokens,
				profile_id: this.backend.profile?.id,
				...(intent === "device" ? { register_source: false } : {}),
				...(intent === "clear" ? { register_source: true } : {}),
			},
			this.backend.auth,
		);
	}

	async checkEventOAuth(
		appId: string,
		event: IEvent,
	): Promise<IOAuthCheckResult> {
		try {
			const requirements = await this.eventOAuthRequirements(appId, event);
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
			history?: unknown[];
			globalState?: Record<string, unknown>;
			localState?: Record<string, unknown>;
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
		beforeDispatch?: () => void,
		preparedPrerun?: IPrerunEventResponse,
	): Promise<ILogMetadata | undefined> {
		beforeDispatch?.();
		return this.executeEventInternal(
			appId,
			eventId,
			payload,
			streamState,
			onEventId,
			cb,
			skipConsentCheck,
			pageTrigger,
			beforeDispatch,
			preparedPrerun,
		);
	}

	private async executeEventInternal(
		appId: string,
		eventId: string,
		payload: IRunPayload,
		streamState?: boolean,
		onEventId?: (id: string) => void,
		cb?: (event: IIntercomEvent[]) => void,
		skipConsentCheck?: boolean,
		pageTrigger?: PageTrigger,
		beforeDispatch?: () => void,
		preparedPrerun?: IPrerunEventResponse,
	): Promise<ILogMetadata | undefined> {
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
				pagePrerun = this.takePagePrerun(
					preparedPrerun,
					appId,
					eventId,
					pageTrigger,
				);
				if (pagePrerun) {
					const reusedAt = runTimingNow();
					recordRunStep("prerun.reused", reusedAt, reusedAt);
				} else {
					pagePrerun = await timeRunStep("prerun", () =>
						this.prerunEvent(appId, eventId, undefined, pageTrigger),
					);
					this.pagePreruns.delete(pagePrerun);
				}
			} catch (error) {
				reportPageContractRejection(appId, eventId, pageTrigger, error);
				throw error;
			}
			activeTrigger = withCurrentManifestRevision(
				pageTrigger,
				pagePrerun.manifest_revision,
			);
		}

		const requirements = pagePrerun
			? pagePrerun.oauth_requirements
			: await (async () => {
					const event = await timeRunStep("get_event", () =>
						this.getEvent(appId, eventId),
					);
					return timeRunStep("prerun", () =>
						this.eventOAuthRequirements(appId, event),
					);
				})();
		// Provider configuration is needed only by OAuth nodes. Other actions can
		// dispatch without waiting for a cold or unavailable Hub configuration.
		const hub = requirements?.length
			? await timeRunStep("hub_config", () =>
					getHubConfig(this.backend.profile),
				)
			: undefined;
		const oauthResult = await timeRunStep("oauth_tokens", () =>
			checkOAuthTokensFromPrerun(
				requirements,
				oauthTokenStore,
				hub,
				oauthOptions,
			),
		);

		console.log("[OAuth] Event check result:", {
			requiredProviders: oauthResult.requiredProviders.map((p) => p.id),
			missingProviders: oauthResult.missingProviders.map((p) => p.id),
			hasTokens: Object.keys(oauthResult.tokens),
			skipConsentCheck,
		});

		if (!skipConsentCheck) {
			const consentedIds = await timeRunStep("oauth_consent", () =>
				oauthConsentStore.getConsentedProviderIds(appId),
			);
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
				throw missingOAuthError(
					"Missing OAuth authorization for",
					providersNeedingConsent,
				);
			}
		} else {
			// Still need to check for missing tokens even if skipping consent
			if (oauthResult.missingProviders.length > 0) {
				throw missingOAuthError(
					"Missing OAuth tokens for",
					oauthResult.missingProviders,
				);
			}
		}

		// Collect OAuth tokens to pass to execution
		const oauthTokens =
			Object.keys(oauthResult.tokens).length > 0
				? oauthResult.tokens
				: undefined;

		const invokePath = `apps/${appId}/events/${eventId}/invoke`;
		const url = `${getApiBaseUrl()}/api/v1/${invokePath}`;

		const session = await prepareSessionRequest(
			{
				"Content-Type": "application/json",
				...(this.backend.auth?.user?.access_token
					? { Authorization: `Bearer ${this.backend.auth.user.access_token}` }
					: {}),
			},
			this.backend.auth,
		);
		const metadata =
			getExecutionMetadata(payload) ??
			this.executionMetadata(appId, eventId, payload.payload);
		const runtime = await getExecutionRuntime(metadata.scope);
		const body = JSON.stringify({
			payload: payload.payload,
			oauth_tokens: oauthTokens,
			runtime_variables: payload.runtime_variables,
			profile_id: this.backend.profile?.id,
			page_trigger: activeTrigger
				? serializePageTrigger(activeTrigger)
				: undefined,
		});
		let foundRunId = false;
		let dispatchStart: number | undefined;
		let runId: string | undefined;
		try {
			beforeDispatch?.();
			dispatchStart = runTimingNow();
			return await runtime.execute(
				metadata,
				{
					url,
					headers: Object.fromEntries(session.headers.entries()),
					body,
					cancelUrl: `${getApiBaseUrl()}/api/v1/execution/run/{runId}`,
					timeoutMs: requestTimeoutMs(invokePath, "POST", body.length),
				},
				{
					onResponse: () => recordRunStep("invoke.response", dispatchStart),
					onUnauthorized: session.onUnauthorized,
					onExecutionStart: (id) => {
						if (foundRunId) return;
						foundRunId = true;
						runId = id;
						this.runScopes.set(id, metadata.scope);
						recordRunStep("invoke.until_run_initiated", dispatchStart);
						onEventId?.(id);
					},
					onEvents: cb,
				},
			);
		} catch (cause) {
			if (!foundRunId && dispatchStart !== undefined)
				recordRunStep("invoke.until_error", dispatchStart);
			const response = cause as {
				status?: number;
				statusText?: string;
				body?: string;
				headers?: Record<string, string>;
			};
			const error =
				typeof response?.status === "number"
					? apiResponseError(
							{
								status: response.status,
								statusText: response.statusText ?? "",
								headers: new Headers(response.headers),
							},
							response.body ?? "",
							url,
						)
					: cause;
			if (response?.status === 401) session.onRejected?.();
			reportPageContractRejection(appId, eventId, activeTrigger, error);
			throw error;
		} finally {
			if (runId) this.runScopes.delete(runId);
		}
	}

	private executionScope(): string {
		return JSON.stringify([
			getApiOrigin(this.backend.profile),
			this.backend.profile?.id ?? "",
			(this.backend.auth?.isAuthenticated
				? this.backend.auth.user?.profile.sub
				: undefined) ?? "local",
		]);
	}

	private executionMetadata(
		appId: string,
		eventId: string,
		payload: unknown,
	): ExecutionMetadata {
		const taskId = crypto.randomUUID();
		const context =
			payload && typeof payload === "object"
				? (payload as Record<string, unknown>)
				: undefined;
		return {
			taskId,
			streamId: taskId,
			scope: this.executionScope(),
			appId,
			eventId,
			title: "Workflow",
			path: `/use?id=${encodeURIComponent(appId)}&eventId=${encodeURIComponent(eventId)}`,
			startedAt: new Date().toISOString(),
			pageId:
				typeof context?._page_id === "string" ? context._page_id : undefined,
			widgetInstanceId:
				typeof context?._widget_instance_id === "string"
					? context._widget_instance_id
					: undefined,
		};
	}

	async invokeMcp(
		appId: string,
		eventId: string,
		method: string,
		params?: Record<string, unknown>,
		beforeDispatch?: () => void,
	): Promise<Record<string, unknown>> {
		beforeDispatch?.();
		return apiPost<Record<string, unknown>>(
			`apps/${appId}/events/${eventId}/mcp-operation`,
			{ method, params: params ?? {}, profile_id: this.backend.profile?.id },
			this.backend.auth,
		);
	}

	async cancelExecution(runId: string): Promise<void> {
		const credentialScope = this.executionScope();
		const ownerScope = this.runScopes.get(runId);
		if (ownerScope && ownerScope !== credentialScope)
			throw new Error("The active account or workspace changed.");
		cancelDeviceCommands(runId);
		const runtime = await getExecutionRuntime(ownerScope ?? credentialScope);
		const active = runtime
			.getTasks()
			.some((task) => task.runId === runId && !isExecutionFinished(task));
		if (active) {
			const session = await prepareSessionRequest(
				this.backend.auth?.user?.access_token
					? { Authorization: `Bearer ${this.backend.auth.user.access_token}` }
					: undefined,
				this.backend.auth,
			);
			try {
				if (this.executionScope() !== credentialScope)
					throw new Error("The active account or workspace changed.");
				await runtime.cancel(runId, {
					headers: Object.fromEntries(session.headers.entries()),
					onUnauthorized: session.onUnauthorized,
				});
			} catch (error) {
				if ((error as { status?: number })?.status === 401)
					session.onRejected?.();
				throw error;
			}
			return;
		}
		if (this.executionScope() !== credentialScope)
			throw new Error("The active account or workspace changed.");
		await apiDelete(
			`execution/run/${encodeURIComponent(runId)}`,
			this.backend.auth,
		);
	}

	async isEventSinkActive(
		eventId: string,
		context?: IEventSinkStatusContext,
	): Promise<boolean> {
		return (await this.getEventSinkStatus(eventId, context)).active;
	}

	async getEventSinkStatus(
		eventId: string,
		context?: IEventSinkStatusContext,
	): Promise<IEventSinkStatus> {
		if (context) {
			if (isDeviceEventSource(context.event)) return { active: false };
			if (!context.event.active) return { active: false };
			// Hosted endpoints use the event's enabled flag. Their setup
			// registrations are separate from worker sink registrations.
			if (["rest", "mcp"].includes(context.event.event_type)) {
				return { active: context.event.active };
			}
		}
		try {
			const result = await apiGet<{
				active: boolean;
				run_as_user_id?: string | null;
				legacy_token?: boolean;
			}>(`sink/${eventId}`, this.backend.auth);
			return {
				active: result.active,
				run_as_user_id: result.run_as_user_id,
				legacy_token: result.legacy_token,
			};
		} catch (error) {
			if (isMissingResourceError(error)) return { active: false };
			throw error;
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

	async getTeamsBot(
		appId: string,
		eventId: string,
	): Promise<TeamsBotConnection> {
		return apiGet<TeamsBotConnection>(
			`apps/${appId}/events/${eventId}/teams`,
			this.backend.auth,
		);
	}

	async setupTeamsBot(
		appId: string,
		eventId: string,
		input: TeamsBotSetup,
	): Promise<TeamsBotConnection> {
		return apiPut<TeamsBotConnection>(
			`apps/${appId}/events/${eventId}/teams`,
			input,
			this.backend.auth,
		);
	}

	async getTeamsBotPackage(
		appId: string,
		eventId: string,
	): Promise<TeamsBotPackage> {
		return apiGet<TeamsBotPackage>(
			`apps/${appId}/events/${eventId}/teams/package`,
			this.backend.auth,
		);
	}

	async rotateTeamsBotSecret(
		appId: string,
		eventId: string,
	): Promise<TeamsBotConnection> {
		return apiPost<TeamsBotConnection>(
			`apps/${appId}/events/${eventId}/teams/rotate`,
			{},
			this.backend.auth,
		);
	}

	async disconnectTeamsBot(
		appId: string,
		eventId: string,
	): Promise<TeamsBotConnection> {
		return apiDelete<TeamsBotConnection>(
			`apps/${appId}/events/${eventId}/teams`,
			this.backend.auth,
		);
	}

	async getTeamsBotAccess(
		appId: string,
		eventId: string,
	): Promise<TeamsBotAccess> {
		return apiGet<TeamsBotAccess>(
			`apps/${appId}/events/${eventId}/teams/access`,
			this.backend.auth,
		);
	}

	async getInboundEmailAddress(
		appId: string,
		eventId: string,
	): Promise<IInboundEmailAddress> {
		return apiGet<IInboundEmailAddress>(
			`apps/${appId}/events/${eventId}/email-address`,
			this.backend.auth,
		);
	}

	async updateInboundEmailAlias(
		appId: string,
		eventId: string,
		alias: string | null,
	): Promise<IInboundEmailAddress> {
		return apiPut<IInboundEmailAddress>(
			`apps/${appId}/events/${eventId}/email-address`,
			{ alias },
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
			// Capture identity before the request; an account switch cannot reassign its result.
			const scope = this.pagePrerunScope(appId, eventId, version, pageTrigger);
			const started = runTimingNow();
			const result = await apiPost<IPrerunEventResponse>(
				`apps/${appId}/events/${eventId}/prerun${params}`,
				{ page_trigger: serializePageTrigger(pageTrigger) },
				this.backend.auth,
			);
			this.pagePreruns.set(result, { scope, started });
			return result;
		}
		return apiGet<IPrerunEventResponse>(
			`apps/${appId}/events/${eventId}/prerun${params}`,
			this.backend.auth,
		);
	}

	/**
	 * Members of a published app may run an event without reading its board,
	 * so OAuth requirements come from prerun (ExecuteEvents), pinned to the
	 * event's board version — prerun otherwise answers for the latest board.
	 */
	private async eventOAuthRequirements(appId: string, event: IEvent) {
		const version = event.board_version
			? (event.board_version as [number, number, number])
			: undefined;
		const prerun = await this.prerunEvent(appId, event.id, version);
		return prerun.oauth_requirements;
	}
}
