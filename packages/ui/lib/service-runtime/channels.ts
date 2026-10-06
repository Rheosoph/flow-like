import { isChannelHandle } from "../channel/handle";
import {
	HTTP_PUSH_TIMEOUT_MS,
	registerScopedHttpChannel,
} from "../channel/http";
import { readBodyExcerpt, timeoutSignal } from "../channel/util";
import type { IChannelHandle } from "../schema/channel";
import type { BrowserFetch } from "./transport";

/** Bind run reply handles to the service that minted them, including after JSON serialization. */
export function createRuntimeChannels(
	fetcher: BrowserFetch,
	signal?: AbortSignal,
) {
	const controller = new AbortController();
	const entries = new Map<
		string,
		{ url: string; expires: number; close(): void }
	>();
	const close = () => {
		controller.abort();
		for (const entry of entries.values()) entry.close();
		entries.clear();
		signal?.removeEventListener("abort", close);
	};
	signal?.addEventListener("abort", close, { once: true });
	if (signal?.aborted) close();
	const bindHandle = (handle: IChannelHandle): IChannelHandle => {
		controller.signal.throwIfAborted();
		const descriptor = handle.transport;
		if (
			descriptor.type !== "http" ||
			!/^[A-Za-z0-9_-]{1,128}$/.test(handle.channel_id) ||
			!new RegExp(
				`^/channels/(?:[A-Za-z0-9_-]{1,128}/)?${handle.channel_id}$`,
			).test(descriptor.push_url) ||
			!/^[A-Za-z0-9_-]{43}$/.test(descriptor.token) ||
			!Number.isFinite(handle.expires_at)
		)
			throw new Error(
				"The deployed service returned an invalid run reply handle.",
			);
		const now = Date.now() / 1000;
		for (const [key, entry] of entries) {
			if (entry.expires <= now) {
				entry.close();
				entries.delete(key);
			}
		}
		const key = `${descriptor.push_url}:${descriptor.token}:${handle.expires_at}`;
		let entry = entries.get(key);
		if (!entry) {
			if (entries.size >= 512)
				throw new Error("Too many active deployed service reply handles.");
			const original = { ...descriptor };
			const expires = handle.expires_at;
			const channelId = handle.channel_id;
			const registration = registerScopedHttpChannel(
				async (scoped, push, options) => {
					controller.signal.throwIfAborted();
					if (
						scoped.token !== original.token ||
						push.channel_id !== channelId ||
						Date.now() / 1000 >= expires
					)
						throw new Error(
							"The deployed service reply handle is invalid or expired.",
						);
					const timeout = timeoutSignal(HTTP_PUSH_TIMEOUT_MS, options.signal);
					const delivery = new AbortController();
					const cancel = () => delivery.abort();
					controller.signal.addEventListener("abort", cancel, { once: true });
					timeout.signal.addEventListener("abort", cancel, { once: true });
					if (timeout.signal.aborted) cancel();
					try {
						const response = await fetcher(original.push_url, {
							method: "POST",
							headers: {
								"content-type": "application/json",
								authorization: `Bearer ${original.token}`,
							},
							body: JSON.stringify(push),
							signal: delivery.signal,
						});
						if (!response.ok)
							throw new Error(
								`The deployed service rejected the reply (${response.status}): ${await readBodyExcerpt(response)}`,
							);
						await response.body?.cancel();
					} finally {
						timeout.dispose();
						controller.signal.removeEventListener("abort", cancel);
						timeout.signal.removeEventListener("abort", cancel);
					}
				},
			);
			entry = { ...registration, expires };
			entries.set(key, entry);
		}
		return {
			...handle,
			transport: { ...descriptor, push_url: entry.url },
			fallback: undefined,
		};
	};
	return {
		bind<T>(value: T): T {
			let count = 0;
			const walk = (input: unknown, depth: number): unknown => {
				if (++count > 100_000 || depth > 64)
					throw new Error(
						"The deployed service response is too deeply nested.",
					);
				if (isChannelHandle(input)) return bindHandle(input);
				if (Array.isArray(input))
					return input.map((item) => walk(item, depth + 1));
				if (input && typeof input === "object")
					return Object.fromEntries(
						Object.entries(input).map(([key, item]) => [
							key,
							walk(item, depth + 1),
						]),
					);
				return input;
			};
			return walk(value, 0) as T;
		},
		close,
	};
}
