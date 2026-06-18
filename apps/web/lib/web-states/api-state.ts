import type { IApiState } from "@flow-like/flow-like-ui";
import { getApiUrl } from "@flow-like/flow-like-ui/lib/api-url";
import type { IProfile } from "@flow-like/flow-like-ui/types";
import { type WebBackendRef, ensureProtectedAppRouteAuth } from "./api-utils";

export class WebApiState implements IApiState {
	constructor(private readonly backend: WebBackendRef) {}

	private tryParseJSON<T>(text: string): T | null {
		try {
			return JSON.parse(text) as T;
		} catch {
			return null;
		}
	}

	private parseSSEBuffer(buffer: string): {
		events: Array<{ data: string; event?: string; id?: string }>;
		remaining: string;
	} {
		const events: Array<{ data: string; event?: string; id?: string }> = [];
		const parts = buffer.split("\n\n");
		const remaining = parts.pop() ?? "";

		for (const part of parts) {
			if (!part.trim()) continue;

			let event: string | undefined;
			let data = "";
			let id: string | undefined;

			for (const line of part.split("\n")) {
				if (line.startsWith("event:")) {
					event = line.slice(6).trim();
				} else if (line.startsWith("data:")) {
					data = line.slice(5).trim();
				} else if (line.startsWith("id:")) {
					id = line.slice(3).trim();
				}
			}

			if (data) {
				events.push({ data, event, id });
			}
		}

		return { events, remaining };
	}

	private buildSSEError(
		message: { data: string; event?: string },
		parsedData: Record<string, unknown> | null,
	): Error {
		const errorMessage =
			typeof parsedData?.message === "string"
				? parsedData.message
				: typeof parsedData?.error === "string"
					? parsedData.error
					: message.data || "SSE stream error";

		return new Error(errorMessage);
	}

	private processSSEEvent<T>(
		message: { data: string; event?: string },
		onMessage?: (data: T) => void,
	): { type: string; error?: Error } {
		const parsedData = this.tryParseJSON<T>(message.data);
		if (parsedData && onMessage) {
			onMessage(parsedData);
		}

		const data = parsedData as Record<string, unknown> | null;
		const evt = message.event ?? "message";
		const eventType = data?.event_type ?? data?.type;

		if (evt === "done" || evt === "completed" || eventType === "completed") {
			return { type: "completed" };
		}

		if (evt === "error" || eventType === "error") {
			return {
				type: "error",
				error: this.buildSSEError(message, data),
			};
		}

		return { type: evt };
	}

	private constructUrl(profile: IProfile, path: string): string {
		return getApiUrl(profile, path);
	}

	private getHeaders(): HeadersInit {
		const headers: HeadersInit = {
			"Content-Type": "application/json",
		};
		if (this.backend.auth?.user?.access_token) {
			headers["Authorization"] =
				`Bearer ${this.backend.auth.user.access_token}`;
		}
		return headers;
	}

	async fetch<T>(
		profile: IProfile,
		path: string,
		options?: RequestInit,
	): Promise<T> {
		ensureProtectedAppRouteAuth(
			path,
			this.backend.auth,
			(options?.method ?? "GET").toUpperCase(),
		);
		const url = this.constructUrl(profile, path);
		const response = await fetch(url, {
			...options,
			headers: {
				...this.getHeaders(),
				...options?.headers,
			},
		});

		if (!response.ok) {
			throw new Error(`API error: ${response.status}`);
		}

		return response.json();
	}

	async get<T>(profile: IProfile, path: string): Promise<T> {
		return this.fetch<T>(profile, path, { method: "GET" });
	}

	async post<T>(profile: IProfile, path: string, data?: unknown): Promise<T> {
		return this.fetch<T>(profile, path, {
			method: "POST",
			body: data ? JSON.stringify(data) : undefined,
		});
	}

	async put<T>(profile: IProfile, path: string, data?: unknown): Promise<T> {
		return this.fetch<T>(profile, path, {
			method: "PUT",
			body: data ? JSON.stringify(data) : undefined,
		});
	}

	async patch<T>(profile: IProfile, path: string, data?: unknown): Promise<T> {
		return this.fetch<T>(profile, path, {
			method: "PATCH",
			body: data ? JSON.stringify(data) : undefined,
		});
	}

	async del<T>(profile: IProfile, path: string, data?: unknown): Promise<T> {
		return this.fetch<T>(profile, path, {
			method: "DELETE",
			body: data ? JSON.stringify(data) : undefined,
		});
	}

	async stream<T>(
		profile: IProfile,
		path: string,
		options?: RequestInit,
		onMessage?: (data: T) => void,
	): Promise<void> {
		const url = this.constructUrl(profile, path);
		const response = await fetch(url, {
			...options,
			headers: {
				...this.getHeaders(),
				...options?.headers,
			},
		});

		if (!response.ok) {
			throw new Error(`Stream error: ${response.status}`);
		}

		if (!response.body) return;

		const reader = response.body.getReader();
		const decoder = new TextDecoder();
		let buffer = "";

		try {
			while (true) {
				const { done, value } = await reader.read();
				if (done) {
					if (buffer.trim()) {
						const { events } = this.parseSSEBuffer(buffer + "\n\n");
						for (const event of events) {
							const result = this.processSSEEvent(event, onMessage);
							if (result.type === "error") {
								throw result.error ?? new Error("SSE stream error");
							}
							if (result.type === "completed") {
								return;
							}
						}
					}
					break;
				}

				buffer += decoder.decode(value, { stream: true });
				const { events, remaining } = this.parseSSEBuffer(buffer);
				buffer = remaining;

				for (const event of events) {
					const result = this.processSSEEvent(event, onMessage);
					if (result.type === "error") {
						throw result.error ?? new Error("SSE stream error");
					}
					if (result.type === "completed") {
						return;
					}
				}
			}
		} finally {
			reader.releaseLock();
		}
	}
}
