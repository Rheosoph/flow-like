import type { HttpClient, QueryParams, SSEChunk } from "./client.js";
import { appPath, eventPath } from "./paths.js";
import type {
	AsyncInvokeResult,
	EventTriggerOptions,
	JsonObject,
	EventUpsertOptions,
	Version,
	VersionType,
} from "./types.js";

export function createEventMethods(http: HttpClient) {
	return {
		triggerEvent(
			appId: string,
			eventId: string,
			payload?: unknown,
			options?: EventTriggerOptions,
		): AsyncIterable<SSEChunk> {
			return http.streamSSE("POST", `${eventPath(appId, eventId)}/invoke`, {
				body: {
					payload,
					version: options?.version,
					token: options?.token,
					oauth_tokens: options?.oauth_tokens,
					runtime_variables: options?.runtime_variables,
					profile_id: options?.profile_id,
					correlation: options?.correlation,
					page_trigger: options?.page_trigger,
				},
				query: {
					local: options?.local,
					isolated: options?.isolated,
					__variant: options?.variant,
				},
				headers: options?.headers,
				signal: options?.signal,
			});
		},

		async triggerEventAsync(
			appId: string,
			eventId: string,
			payload?: unknown,
			options?: EventTriggerOptions,
		): Promise<AsyncInvokeResult> {
			return http.request<AsyncInvokeResult>(
				"POST",
				`${eventPath(appId, eventId)}/invoke/async`,
				{
					body: {
						payload,
						version: options?.version,
						token: options?.token,
						oauth_tokens: options?.oauth_tokens,
						runtime_variables: options?.runtime_variables,
						profile_id: options?.profile_id,
						correlation: options?.correlation,
						page_trigger: options?.page_trigger,
					},
					query: {
						local: options?.local,
						isolated: options?.isolated,
						__variant: options?.variant,
					},
					headers: options?.headers,
					signal: options?.signal,
				},
			);
		},
		listEvents(appId: string): Promise<JsonObject[]> {
			return http.request("GET", `${appPath(appId)}/events`);
		},
		getEvent(
			appId: string,
			eventId: string,
			version?: string,
		): Promise<JsonObject> {
			return http.request("GET", eventPath(appId, eventId), {
				query: { version },
			});
		},
		upsertEvent(
			appId: string,
			eventId: string,
			event: JsonObject,
			options: EventUpsertOptions = {},
		): Promise<JsonObject> {
			return http.request("PUT", eventPath(appId, eventId), {
				body: { event, ...options },
			});
		},
		async deleteEvent(appId: string, eventId: string): Promise<void> {
			await http.request("DELETE", eventPath(appId, eventId));
		},
		getEventVersions(appId: string, eventId: string): Promise<Version[]> {
			return http.request("GET", `${eventPath(appId, eventId)}/versions`);
		},
		getEventTimeline(appId: string, eventId: string): Promise<JsonObject> {
			return http.request("GET", `${eventPath(appId, eventId)}/timeline`);
		},
		getEventRuns(
			appId: string,
			eventId: string,
			options: QueryParams = {},
		): Promise<JsonObject> {
			return http.request("GET", `${eventPath(appId, eventId)}/runs`, {
				query: options,
			});
		},
		async validateEvent(
			appId: string,
			eventId: string,
			version?: string,
		): Promise<void> {
			await http.request("POST", `${eventPath(appId, eventId)}/validate`, {
				query: { version },
			});
		},
		setupEvent(
			appId: string,
			eventId: string,
			options: JsonObject = {},
		): Promise<JsonObject> {
			return http.request("POST", `${eventPath(appId, eventId)}/setup`, {
				body: options,
			});
		},
		restoreEvent(
			appId: string,
			eventId: string,
			version: Version,
			options: {
				version_type?: VersionType;
				dry_run?: boolean;
				restore_route?: boolean;
				drop_canary?: boolean;
				accept_blank_secrets?: boolean;
			} = {},
		): Promise<JsonObject> {
			return http.request("POST", `${eventPath(appId, eventId)}/restore`, {
				body: { version, ...options },
			});
		},
		prerunEvent(
			appId: string,
			eventId: string,
			options: QueryParams = {},
		): Promise<JsonObject> {
			return http.request("GET", `${eventPath(appId, eventId)}/prerun`, {
				query: options,
			});
		},
		listEventSetups(appId: string, eventId: string): Promise<JsonObject[]> {
			return http.request("GET", `${eventPath(appId, eventId)}/setups`);
		},
		listEventRegistrations(
			appId: string,
			eventId: string,
		): Promise<JsonObject[]> {
			return http.request("GET", `${eventPath(appId, eventId)}/registrations`);
		},
		listSchedules(
			options: { app_id?: string; limit?: number } = {},
		): Promise<JsonObject> {
			return http.request("GET", "/user/schedules", { query: options });
		},
		explainEventCanary(
			appId: string,
			eventId: string,
			key: string,
			source?: string,
		): Promise<JsonObject> {
			return http.request(
				"GET",
				`${eventPath(appId, eventId)}/canary/explain`,
				{ query: { key, source } },
			);
		},
		getEventCanaryStats(
			appId: string,
			eventId: string,
			window?: string,
		): Promise<JsonObject> {
			return http.request("GET", `${eventPath(appId, eventId)}/canary/stats`, {
				query: { window },
			});
		},
		updateEventCanary(
			appId: string,
			eventId: string,
			patch: { name: string; weight?: number; sample_rate?: number },
		): Promise<JsonObject> {
			return http.request("PATCH", `${eventPath(appId, eventId)}/canary`, {
				body: patch,
			});
		},
		setEventVariants(
			appId: string,
			eventId: string,
			variants: JsonObject[],
		): Promise<JsonObject> {
			return http.request("PUT", `${eventPath(appId, eventId)}/variants`, {
				body: { variants },
			});
		},
		promoteEventCanary(
			appId: string,
			eventId: string,
			variant: string,
			versionType?: VersionType,
		): Promise<JsonObject> {
			return http.request(
				"POST",
				`${eventPath(appId, eventId)}/canary/promote`,
				{ body: { variant, version_type: versionType } },
			);
		},
		abortEventCanary(
			appId: string,
			eventId: string,
			variant: string,
		): Promise<JsonObject> {
			return http.request("POST", `${eventPath(appId, eventId)}/canary/abort`, {
				body: { variant },
			});
		},
	};
}
