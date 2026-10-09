import type { HttpClient } from "./client.js";
import { boardPath, segment } from "./paths.js";
import type {
	JsonObject,
	PollOptions,
	PollResult,
	RunStatus,
	RunListOptions,
	RunLogOptions,
	LogQuery,
} from "./types.js";

export function createExecutionMethods(http: HttpClient) {
	return {
		getRunStatus(runId: string): Promise<RunStatus> {
			return http.request("GET", `/execution/run/${segment(runId)}`);
		},
		cancelRun(
			runId: string,
		): Promise<{ run_id: string; status: string; cancelled: boolean }> {
			return http.request("DELETE", `/execution/run/${segment(runId)}`);
		},
		async pollExecution(
			pollToken: string,
			options?: PollOptions,
		): Promise<PollResult> {
			const after = options?.afterSequence ?? -1;
			const response = await http.request<Omit<PollResult, "lastSequence">>(
				"GET",
				"/execution/poll",
				{
					auth: false,
					headers: { Authorization: `Bearer ${pollToken}` },
					query: { after_sequence: after, timeout: options?.timeout },
					signal: options?.signal,
				},
			);
			return {
				...response,
				lastSequence: response.events.reduce(
					(cursor, event) => Math.max(cursor, event.sequence),
					after,
				),
			};
		},
		listRuns(
			appId: string,
			boardId: string,
			options: RunListOptions = {},
		): Promise<JsonObject[]> {
			return http.request("GET", `${boardPath(appId, boardId)}/runs`, {
				query: { ...options },
			});
		},
		getRunLogs(
			appId: string,
			boardId: string,
			runId: string,
			options: RunLogOptions = {},
		): Promise<JsonObject[]> {
			return http.request("POST", `${boardPath(appId, boardId)}/logs/query`, {
				body: {
					run_id: runId,
					query: options.query ?? {},
					offset: options.offset ?? 0,
					limit: options.limit ?? 100,
				},
			});
		},
		countRunLogs(
			appId: string,
			boardId: string,
			runId: string,
			query: LogQuery = {},
		): Promise<{ count: number }> {
			return http.request("POST", `${boardPath(appId, boardId)}/logs/count`, {
				body: { run_id: runId, query },
			});
		},
		getRunSummary(
			appId: string,
			boardId: string,
			runId: string,
		): Promise<JsonObject | null> {
			return http.request("GET", `${boardPath(appId, boardId)}/logs/summary`, {
				query: { run_id: runId },
			});
		},
		getRunPayload(
			appId: string,
			boardId: string,
			runId: string,
		): Promise<unknown> {
			return http.request(
				"GET",
				`${boardPath(appId, boardId)}/runs/${segment(runId)}/payload`,
			);
		},
		getExecutionElements(
			appId: string,
			boardId: string,
			pageId: string,
			options: { version?: string; wildcard?: boolean } = {},
		): Promise<{ elements: JsonObject }> {
			return http.request("GET", `${boardPath(appId, boardId)}/elements`, {
				query: { page_id: pageId, ...options },
			});
		},
	};
}
