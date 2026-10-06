import { runtimeAssetBlob } from "./resource-content";

export const RUNTIME_ASSET_LIMIT = 32 * 1024 * 1024;
export const RUNTIME_ASSET_SESSION_LIMIT = 128 * 1024 * 1024;
const ASSET_COUNT_LIMIT = 64;

export interface RuntimeResourceOptions {
	/** Authenticated request scoped to this deployed service. */
	fetch: (path: string, init?: RequestInit) => Promise<Response>;
	appId?: string;
	signal?: AbortSignal;
}

/** Resolve protected device resources without putting credentials into element URLs. */
export function createRuntimeResources(options: RuntimeResourceOptions) {
	const lifetime = new AbortController();
	const cache = new Map<string, Promise<string>>();
	const urls = new Set<string>();
	let bytes = 0;
	let active = 0;
	const waiting: Array<() => void> = [];
	const close = () => {
		lifetime.abort();
		for (const url of urls) URL.revokeObjectURL(url);
		urls.clear();
		cache.clear();
		while (waiting.length) waiting.shift()?.();
		options.signal?.removeEventListener("abort", close);
	};
	options.signal?.addEventListener("abort", close, { once: true });
	if (options.signal?.aborted) close();
	const acquire = async () => {
		if (active >= 4)
			await new Promise<void>((resolve) => waiting.push(resolve));
		else active++;
		lifetime.signal.throwIfAborted();
	};
	const download = async (path: string): Promise<string> => {
		await acquire();
		let retained = 0;
		let complete = false;
		let reader: ReadableStreamDefaultReader<Uint8Array> | undefined;
		try {
			const response = await options.fetch(path, { signal: lifetime.signal });
			if (!response.ok) {
				await response.body?.cancel();
				throw new Error(
					`The deployed asset could not be loaded (${response.status}).`,
				);
			}
			const declared = response.headers.get("content-length");
			if (
				declared &&
				(!/^\d+$/.test(declared) || Number(declared) > RUNTIME_ASSET_LIMIT)
			) {
				await response.body?.cancel();
				throw new Error("Deployed assets are limited to 32 MiB each.");
			}
			reader = response.body?.getReader();
			const chunks: BlobPart[] = [];
			while (reader) {
				lifetime.signal.throwIfAborted();
				const next = await reader.read();
				if (next.done) break;
				if (
					retained + next.value.byteLength > RUNTIME_ASSET_LIMIT ||
					bytes + next.value.byteLength > RUNTIME_ASSET_SESSION_LIMIT
				)
					throw new Error("Deployed assets exceed the session's memory limit.");
				retained += next.value.byteLength;
				bytes += next.value.byteLength;
				chunks.push(next.value.slice().buffer as ArrayBuffer);
			}
			lifetime.signal.throwIfAborted();
			const blob = await runtimeAssetBlob(
				chunks,
				response.headers.get("content-type"),
			);
			lifetime.signal.throwIfAborted();
			if (
				blob.size > RUNTIME_ASSET_LIMIT ||
				bytes - retained + blob.size > RUNTIME_ASSET_SESSION_LIMIT
			)
				throw new Error("Deployed assets exceed the session's memory limit.");
			bytes += blob.size - retained;
			retained = blob.size;
			const url = URL.createObjectURL(blob);
			urls.add(url);
			complete = true;
			return url;
		} finally {
			if (!complete) bytes -= retained;
			await reader?.cancel().catch(() => {});
			reader?.releaseLock();
			const next = waiting.shift();
			if (next) next();
			else active--;
		}
	};
	const resolve = async (
		input: string,
		signal?: AbortSignal,
	): Promise<string> => {
		lifetime.signal.throwIfAborted();
		signal?.throwIfAborted();
		const path = resourcePath(input, options.appId);
		if (path === null) return input;
		let pending = cache.get(path);
		if (!pending) {
			if (cache.size >= ASSET_COUNT_LIMIT)
				throw new Error(
					"A deployed service session can load at most 64 assets. Reopen it to release earlier assets.",
				);
			pending = download(path);
			cache.set(path, pending);
			void pending.catch(() => {
				cache.delete(path);
			});
		}
		if (!signal) return pending;
		return new Promise<string>((done, reject) => {
			const cancel = () =>
				reject(
					new DOMException("The asset request was cancelled.", "AbortError"),
				);
			signal.addEventListener("abort", cancel, { once: true });
			void pending
				.then(done, reject)
				.finally(() => signal.removeEventListener("abort", cancel));
			if (signal.aborted) cancel();
		});
	};
	return {
		resolve,
		/** Map only server-issued resource URLs. Ordinary text and authored paths keep their meaning. */
		async mapValue<T>(value: T, signal?: AbortSignal): Promise<T> {
			let count = 0;
			const walk = async (input: unknown, depth: number): Promise<unknown> => {
				if (++count > 100_000 || depth > 64)
					throw new Error(
						"The deployed service response is too deeply nested.",
					);
				if (typeof input === "string" && input.startsWith("/ui/assets"))
					return resolve(input, signal);
				if (typeof input === "string" && isLocalAssetUrl(input))
					throw new Error(
						"This device-local asset needs a current service-issued resource URL.",
					);
				if (Array.isArray(input))
					return Promise.all(input.map((item) => walk(item, depth + 1)));
				if (input && typeof input === "object")
					return Object.fromEntries(
						await Promise.all(
							Object.entries(input).map(async ([key, item]) => [
								key,
								await walk(item, depth + 1),
							]),
						),
					);
				return input;
			};
			return (await walk(value, 0)) as T;
		},
		close,
	};
}

function resourcePath(input: string, appId?: string): string | null {
	if (input.startsWith("storage://"))
		return resourcePath(input.slice("storage://".length), appId);
	if (/^\/ui\/assets\/(?:[0-9a-f]{32}\/)?[0-9a-f]{32}$/.test(input))
		return input;
	if (input.startsWith("/ui/assets?")) {
		const query = new URLSearchParams(input.slice("/ui/assets?".length));
		if (
			query.size !== 2 ||
			query.get("store") !== "upload" ||
			!query.has("path")
		)
			throw new Error("The deployed asset URL is invalid.");
		return uploadPath(query.get("path") ?? "");
	}
	if (isLocalAssetUrl(input))
		throw new Error(
			"This device-local asset needs a current service-issued resource URL.",
		);
	if (/^(?:https?:|data:|blob:)/i.test(input)) return null;
	if (input.startsWith("apps/")) {
		const prefix = `apps/${appId}/upload/`;
		if (!appId || !input.startsWith(prefix))
			throw new Error("The asset belongs to another application.");
		return uploadPath(input.slice(prefix.length));
	}
	return uploadPath(input);
}

function isLocalAssetUrl(input: string): boolean {
	return /^(?:asset:|file:|tauri:|https?:\/\/(?:asset|tauri)\.localhost(?:[/:]|$))/i.test(
		input,
	);
}

function uploadPath(path: string): string {
	if (
		!path ||
		path.length > 4096 ||
		path.startsWith("/") ||
		/[\\?#:]/.test(path) ||
		hasControl(path)
	)
		throw new Error(
			"Use a relative path inside the deployed application's uploads.",
		);
	const parts = path.split("/");
	for (const part of parts) {
		let decoded: string;
		try {
			decoded = decodeURIComponent(part);
		} catch {
			throw new Error("The deployed asset path is invalid.");
		}
		if (
			!decoded ||
			decoded === "." ||
			decoded === ".." ||
			/[%/\\]/.test(decoded) ||
			hasControl(decoded)
		)
			throw new Error("The deployed asset path is invalid.");
	}
	return `/ui/assets?${new URLSearchParams({ store: "upload", path }).toString()}`;
}

function hasControl(value: string): boolean {
	return [...value].some(
		(character) =>
			character.charCodeAt(0) < 32 || character.charCodeAt(0) === 127,
	);
}
