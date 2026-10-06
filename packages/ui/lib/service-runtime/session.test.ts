import { afterEach, describe, expect, spyOn, test } from "bun:test";
import { replyToChannel } from "../channel";
import type { DeviceServiceStream } from "../device-management/tunnel";
import type { IChannelHandle } from "../schema/channel";
import type { IIntercomEvent } from "../schema/events/intercom-event";
import { openRuntimeSession } from "./session";
import * as history from "./session-history";

const cleanup: Array<() => void> = [];
afterEach(() => {
	for (const close of cleanup.splice(0).reverse()) close();
});
const encoder = new TextEncoder();
const frame = (type: string, payload: unknown) =>
	`event: ${type}\ndata: ${JSON.stringify(payload)}\n\n`;
const event = {
	id: "chat",
	name: "Chat",
	event_type: "simple_chat",
	board_id: "board",
	node_id: "node",
	event_version: [1, 0, 0],
};
const inventory = { project_id: "project", events: [event] };
const assetPath = "/ui/assets/0123456789abcdef0123456789abcdef";
const runHandle: IChannelHandle = {
	channel_id: "run",
	request_id: "approval",
	expires_at: Date.now() / 1000 + 3600,
	transport: {
		type: "http",
		push_url: "/channels/incarnation/run",
		token: "r".repeat(43),
	},
};
type Request = { path: string; headers: Headers; body: string };
type Reply = {
	status?: number;
	type?: string;
	body?: string;
	keepOpen?: boolean;
};

function device(
	route: (request: Request, emit: (bytes: string) => void) => Reply,
) {
	const requests: Request[] = [];
	let resets = 0;
	const open = async () => {
		let input = "";
		let responded = false;
		let reset = false;
		const queue: Uint8Array[] = [];
		let waiting:
			| { resolve(value: Uint8Array): void; reject(error: Error): void }
			| undefined;
		const emit = (text: string) => {
			for (let start = 0; start < text.length; start += 4096) {
				const bytes = encoder.encode(text.slice(start, start + 4096));
				if (waiting) {
					waiting.resolve(bytes);
					waiting = undefined;
				} else queue.push(bytes);
			}
		};
		return {
			finished: false,
			async write(bytes: Uint8Array) {
				input += new TextDecoder().decode(bytes);
				const end = input.indexOf("\r\n\r\n");
				if (responded || end < 0) return;
				const lines = input.slice(0, end).split("\r\n");
				const headers = new Headers(
					lines.slice(1).map((line): [string, string] => {
						const split = line.indexOf(":");
						return [line.slice(0, split), line.slice(split + 1).trim()];
					}),
				);
				const body = input.slice(end + 4);
				if (
					encoder.encode(body).byteLength <
					Number(headers.get("content-length"))
				)
					return;
				responded = true;
				const request = { path: lines[0].split(" ")[1], headers, body };
				requests.push(request);
				const reply = route(request, emit);
				const status = reply.status ?? 200;
				emit(
					`HTTP/1.1 ${status} OK\r\nContent-Type: ${reply.type ?? "application/json"}\r\n${status === 204 || reply.keepOpen ? "" : `Content-Length: ${encoder.encode(reply.body ?? "").byteLength}\r\n`}\r\n`,
				);
				if (reply.body) emit(reply.body);
			},
			async read() {
				if (reset) throw new Error("reset");
				const next = queue.shift();
				if (next) return next;
				return new Promise<Uint8Array>((resolve, reject) => {
					waiting = { resolve, reject };
				});
			},
			reset() {
				if (reset) return;
				reset = true;
				resets++;
				waiting?.reject(new Error("reset"));
			},
		} as unknown as DeviceServiceStream;
	};
	return {
		open,
		requests,
		get resets() {
			return resets;
		},
	};
}

function isolateHistory() {
	const closed: string[] = [];
	const spy = spyOn(history, "startRuntimeHistory").mockImplementation(
		(appId) => async () => {
			closed.push(appId);
		},
	);
	cleanup.push(() => spy.mockRestore());
	return closed;
}

describe("deployed runtime session over HTTP tunnel", () => {
	test("loads inventory, streams a rich result and returns an interactive reply with isolated credentials", async () => {
		const retired = isolateHistory();
		let finishRun: ((bytes: string) => void) | undefined;
		const peer = device((request, emit) => {
			if (request.path === "/services")
				return { body: JSON.stringify(inventory) };
			if (request.path === "/chat/chat") {
				finishRun = emit;
				return {
					keepOpen: true,
					type: "text/event-stream",
					body:
						frame("run_initiated", { run_id: "run" }) +
						frame("chat_input", { channel: runHandle, image: assetPath }),
				};
			}
			if (request.path === assetPath || request.path.startsWith("/ui/assets?"))
				return { type: "image/png", body: "image-bytes" };
			if (request.path === "/channels/incarnation/run") {
				finishRun?.(frame("done", { completed: true }));
				return { status: 204 };
			}
			throw new Error(`Unexpected request ${request.path}`);
		});
		const session = await openRuntimeSession(
			{ open: peer.open },
			"s".repeat(43),
		);
		cleanup.push(session.close);
		expect(session.appId).not.toBe(inventory.project_id);
		let ready!: () => void;
		const presented = new Promise<void>((resolve) => {
			ready = resolve;
		});
		const events: IIntercomEvent[] = [];
		const execution = session.execution.executeEvent(
			session.appId,
			"chat",
			{ id: "", payload: { message: "hello" } },
			true,
			undefined,
			(incoming) => {
				events.push(...incoming);
				if (incoming.some((event) => event.event_type === "chat_input"))
					ready();
			},
		);
		await presented;
		const message = events.find((event) => event.event_type === "chat_input");
		if (!message) throw new Error("Missing interactive event");
		expect(message.payload.image.startsWith("blob:")).toBe(true);
		await replyToChannel(message.payload.channel, { approved: true });
		await execution;
		expect(events.at(-1)?.event_type).toBe("completed");
		const reply = peer.requests.find((request) =>
			request.path.startsWith("/channels/"),
		);
		expect(reply?.headers.get("authorization")).toBe(
			`Bearer ${"r".repeat(43)}`,
		);
		for (const request of peer.requests.filter(
			(request) => !request.path.startsWith("/channels/"),
		))
			expect(request.headers.get("authorization")).toBe(
				`Bearer ${"s".repeat(43)}`,
			);
		await session.backend.storageState.downloadStorageItems(session.appId, [
			"apps/project/upload/logo.png",
		]);
		expect(peer.requests.at(-1)?.path).toBe(
			"/ui/assets?store=upload&path=logo.png",
		);
		session.close();
		session.close();
		expect(retired).toEqual([session.appId]);
		await expect(replyToChannel(message.payload.channel, 2)).rejects.toThrow(
			/session has ended/,
		);
		await expect(fetch(message.payload.image)).rejects.toThrow();
	});

	test("closing a session cancels its active response and refuses later work", async () => {
		isolateHistory();
		const peer = device((request) =>
			request.path === "/services"
				? { body: JSON.stringify(inventory) }
				: { keepOpen: true, body: frame("run_initiated", { run_id: "run" }) },
		);
		const parent = new AbortController();
		const session = await openRuntimeSession(
			{ open: peer.open, signal: parent.signal },
			null,
		);
		cleanup.push(session.close);
		let started!: () => void;
		const ready = new Promise<void>((resolve) => {
			started = resolve;
		});
		const execution = session.execution.executeEvent(
			session.appId,
			"chat",
			{ id: "", payload: {} },
			true,
			started,
		);
		const failed = execution.catch((error) => error);
		await ready;
		parent.abort();
		expect(await failed).toMatchObject({ name: "AbortError" });
		expect(session.signal.aborted).toBe(true);
		expect(peer.resets).toBe(2);
		await expect(
			session.backend.eventState.getEvents(session.appId),
		).rejects.toMatchObject({ name: "AbortError" });
		expect(
			peer.requests.every((request) => !request.headers.has("authorization")),
		).toBe(true);
	});

	test("invalid service credentials are rejected before subscribing or opening a stream", async () => {
		const parent = new AbortController();
		const listen = spyOn(parent.signal, "addEventListener");
		cleanup.push(() => listen.mockRestore());
		const peer = device(() => {
			throw new Error("Unexpected request");
		});
		await expect(
			openRuntimeSession({ open: peer.open, signal: parent.signal }, "bad"),
		).rejects.toThrow(/token/);
		expect(listen).not.toHaveBeenCalled();
		expect(peer.requests).toHaveLength(0);
	});
});
