import { afterEach, beforeEach, describe, expect, it } from "bun:test";
import type { IChannelHandle } from "../../schema/channel";
import { replyToChannel, resetChannelClientCaches } from "../index";

type Listener = (event: unknown) => void;

class FakeWebSocket {
	static instances: FakeWebSocket[] = [];
	readyState = 0;
	sent: string[] = [];
	private listeners = new Map<string, Listener[]>();

	constructor(
		public url: string,
		public protocol: string,
	) {
		FakeWebSocket.instances.push(this);
	}

	addEventListener(type: string, listener: Listener) {
		const list = this.listeners.get(type) ?? [];
		list.push(listener);
		this.listeners.set(type, list);
	}

	removeEventListener() {}

	send(data: string) {
		if (this.readyState !== 1) throw new Error("socket not open");
		this.sent.push(data);
	}

	close() {
		this.readyState = 3;
		this.emit("close", { code: 1000, reason: "" });
	}

	open() {
		this.readyState = 1;
		this.emit("open", {});
	}

	receive(payload: unknown) {
		this.emit("message", { data: JSON.stringify(payload) });
	}

	emit(type: string, event: unknown) {
		for (const listener of this.listeners.get(type) ?? []) listener(event);
	}
}

const tick = () => new Promise((resolve) => setTimeout(resolve, 0));

function azureHandle(requestId = "req-1"): IChannelHandle {
	return {
		channel_id: "run-1",
		request_id: requestId,
		expires_at: 4_102_444_800,
		transport: {
			type: "azure_web_pubsub",
			url: "wss://hub.webpubsub.azure.com/client/hubs/runs?access_token=t",
			group: "run:run-1",
			expires_at: 4_102_444_800,
		},
	};
}

const originalWebSocket = globalThis.WebSocket;

beforeEach(() => {
	FakeWebSocket.instances = [];
	resetChannelClientCaches();
	globalThis.WebSocket = FakeWebSocket as unknown as typeof WebSocket;
});

afterEach(() => {
	resetChannelClientCaches();
	globalThis.WebSocket = originalWebSocket;
});

describe("azure_web_pubsub channel push", () => {
	it("sends to the group on open and resolves on the matching ack", async () => {
		const pending = replyToChannel(azureHandle(), { ok: true });
		const [socket] = FakeWebSocket.instances;
		expect(socket.protocol).toBe("json.webpubsub.azure.v1");
		expect(socket.url).toContain("access_token=t");

		socket.open();
		await tick();
		expect(socket.sent).toHaveLength(1);
		expect(JSON.parse(socket.sent[0])).toEqual({
			type: "sendToGroup",
			group: "run:run-1",
			ackId: 1,
			dataType: "json",
			data: {
				channel_id: "run-1",
				request_id: "req-1",
				kind: "reply",
				value: { ok: true },
			},
		});

		socket.receive({ type: "system", event: "connected" });
		socket.receive({ type: "ack", ackId: 1, success: true });
		await pending;
	});

	it("correlates acks by id and reuses one socket per channel", async () => {
		const first = replyToChannel(azureHandle("req-1"), 1);
		FakeWebSocket.instances[0].open();
		await tick();
		const second = replyToChannel(azureHandle("req-2"), 2);
		await tick();
		expect(FakeWebSocket.instances).toHaveLength(1);
		const [socket] = FakeWebSocket.instances;
		expect(socket.sent).toHaveLength(2);

		let firstSettled = false;
		void first.then(() => {
			firstSettled = true;
		});
		socket.receive({ type: "ack", ackId: 2, success: true });
		await second;
		await tick();
		expect(firstSettled).toBe(false);

		socket.receive({ type: "ack", ackId: 1, success: true });
		await first;
	});

	it("rejects on a failed ack with the error name", async () => {
		const pending = replyToChannel(azureHandle(), 1);
		const [socket] = FakeWebSocket.instances;
		socket.open();
		await tick();

		socket.receive({
			type: "ack",
			ackId: 1,
			success: false,
			error: { name: "Forbidden", message: "not in group" },
		});
		await expect(pending).rejects.toThrow(/Forbidden.*not in group/);
	});

	it("reconnects with the given handle after the socket closed", async () => {
		const first = replyToChannel(azureHandle(), 1);
		FakeWebSocket.instances[0].open();
		await tick();
		FakeWebSocket.instances[0].receive({
			type: "ack",
			ackId: 1,
			success: true,
		});
		await first;
		FakeWebSocket.instances[0].close();

		const second = replyToChannel(azureHandle("req-2"), 2);
		expect(FakeWebSocket.instances).toHaveLength(2);
		FakeWebSocket.instances[1].open();
		await tick();
		FakeWebSocket.instances[1].receive({
			type: "ack",
			ackId: 2,
			success: true,
		});
		await second;
	});

	it("fails pending sends when the socket closes", async () => {
		const pending = replyToChannel(azureHandle(), 1);
		const [socket] = FakeWebSocket.instances;
		socket.open();
		await tick();
		socket.close();
		await expect(pending).rejects.toThrow(/closed before the message/);
	});
});
