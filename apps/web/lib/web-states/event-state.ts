import {
	type IBoard,
	type IEvent,
	type IEventState,
	type IHub,
	type IIntercomEvent,
	type ILogMetadata,
	type IOAuthProvider,
	type IOAuthToken,
	type IRunPayload,
	type IVersionType,
	type ProgressToastData,
	checkOAuthTokens,
	finishAllProgressToasts,
	showProgressToast,
} from "@flow-like/flow-like-ui";
import type { IOAuthCheckResult } from "@flow-like/flow-like-ui/state/backend-state/event-state";
import type {
	IEventAlias,
	IListRegistrationsResponse,
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
	apiPost,
	apiPut,
	getApiBaseUrl,
} from "./api-utils";

// Hub configuration cache
let hubCache: IHub | undefined;
let hubCachePromise: Promise<IHub | undefined> | undefined;

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

export class WebEventState implements IEventState {
	readonly alwaysRemote = true;

	constructor(private readonly backend: WebBackendRef) {}

	async getEvent(
		appId: string,
		eventId: string,
		version?: [number, number, number],
	): Promise<IEvent> {
		const params = version ? `?version=${version.join(".")}` : "";
		return apiGet<IEvent>(
			`apps/${appId}/events/${eventId}${params}`,
			this.backend.auth,
		);
	}

	async getEvents(appId: string, _force?: boolean): Promise<IEvent[]> {
		try {
			return await apiGet<IEvent[]>(`apps/${appId}/events`, this.backend.auth);
		} catch {
			return [];
		}
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
				? `?version=${event.board_version.join(".")}`
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

	async deleteEvent(appId: string, eventId: string): Promise<void> {
		await apiDelete(`apps/${appId}/events/${eventId}`, this.backend.auth);
	}

	async validateEvent(
		appId: string,
		eventId: string,
		version?: [number, number, number],
	): Promise<void> {
		const params = version ? `?version=${version.join(".")}` : "";
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
		const context =
			feedback.history || feedback.globalState || feedback.localState
				? {
						history: feedback.history,
						global_state: feedback.globalState,
						local_state: feedback.localState,
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
	): Promise<ILogMetadata | undefined> {
		// Get the event and its board for OAuth checking
		const event = await this.getEvent(appId, eventId);
		const boardParams = event.board_version
			? `?version=${event.board_version.join(".")}`
			: "";
		const board = await apiGet<IBoard>(
			`apps/${appId}/board/${event.board_id}${boardParams}`,
			this.backend.auth,
		);

		// Check OAuth tokens
		const hub = await getHubConfig(this.backend.profile);
		const oauthService = getOAuthService(
			getOAuthApiBaseUrl(this.backend.profile?.hub),
		);
		const oauthResult = await checkOAuthTokens(board, oauthTokenStore, hub, {
			refreshToken: oauthService.refreshToken.bind(oauthService),
		});

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
				}),
			});

			if (!response.ok) {
				throw new Error(`Event execution failed: ${response.status}`);
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
								dataLines.push(line.slice(5).trim());
							} else if (line.startsWith(":")) {
								continue;
							}
						}

						const eventData = dataLines.join("\n");

						if (!eventData || eventData === "keep-alive") continue;

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

							// Check for terminal events
							if (
								eventName === "done" ||
								eventName === "completed" ||
								event.event_type === "completed"
							) {
								executionFinished = true;
								finishAllProgressToasts(true);
								break;
							}
							if (event.event_type === "error") {
								executionFinished = true;
								finishAllProgressToasts(false);
								break;
							}
						} catch {
							// Ignore parse errors
						}
					}
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
	): Promise<IListRegistrationsResponse> {
		const qs = version ? `?version=${encodeURIComponent(version)}` : "";
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
	): Promise<ISetupEventResponse> {
		return apiPost<ISetupEventResponse>(
			`apps/${appId}/events/${eventId}/setup`,
			{ force },
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

	async prerunEvent(
		appId: string,
		eventId: string,
		version?: [number, number, number],
	): Promise<IPrerunEventResponse> {
		const params = version ? `?version=${version.join(".")}` : "";
		return apiGet<IPrerunEventResponse>(
			`apps/${appId}/events/${eventId}/prerun${params}`,
			this.backend.auth,
		);
	}
}
