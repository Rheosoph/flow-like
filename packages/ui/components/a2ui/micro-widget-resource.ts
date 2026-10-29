import type {
	FlwEnvelope,
	ResourceReference,
	ResourceResultPayload,
} from "@flow-like/widget-sdk";
import type { IStorageState } from "../../state/backend-state/storage-state";

const MAX_RESOURCE_BYTES = 256 * 1024 * 1024;
const MAX_PENDING_REQUESTS = 128;
const MAX_REFERENCES = 20_000;
const MAX_QUERY_SCOPES = 32;

interface ResourceOptions {
	enabled(): boolean;
	desktop: boolean;
	result(payload: ResourceResultPayload, transfer?: Transferable[]): void;
	resolve(reference: ResourceReference): Promise<string | ArrayBuffer>;
	fetch?: typeof fetch;
	maxBytes?: number;
}

function record(value: unknown): value is Record<string, unknown> {
	return value !== null && typeof value === "object" && !Array.isArray(value);
}

function hasControls(value: string): boolean {
	return Array.from(value).some(
		(character) =>
			character.charCodeAt(0) < 32 || character.charCodeAt(0) === 127,
	);
}

function supportedUrl(value: unknown): value is string {
	if (typeof value !== "string" || value.length > 16_384) return false;
	try {
		const url = new URL(value);
		if (url.username || url.password || value.trim() !== value) return false;
		return url.protocol === "https:" && url.hostname !== "asset.localhost";
	} catch {
		return false;
	}
}

export function referenceKey(value: unknown): string | null {
	if (
		!record(value) ||
		typeof value.path !== "string" ||
		!value.path ||
		typeof value.store_ref !== "string" ||
		!value.store_ref ||
		value.path.length > 16_384 ||
		value.store_ref.length > 16_384 ||
		hasControls(value.path + value.store_ref) ||
		(typeof value.cache_store_ref === "string" &&
			(value.cache_store_ref.length > 16_384 ||
				hasControls(value.cache_store_ref)))
	)
		return null;
	return JSON.stringify([
		value.path,
		value.store_ref,
		typeof value.cache_store_ref === "string" ? value.cache_store_ref : null,
	]);
}

/** The reference is a lookup key. Its supplied URL is never trusted or fetched. */
export function resourceReferences(
	value: unknown,
): Map<string, ResourceReference> {
	const references = new Map<string, ResourceReference>();
	const seen = new WeakSet<object>();
	const pending: { value: unknown; depth: number }[] = [{ value, depth: 0 }];
	let visited = 0;
	while (pending.length && visited++ < 200_000) {
		const item = pending.pop();
		if (!item) break;
		if (!item.value || typeof item.value !== "object" || seen.has(item.value))
			continue;
		seen.add(item.value);
		if (record(item.value) && item.value.encoding === "reference") {
			const key = referenceKey(item.value.storage);
			if (key && record(item.value.storage)) {
				const reference = item.value.storage;
				references.set(key, {
					path: String(reference.path),
					store_ref: String(reference.store_ref),
					cache_store_ref:
						typeof reference.cache_store_ref === "string"
							? reference.cache_store_ref
							: null,
				});
				if (references.size >= MAX_REFERENCES) break;
			}
		}
		if (item.depth < 32)
			for (const child of Object.values(item.value)) {
				if (child && typeof child === "object")
					pending.push({ value: child, depth: item.depth + 1 });
			}
	}
	return references;
}

/** Read protected local bytes or obtain a fresh URL from the authorized storage API. */
export async function resolveAppResource(
	reference: ResourceReference,
	appId: string | undefined,
	storage: IStorageState,
): Promise<string | ArrayBuffer> {
	if (!appId || !/^[a-zA-Z0-9_-]+$/.test(appId))
		throw new Error("A stored resource needs a current app.");
	const path = reference.path;
	const parts = path.split("/");
	for (const part of parts) {
		let decoded: string;
		try {
			decoded = decodeURIComponent(part);
		} catch {
			throw new Error("Invalid stored resource path.");
		}
		if (
			!decoded ||
			decoded === "." ||
			decoded === ".." ||
			decoded.includes("/") ||
			decoded.includes("\\") ||
			hasControls(decoded)
		)
			throw new Error("Invalid stored resource path.");
	}
	let userScoped: boolean;
	if (
		parts[0] === "apps" &&
		parts[1] === appId &&
		parts[2] === "upload" &&
		parts.length > 3
	) {
		userScoped = false;
	} else if (
		parts[0] === "users" &&
		parts[2] === "apps" &&
		parts[3] === appId &&
		parts.length > 4
	) {
		userScoped = true;
	} else
		throw new Error(
			"This resource is outside the current app's upload storage.",
		);
	if (storage.readStorageItem) {
		const bytes = await storage.readStorageItem(appId, path, userScoped);
		if (bytes !== null) {
			if (!(bytes instanceof ArrayBuffer))
				throw new Error("The local storage reader returned invalid bytes.");
			return bytes;
		}
	}
	const items = userScoped
		? await storage.downloadStorageItemsUser(appId, [path])
		: await storage.downloadStorageItems(appId, [path]);
	const item = items.find((item) => item.prefix === path);
	if (!item?.url)
		throw new Error(item?.error || "The stored resource is unavailable.");
	if (!supportedUrl(item.url))
		throw new Error("Unsupported storage resource URL.");
	return item.url;
}

async function readResource(
	response: Response,
	limit: number,
): Promise<ArrayBuffer> {
	if (!response.ok)
		throw new Error(`Resource returned HTTP ${response.status}.`);
	const length = response.headers.get("content-length");
	if (length && Number(length) > limit) {
		await response.body?.cancel();
		throw new Error("This file exceeds the preview resource limit.");
	}
	if (!response.body) return new ArrayBuffer(0);
	const reader = response.body.getReader();
	const chunks: Uint8Array[] = [];
	let size = 0;
	try {
		while (true) {
			const chunk = await reader.read();
			if (chunk.done) break;
			size += chunk.value.byteLength;
			if (size > limit) {
				await reader.cancel();
				throw new Error("This file exceeds the preview resource limit.");
			}
			chunks.push(chunk.value);
		}
	} finally {
		reader.releaseLock();
	}
	const data = new Uint8Array(size);
	let offset = 0;
	for (const chunk of chunks) {
		data.set(chunk, offset);
		offset += chunk.byteLength;
	}
	return data.buffer;
}

/** The iframe can resolve host-provided files without receiving a network grant. */
export function createMicroWidgetResources(options: ResourceOptions) {
	let props = new Map<string, ResourceReference>();
	const queries = new Map<
		string,
		{ sequence: number; references: Map<string, ResourceReference> }
	>();
	const pendingQueries = new Map<
		string,
		{
			name: string;
			sequence: number;
			references: Map<string, ResourceReference>;
		}
	>();
	let querySequence = 0;
	const requests = new Map<
		string,
		{ key: string; controller: AbortController; generation: number }
	>();
	const queue: string[] = [];
	let active = 0;
	let generation = 0;
	let disposed = false;
	const fetchResource = options.fetch ?? globalThis.fetch;
	const allowed = (key: string) =>
		props.get(key) ??
		[...pendingQueries.values(), ...queries.values()]
			.map((scope) => scope.references.get(key))
			.find(Boolean);
	const failure = (requestId: string, error: string) =>
		options.result({ requestId, ok: false, error });

	const pump = () => {
		while (!disposed && active < 2 && queue.length) {
			const requestId = queue.shift();
			if (requestId === undefined) break;
			const request = requests.get(requestId);
			if (!request) continue;
			active++;
			void (async () => {
				try {
					if (!options.enabled() || !allowed(request.key))
						throw new Error("Resource access was revoked.");
					const reference = allowed(request.key);
					if (!reference) throw new Error("Resource access was revoked.");
					const resource = await options.resolve(reference);
					request.controller.signal.throwIfAborted();
					let data: ArrayBuffer;
					let rawMime = "";
					const limit = options.maxBytes ?? MAX_RESOURCE_BYTES;
					if (resource instanceof ArrayBuffer) {
						if (resource.byteLength > limit)
							throw new Error("This file exceeds the preview resource limit.");
						data = resource;
					} else {
						if (!supportedUrl(resource))
							throw new Error("Unsupported storage resource URL.");
						const response = await fetchResource(resource, {
							credentials: "omit",
							redirect: "error",
							mode: "cors",
							signal: request.controller.signal,
						});
						data = await readResource(response, limit);
						rawMime =
							response.headers.get("content-type")?.split(";")[0].trim() ?? "";
					}
					if (
						disposed ||
						request.generation !== generation ||
						request.controller.signal.aborted
					)
						return;
					if (!options.enabled() || !allowed(request.key))
						throw new Error("Resource access was revoked.");
					const mimeType = /^[a-z0-9!#$&^_.+-]+\/[a-z0-9!#$&^_.+-]+$/i.test(
						rawMime,
					)
						? rawMime
						: "application/octet-stream";
					options.result({ requestId, ok: true, data, mimeType }, [data]);
				} catch (error) {
					if (
						!disposed &&
						request.generation === generation &&
						requests.has(requestId)
					) {
						failure(
							requestId,
							error instanceof Error
								? error.message
								: "Resource could not be loaded.",
						);
					}
				} finally {
					if (requests.get(requestId) === request) requests.delete(requestId);
					active--;
					pump();
				}
			})();
		}
	};
	const revokeMissing = () => {
		for (const [requestId, request] of requests)
			if (!allowed(request.key)) {
				request.controller.abort();
				requests.delete(requestId);
				failure(requestId, "Resource access was revoked.");
			}
	};
	const reset = () => {
		generation++;
		for (const request of requests.values()) request.controller.abort();
		requests.clear();
		queue.length = 0;
		queries.clear();
		pendingQueries.clear();
		props.clear();
	};
	return {
		setProps(value: unknown) {
			props = resourceReferences(value);
			revokeMissing();
		},
		beginQuery(queryId: string, name: string, args: unknown) {
			pendingQueries.set(queryId, {
				name,
				sequence: ++querySequence,
				references: resourceReferences(args),
			});
		},
		completeQuery(queryId: string, accepted: boolean) {
			const pending = pendingQueries.get(queryId);
			pendingQueries.delete(queryId);
			if (
				pending &&
				accepted &&
				pending.sequence > (queries.get(pending.name)?.sequence ?? -1)
			) {
				queries.delete(pending.name);
				queries.set(pending.name, pending);
				while (queries.size > MAX_QUERY_SCOPES) {
					const oldest = queries.keys().next().value;
					if (oldest === undefined) break;
					queries.delete(oldest);
				}
			}
			revokeMissing();
		},
		handle(envelope: FlwEnvelope) {
			if (
				disposed ||
				(envelope.type !== "resource:request" &&
					envelope.type !== "resource:cancel")
			)
				return;
			const payload = envelope.payload as unknown;
			if (
				!record(payload) ||
				typeof payload.requestId !== "string" ||
				!payload.requestId ||
				payload.requestId.length > 128
			)
				return;
			const { requestId } = payload;
			if (envelope.type === "resource:cancel") {
				requests.get(requestId)?.controller.abort();
				requests.delete(requestId);
				return;
			}
			const key = referenceKey(payload.reference);
			if (!options.enabled() || !key || !allowed(key)) {
				failure(requestId, "This resource was not supplied by the host.");
				return;
			}
			if (requests.has(requestId) || requests.size >= MAX_PENDING_REQUESTS) {
				failure(requestId, "Too many pending resource requests.");
				return;
			}
			requests.set(requestId, {
				key,
				controller: new AbortController(),
				generation,
			});
			queue.push(requestId);
			pump();
		},
		reset,
		dispose() {
			disposed = true;
			reset();
		},
	};
}
