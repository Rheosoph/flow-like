import type { IIntercomEvent } from "../schema/events/intercom-event";

const MAX_FRAME = 10 * 1024 * 1024;
export const MAX_SERVICE_JSON_BYTES = 10 * 1024 * 1024;
const SERVICE_PATH =
	/^\/(services|pages\/[A-Za-z0-9_.-]{1,128}\/(bootstrap|invoke)|(chat|run)\/[A-Za-z0-9_.-]{1,128})$/;

const STATUS_ERRORS: Record<number, string> = {
	401: "The service access token was rejected. Unlock the service again.",
	413: "The request exceeds the service's size limit. Remove large attachments and try again.",
	429: "This service is busy. Try again when the current request finishes.",
};

export class ServiceRequestError extends Error {
	constructor(
		message: string,
		readonly status: number,
	) {
		super(message);
		this.name = "ServiceRequestError";
	}
}

const isText = (name: unknown): name is string => typeof name === "string";
const capName = (name: string) => name.slice(0, 64);

/** The names a run through the service page was refused for, from the device's 400 answer. */
async function refusedFields(response: Response) {
	try {
		const body = (await readServiceJson(response, undefined, 64 * 1024)) as {
			code?: unknown;
			fields?: unknown;
		};
		if (body.code !== "invalid_fields" || !Array.isArray(body.fields))
			return null;
		return body.fields.filter(isText).map(capName);
	} catch {
		return null;
	}
}

/** What a refused request says, without the service's own text. */
async function requestError(response: Response) {
	const known = STATUS_ERRORS[response.status];
	if (known) {
		await response.body?.cancel().catch(() => {});
		return new ServiceRequestError(known, response.status);
	}
	const refused =
		response.status === 400 ? await refusedFields(response) : null;
	if (refused)
		return new Error(
			refused.length
				? `The service refused these fields: ${refused.join(", ")}.`
				: "The service refused this form's input.",
		);
	await response.body?.cancel().catch(() => {});
	return new Error(
		`The service could not complete this request (${response.status}).`,
	);
}

const dotSegment = (part: string) => part === "." || part === "..";

function isAssetPath(path: string): boolean {
	if (/^\/ui\/assets\/(?:[0-9a-f]{32}\/)?[0-9a-f]{32}$/.test(path)) return true;
	if (!path.startsWith("/ui/assets?")) return false;
	const query = new URLSearchParams(path.slice("/ui/assets?".length));
	const keys = [...query.keys()];
	return (
		keys.length === 2 &&
		new Set(keys).size === 2 &&
		keys.includes("path") &&
		keys.includes("store") &&
		query.get("store") === "upload" &&
		Boolean(query.get("path"))
	);
}

export type BrowserFetch = (
	input: RequestInfo | URL,
	init?: RequestInit,
) => Promise<Response>;

export function createServiceRequest(
	token: string | null,
	fetcher: BrowserFetch = fetch,
) {
	if (token !== null && !/^[\x21-\x7e]{32,4096}$/.test(token))
		throw new Error("Enter the service access token.");
	return async (path: string, init: RequestInit = {}) => {
		if (
			(!SERVICE_PATH.test(path) || path.split("/").some(dotSegment)) &&
			!isAssetPath(path)
		)
			throw new Error("Unsupported service endpoint.");
		const headers = new Headers(init.headers);
		if (token === null) headers.delete("Authorization");
		else headers.set("Authorization", `Bearer ${token}`);
		if (init.body) headers.set("Content-Type", "application/json");
		const response = await fetcher(path, {
			...init,
			headers,
			cache: "no-store",
			credentials: "omit",
			redirect: "error",
		});
		if (!response.ok) throw await requestError(response);
		return response;
	};
}
export type ServiceRequest = ReturnType<typeof createServiceRequest>;

/** Read metadata with the same bounded response limit as the hosted service. */
export async function readServiceJson(
	response: Response,
	signal?: AbortSignal,
	limit = MAX_SERVICE_JSON_BYTES,
): Promise<unknown> {
	if (!response.body) throw new Error("The service returned no response body.");
	const reader = response.body.getReader();
	const chunks: Uint8Array[] = [];
	let bytes = 0;
	const abort = () => {
		void reader.cancel(signal?.reason).catch(() => {});
	};
	signal?.addEventListener("abort", abort, { once: true });
	try {
		signal?.throwIfAborted();
		for (;;) {
			const result = await reader.read();
			signal?.throwIfAborted();
			if (result.done) break;
			bytes += result.value.byteLength;
			if (bytes > limit)
				throw new Error("The service response exceeds its size limit.");
			chunks.push(result.value);
		}
		const buffer = new Uint8Array(bytes);
		let offset = 0;
		for (const chunk of chunks) {
			buffer.set(chunk, offset);
			offset += chunk.byteLength;
		}
		return JSON.parse(new TextDecoder("utf-8", { fatal: true }).decode(buffer));
	} finally {
		signal?.removeEventListener("abort", abort);
		await reader.cancel().catch(() => {});
		reader.releaseLock();
	}
}

export type ServiceValueMapper = (
	value: unknown,
	signal: AbortSignal,
) => Promise<unknown>;

export interface ServiceStreamOptions {
	signal?: AbortSignal;
	mapValue?: ServiceValueMapper;
}

export async function consumeServiceStream(
	body: ReadableStream<Uint8Array>,
	onEvents?: (events: IIntercomEvent[]) => void,
	onRunId?: (id: string) => void,
	options: ServiceStreamOptions = {},
) {
	const signal = options.signal ?? new AbortController().signal;
	const reader = body.getReader();
	const decoder = new TextDecoder("utf-8", { fatal: true });
	const encoder = new TextEncoder();
	let buffer = "";
	let bufferBytes = 0;
	let scanFrom = 0;
	let terminal = false;
	let runId: string | undefined;
	const abort = () => {
		void reader.cancel(signal.reason).catch(() => {});
	};
	signal.addEventListener("abort", abort, { once: true });
	const consume = async (
		frame: string,
		frameBytes = encoder.encode(frame).byteLength,
	) => {
		signal.throwIfAborted();
		if (frameBytes > MAX_FRAME)
			throw new Error("The service response exceeds its size limit.");
		const lines = frame.split(/\r?\n/);
		const eventType = lines
			.find((line) => line.startsWith("event:"))
			?.slice(6)
			.trim();
		const data = lines
			.filter((line) => line.startsWith("data:"))
			.map((line) => line.slice(5).replace(/^ /, ""))
			.join("\n");
		if (!data) return;
		if (!eventType || !/^[A-Za-z0-9_]{1,128}$/.test(eventType))
			throw new Error("The service returned an invalid stream event.");
		let payload = JSON.parse(data);
		if (eventType === "error")
			throw new Error("The workflow failed or exceeded its request deadline.");
		if (eventType === "done") {
			if (payload?.completed !== true)
				throw new Error("The workflow did not complete.");
			terminal = true;
		}
		if (eventType === "run_initiated") {
			if (
				typeof payload?.run_id !== "string" ||
				!/^[A-Za-z0-9_.-]{1,128}$/.test(payload.run_id) ||
				(runId && runId !== payload.run_id)
			)
				throw new Error("The service returned an invalid run identifier.");
			runId = payload.run_id;
		}
		if (options.mapValue) payload = await options.mapValue(payload, signal);
		signal.throwIfAborted();
		if (eventType === "run_initiated" && runId) onRunId?.(runId);
		const now = Date.now();
		onEvents?.([
			{
				event_id: crypto.randomUUID(),
				event_type: eventType === "done" ? "completed" : eventType,
				payload: eventType === "done" ? { status: "completed" } : payload,
				timestamp: {
					secs_since_epoch: Math.floor(now / 1000),
					nanos_since_epoch: (now % 1000) * 1_000_000,
				},
			},
		]);
	};
	try {
		signal.throwIfAborted();
		while (!terminal) {
			const { value, done } = await reader.read();
			signal.throwIfAborted();
			const text = decoder.decode(value, { stream: !done });
			buffer += text;
			bufferBytes += encoder.encode(text).byteLength;
			const separator = /\r?\n\r?\n/g;
			separator.lastIndex = scanFrom;
			for (;;) {
				const boundary = separator.exec(buffer);
				if (!boundary) break;
				const frame = buffer.slice(0, boundary.index);
				const frameBytes = encoder.encode(frame).byteLength;
				buffer = buffer.slice(boundary.index + boundary[0].length);
				bufferBytes -= frameBytes + boundary[0].length;
				await consume(frame, frameBytes);
				if (terminal) break;
				separator.lastIndex = 0;
			}
			if (terminal) break;
			// A delimiter can begin in the last three characters of the previous chunk.
			scanFrom = Math.max(0, buffer.length - 3);
			if (bufferBytes > MAX_FRAME)
				throw new Error("The service response exceeds its size limit.");
			if (done) {
				if (buffer.trim()) await consume(buffer);
				break;
			}
		}
		if (!terminal)
			throw new Error("The connection ended before the workflow completed.");
	} finally {
		signal.removeEventListener("abort", abort);
		await reader.cancel().catch(() => {});
		reader.releaseLock();
	}
}
