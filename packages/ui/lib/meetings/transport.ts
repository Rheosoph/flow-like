import { apiResponseError } from "../api-error";
import { withRequestDeadline } from "../request-deadline";
import type { MeetingRequest } from "./client";

interface MeetingTransportOptions {
	accountRequest: <T>(path: string, init: RequestInit) => Promise<T>;
	apiUrl: (path: string) => string;
	fetch?: typeof globalThis.fetch;
}

/** Meeting sessions use their own capability, including when an account is signed in. */
export function createMeetingTransport(
	options: MeetingTransportOptions,
): MeetingRequest {
	return async <T>(
		path: string,
		init: RequestInit,
		{ authenticated }: { authenticated: boolean },
	): Promise<T> => {
		if (authenticated) return options.accountRequest<T>(path, init);
		const headers = new Headers(init.headers);
		headers.delete("Authorization");
		return withRequestDeadline(
			path,
			async ({ signal }) => {
				const response = await (options.fetch ?? globalThis.fetch)(
					options.apiUrl(path),
					{
						...init,
						headers,
						signal,
						credentials: "omit",
						cache: "no-store",
						referrerPolicy: "no-referrer",
						redirect: "error",
					},
				);
				const text = await response.text();
				if (!response.ok) throw apiResponseError(response, text, path);
				return (text ? JSON.parse(text) : undefined) as T;
			},
			{ signal: init.signal, timeoutMs: 15_000 },
		);
	};
}

/** A native app shares its hub's web address; a browser can use its own origin. */
export function meetingPageUrl(
	webBase?: string | null,
	browserOrigin?: string,
): string | undefined {
	const base = webBase || browserOrigin;
	if (!base) return undefined;
	try {
		const url = new URL(base);
		if (!/^https?:$/.test(url.protocol) || url.username || url.password)
			return undefined;
		url.pathname = `${url.pathname.replace(/\/+$/, "")}/meeting`;
		url.search = "";
		url.hash = "";
		return url.href;
	} catch {
		return undefined;
	}
}
