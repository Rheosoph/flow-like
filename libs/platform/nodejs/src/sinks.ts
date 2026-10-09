import { segment } from "./paths.js";
import type { HttpClient } from "./client.js";
import type { HttpSinkOptions } from "./types.js";

export function createSinkMethods(http: HttpClient) {
	return {
		async triggerHttpSink(
			appId: string,
			path: string,
			method = "POST",
			body?: unknown,
			options?: HttpSinkOptions,
		): Promise<unknown> {
			return http.request(
				method,
				`/sink/trigger/http/${segment(appId)}/${path.split("/").filter(Boolean).map(segment).join("/")}`,
				{
					body,
					headers: options?.headers,
					signal: options?.signal,
				},
			);
		},
	};
}
