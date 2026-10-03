import type { IIntercomEvent } from "@flow-like/flow-like-ui/lib/schema/events/intercom-event";

const MAX_FRAME = 10 * 1024 * 1024;
const SERVICE_PATH =
	/^\/(services|pages\/[A-Za-z0-9_.-]{1,128}\/(bootstrap|invoke)|(chat|run)\/[A-Za-z0-9_.-]{1,128})$/;

const STATUS_ERRORS: Record<number, string> = {
	401: "The service access token was rejected. Unlock the service again.",
	413: "The request exceeds the service's size limit. Remove large attachments and try again.",
	429: "This service is busy. Try again when the current request finishes.",
};

const isText = (name: unknown): name is string => typeof name === "string";
const capName = (name: string) => name.slice(0, 64);

/** The names a run through the service page was refused for, from the device's 400 answer. */
async function refusedFields(response: Response) {
	try {
		const body = (await response.json()) as {
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
	if (known) return new Error(known);
	const refused =
		response.status === 400 ? await refusedFields(response) : null;
	if (refused)
		return new Error(
			refused.length
				? `The service refused these fields: ${refused.join(", ")}.`
				: "The service refused this form's input.",
		);
	return new Error(
		`The service could not complete this request (${response.status}).`,
	);
}

const dotSegment = (part: string) => part === "." || part === "..";

export function createServiceRequest(
	token: string,
	fetcher: typeof fetch = fetch,
) {
	if (!/^[\x21-\x7e]{32,4096}$/.test(token))
		throw new Error("Enter the service access token.");
	return async (path: string, init: RequestInit = {}) => {
		if (!SERVICE_PATH.test(path) || path.split("/").some(dotSegment))
			throw new Error("Unsupported service endpoint.");
		const headers = new Headers(init.headers);
		headers.set("Authorization", `Bearer ${token}`);
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

export async function consumeServiceStream(
	body: ReadableStream<Uint8Array>,
	onEvents?: (events: IIntercomEvent[]) => void,
	onRunId?: (id: string) => void,
) {
	const reader = body.getReader();
	const decoder = new TextDecoder();
	let buffer = "";
	let terminal = false;
	const consume = (frame: string) => {
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
		if (!eventType)
			throw new Error("The service returned an invalid stream event.");
		const payload = JSON.parse(data);
		if (eventType === "error")
			throw new Error("The workflow failed or exceeded its request deadline.");
		if (eventType === "done") {
			if (payload?.completed !== true)
				throw new Error("The workflow did not complete.");
			terminal = true;
		}
		if (eventType === "run_initiated" && typeof payload?.run_id === "string")
			onRunId?.(payload.run_id);
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
		while (!terminal) {
			const { value, done } = await reader.read();
			buffer += decoder.decode(value, { stream: !done });
			const frames = buffer.split(/\r?\n\r?\n/);
			buffer = frames.pop() ?? "";
			for (const frame of frames) {
				if (frame.length > MAX_FRAME)
					throw new Error("The service response exceeds its size limit.");
				consume(frame);
				if (terminal) break;
			}
			if (buffer.length > MAX_FRAME)
				throw new Error("The service response exceeds its size limit.");
			if (done) {
				if (buffer.trim()) consume(buffer);
				break;
			}
		}
		if (!terminal)
			throw new Error("The connection ended before the workflow completed.");
	} finally {
		await reader.cancel().catch(() => {});
		reader.releaseLock();
	}
}
