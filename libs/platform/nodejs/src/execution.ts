import type { HttpClient } from "./client.js";
import type { PollOptions, PollResult, RunStatus } from "./types.js";

export function createExecutionMethods(http: HttpClient) {
	return {
		async getRunStatus(runId: string): Promise<RunStatus> {
			return http.request<RunStatus>("GET", `/execution/run/${runId}`);
		},

		async pollExecution(
			pollToken: string,
			options?: PollOptions,
		): Promise<PollResult> {
			return http.request<PollResult>("GET", "/execution/poll", {
				query: {
					poll_token: pollToken,
					after_sequence: options?.afterSequence,
					timeout: options?.timeout,
				},
				signal: options?.signal,
			});
		},
	};
}
