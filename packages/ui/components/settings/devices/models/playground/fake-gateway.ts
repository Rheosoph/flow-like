import type { DeviceServiceStream } from "../../../../../lib/device-management/tunnel";

/*
 * Tests only: a device's model gateway behind fake tunnel streams. Each
 * stream carries one HTTP exchange; the answer is close-delimited, and the
 * stream closes once it was read to its end or reset.
 */

export interface GatewayRequest {
	method: string;
	path: string;
	body: string;
}

export interface GatewayAnswer {
	status?: number;
	type?: string;
	/** The body in reads; a promise holds the reads after it back until it settles. */
	chunks: (string | Promise<void>)[];
}

const encoder = new TextEncoder();
const decoder = new TextDecoder();

/** The request once its head and its Content-Length body are all written. */
function requestOf(written: string) {
	const end = written.indexOf("\r\n\r\n");
	if (end < 0) return undefined;
	const [line = "", ...fields] = written.slice(0, end).split("\r\n");
	let length = 0;
	for (const field of fields) {
		const match = /^content-length:\s*(\d+)$/i.exec(field);
		if (match) length = Number(match[1]);
	}
	const body = written.slice(end + 4);
	if (body.length < length) return undefined;
	const [method = "", path = ""] = line.split(" ");
	const request: GatewayRequest = { method, path, body };
	return request;
}

const head = (answer: GatewayAnswer) =>
	`HTTP/1.1 ${answer.status ?? 200} OK\r\ncontent-type: ${answer.type ?? "application/json"}\r\n\r\n`;

/** `open` stands in for the live session's `openModelGateway`; `requests` lists what arrived. */
export function fakeGateway(
	answer: (request: GatewayRequest) => GatewayAnswer,
) {
	const requests: GatewayRequest[] = [];
	const open = async () => {
		let written = "";
		let queue: GatewayAnswer["chunks"] | undefined;
		let wake: (() => void) | undefined;
		let close!: () => void;
		const closed = new Promise<void>((resolve) => {
			close = resolve;
		});
		const stream = {
			finished: false,
			closed,
			async write(bytes: Uint8Array) {
				written += decoder.decode(bytes);
				const request = requestOf(written);
				if (!request || queue) return;
				requests.push(request);
				const reply = answer(request);
				queue = [head(reply), ...reply.chunks];
				wake?.();
			},
			async read() {
				while (!queue)
					await new Promise<void>((resolve) => {
						wake = resolve;
					});
				for (;;) {
					const next = queue.shift();
					if (next === undefined) {
						close();
						return null;
					}
					if (typeof next === "string") return encoder.encode(next);
					await next;
				}
			},
			reset() {
				close();
			},
		};
		return stream as unknown as DeviceServiceStream;
	};
	return { open, requests };
}

/** One server-sent event carrying `value` as JSON. */
export const sse = (value: unknown) => `data: ${JSON.stringify(value)}\n\n`;
