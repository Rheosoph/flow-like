import type { IBackendState } from "@flow-like/flow-like-ui/state/backend-state";

export type NativeIconFetch = (
	input: RequestInfo | URL,
	init?: RequestInit,
) => Promise<Response>;

export interface NativeAppIconPublication {
	scope: string;
	icons: { appId: string; data: string | null }[];
}

const IMAGE_BYTES = 32_768;
const SOURCE_BYTES = 4_194_304;
const BATCH_BYTES = 262_144;
const MAX_APPS = 128;
const SOURCE_TIMEOUT_MS = 10_000;
const CACHE_MS = 15 * 60_000;
const encodedSize = (value: unknown) =>
	new TextEncoder().encode(JSON.stringify(value)).byteLength;

function checkActive(signal: AbortSignal) {
	if (signal.aborted)
		throw new DOMException("Icon request cancelled", "AbortError");
}

/** App icons follow the current native catalog; image failures use native fallbacks. */
export function createNativeAppIconPublisher(options: {
	publish: (publication: NativeAppIconPublication) => Promise<void>;
	fetch?: NativeIconFetch;
	rasterize?: (blob: Blob, signal: AbortSignal) => Promise<string>;
}) {
	let controller: AbortController | undefined;
	let scope: string | undefined;
	let disposed = false;
	const cache = new Map<
		string,
		{ source: string; data: string; expires: number }
	>();
	const published = new Map<string, string | null>();
	const fetchIcon = options.fetch ?? globalThis.fetch;
	const rasterize = options.rasterize ?? rasterizeNativeAppIcon;

	async function sync(input: {
		backend: IBackendState;
		scope: string;
		appIds: string[];
		signal?: AbortSignal;
	}) {
		if (disposed) return;
		controller?.abort();
		const current = new AbortController();
		controller = current;
		const abort = () => current.abort();
		input.signal?.addEventListener("abort", abort, { once: true });
		if (input.signal?.aborted) current.abort();
		if (scope !== input.scope) {
			scope = input.scope;
			cache.clear();
			published.clear();
		}
		const signal = current.signal;
		try {
			checkActive(signal);
			const ids = [...new Set(input.appIds)]
				.filter((id) => new TextEncoder().encode(id).length <= 512)
				.slice(0, MAX_APPS);
			const allowed = new Set(ids);
			for (const key of cache.keys()) if (!allowed.has(key)) cache.delete(key);
			for (const key of published.keys())
				if (!allowed.has(key)) published.delete(key);
			const apps = await input.backend.appState.getApps();
			checkActive(signal);
			const sources = new Map(apps.map(([app, meta]) => [app.id, meta?.icon]));
			for (let offset = 0; offset < ids.length; offset += 32) {
				const pending: NativeAppIconPublication["icons"] = [];
				const group = ids.slice(offset, offset + 32);
				let index = 0;
				await Promise.all(
					Array.from({ length: Math.min(4, group.length) }, async () => {
						while (index < group.length) {
							const appId = group[index++];
							checkActive(signal);
							const source = sources.get(appId);
							let data: string | null = null;
							if (source) {
								const cached = cache.get(appId);
								if (cached?.source === source && cached.expires > Date.now())
									data = cached.data;
								else {
									const acquisition = new AbortController();
									const cancel = () => acquisition.abort();
									signal.addEventListener("abort", cancel, { once: true });
									const timeout = setTimeout(cancel, SOURCE_TIMEOUT_MS);
									try {
										const blob = await readNativeAppIcon(
											source,
											fetchIcon,
											acquisition.signal,
										);
										data = await rasterize(blob, acquisition.signal);
										checkActive(acquisition.signal);
										if (!isBoundedPNG(data))
											throw new Error("Invalid app icon");
										cache.set(appId, {
											source,
											data,
											expires: Date.now() + CACHE_MS,
										});
									} catch {
										if (!signal.aborted) cache.delete(appId);
										data = null;
									} finally {
										clearTimeout(timeout);
										acquisition.abort();
										signal.removeEventListener("abort", cancel);
									}
								}
							} else cache.delete(appId);
							checkActive(signal);
							if (!published.has(appId) || published.get(appId) !== data)
								pending.push({ appId, data });
						}
					}),
				);
				let batch: NativeAppIconPublication["icons"] = [];
				const publish = async () => {
					if (!batch.length) return;
					checkActive(signal);
					const sending = batch;
					await options.publish({ scope: input.scope, icons: sending });
					checkActive(signal);
					for (const icon of sending) published.set(icon.appId, icon.data);
					batch = [];
				};
				for (const icon of pending) {
					if (
						encodedSize({ scope: input.scope, icons: [...batch, icon] }) >
						BATCH_BYTES
					)
						await publish();
					batch.push(icon);
				}
				await publish();
			}
		} catch {
			// The snapshot remains usable when artwork cannot be refreshed.
		} finally {
			input.signal?.removeEventListener("abort", abort);
		}
	}

	return {
		sync,
		dispose() {
			disposed = true;
			controller?.abort();
			cache.clear();
			published.clear();
		},
	};
}

function isBoundedPNG(encoded: string): boolean {
	if (encoded.length > 43_692) return false;
	try {
		const bytes = atob(encoded);
		return bytes.length <= IMAGE_BYTES && bytes.startsWith("\x89PNG\r\n\x1a\n");
	} catch {
		return false;
	}
}

export async function readNativeAppIcon(
	source: string,
	fetchIcon: NativeIconFetch,
	signal: AbortSignal,
): Promise<Blob> {
	checkActive(signal);
	const url = new URL(source, globalThis.location?.href ?? "https://localhost");
	if (
		!["https:", "http:", "asset:", "blob:", "data:"].includes(url.protocol) ||
		url.username ||
		url.password
	)
		throw new Error("Unsupported app icon URL");
	const response = await fetchIcon(url.href, {
		signal,
		credentials: "omit",
		referrerPolicy: "no-referrer",
	});
	checkActive(signal);
	if (!response.ok) {
		await response.body?.cancel();
		throw new Error("App icon unavailable");
	}
	const type = response.headers
		.get("content-type")
		?.split(";")[0]
		.trim()
		.toLowerCase();
	if (!type?.startsWith("image/")) {
		await response.body?.cancel();
		throw new Error("App icon is not an image");
	}
	const declared = Number(response.headers.get("content-length") ?? 0);
	if (!Number.isFinite(declared) || declared > SOURCE_BYTES) {
		await response.body?.cancel();
		throw new Error("App icon too large");
	}
	const reader = response.body?.getReader();
	if (!reader) throw new Error("App icon has no body");
	const chunks: Uint8Array<ArrayBuffer>[] = [];
	let size = 0;
	const cancel = () => {
		void reader.cancel().catch(() => {});
	};
	signal.addEventListener("abort", cancel, { once: true });
	try {
		while (true) {
			checkActive(signal);
			const { done, value } = await reader.read();
			if (done) break;
			size += value.byteLength;
			if (size > SOURCE_BYTES) throw new Error("App icon too large");
			chunks.push(new Uint8Array(value));
		}
		checkActive(signal);
		return new Blob(chunks, { type });
	} finally {
		signal.removeEventListener("abort", cancel);
		await reader.cancel().catch(() => {});
		reader.releaseLock();
	}
}

export async function rasterizeNativeAppIcon(
	blob: Blob,
	signal: AbortSignal,
): Promise<string> {
	checkActive(signal);
	const url = URL.createObjectURL(blob);
	const image = new Image();
	try {
		await new Promise<void>((resolve, reject) => {
			const abort = () =>
				finish(new DOMException("Icon request cancelled", "AbortError"));
			const finish = (error?: Error) => {
				image.onload = null;
				image.onerror = null;
				signal.removeEventListener("abort", abort);
				error ? reject(error) : resolve();
			};
			image.onload = () => finish();
			image.onerror = () => finish(new Error("App icon cannot be decoded"));
			signal.addEventListener("abort", abort, { once: true });
			image.src = url;
		});
		checkActive(signal);
		if (
			!image.naturalWidth ||
			!image.naturalHeight ||
			image.naturalWidth > 4096 ||
			image.naturalHeight > 4096
		)
			throw new Error("App icon dimensions unsupported");
		const scale = Math.min(
			1,
			96 / Math.max(image.naturalWidth, image.naturalHeight),
		);
		const canvas = document.createElement("canvas");
		canvas.width = Math.max(1, Math.round(image.naturalWidth * scale));
		canvas.height = Math.max(1, Math.round(image.naturalHeight * scale));
		const context = canvas.getContext("2d");
		if (!context) throw new Error("App icon canvas unavailable");
		context.drawImage(image, 0, 0, canvas.width, canvas.height);
		const encoded = canvas.toDataURL("image/png").split(",")[1];
		if (!encoded || !isBoundedPNG(encoded))
			throw new Error("App icon too large");
		return encoded;
	} finally {
		image.src = "";
		URL.revokeObjectURL(url);
	}
}
