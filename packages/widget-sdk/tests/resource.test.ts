import { describe, expect, test, vi } from "bun:test";
import {
	type FlwEnvelope,
	type InitCapabilities,
	createEnvelope,
} from "../src/protocol";
import {
	MAX_WIDGET_RESOURCES_IN_FLIGHT,
	MAX_WIDGET_RESOURCE_BYTES,
	type ResourceReference,
	type ResourceRequestPayload,
	WIDGET_RESOURCE_TIMEOUT_MS,
	createWidgetResourceClient,
} from "../src/resource";

const reference = (path = "asset.bin"): ResourceReference => ({
	path,
	store_ref: "project-store",
	cache_store_ref: null,
});

function setup(capabilities: InitCapabilities = { resources: true }) {
	const messages: FlwEnvelope[] = [];
	const client = createWidgetResourceClient(
		(type, payload) => {
			messages.push(createEnvelope(type, payload, "nonce", "instance"));
		},
		() => capabilities,
	);
	const reply = (requestId: string, result: Record<string, unknown>) =>
		client.handle(
			createEnvelope(
				"resource:result",
				{ requestId, ok: false, ...result },
				"nonce",
				"instance",
			),
		);
	const request = () => messages.at(-1)?.payload as ResourceRequestPayload;
	return { client, messages, reply, request };
}

describe("widget resource client", () => {
	test("receives an ArrayBuffer as a Blob with a normalized MIME type", async () => {
		const { client, messages, reply, request } = setup();
		const source = {
			...reference("public/photo.png"),
			url: "https://untrusted.example/ignored",
		};
		const result = client.resolveResource(source);
		expect(messages[0]?.type).toBe("resource:request");
		expect(request().reference).toEqual(reference("public/photo.png"));
		reply(request().requestId, {
			ok: true,
			data: new Uint8Array([0, 255, 128]).buffer,
			mimeType: "IMAGE/PNG; charset=utf-8",
		});
		const blob = await result;
		expect(blob.type).toBe("image/png");
		expect([...new Uint8Array(await blob.arrayBuffer())]).toEqual([
			0, 255, 128,
		]);
		client.dispose();
	});

	test("requires permission and rejects invalid references before posting", async () => {
		const denied = setup({});
		await expect(
			denied.client.resolveResource(reference("a")),
		).rejects.toMatchObject({ code: "not-granted" });
		expect(denied.messages).toHaveLength(0);
		const { client, messages } = setup();
		for (const source of [
			null,
			{},
			reference(""),
			reference("../secret"),
			reference("a\nb"),
			reference("x".repeat(4097)),
			{ ...reference(), store_ref: "" },
			{ ...reference(), cache_store_ref: 42 },
		])
			await expect(
				client.resolveResource(source as ResourceReference),
			).rejects.toBeInstanceOf(TypeError);
		expect(messages).toHaveLength(0);
		client.dispose();
	});

	test("rejects malformed and oversized binary replies and accepts empty files", async () => {
		const { client, reply, request } = setup();
		const malformed = client.resolveResource(reference("a"));
		reply(request().requestId, { ok: true, data: new Uint8Array([1]) });
		await expect(malformed).rejects.toMatchObject({ code: "failed" });
		const oversized = client.resolveResource(reference("b"));
		reply(request().requestId, {
			ok: true,
			data: new ArrayBuffer(MAX_WIDGET_RESOURCE_BYTES + 1),
		});
		await expect(oversized).rejects.toMatchObject({ code: "too-large" });
		const empty = client.resolveResource(reference("c"));
		reply(request().requestId, {
			ok: true,
			data: new ArrayBuffer(0),
			mimeType: "text/plain\r\nX-Injected: yes",
		});
		expect((await empty).size).toBe(0);
		expect((await empty).type).toBe("application/octet-stream");
		client.dispose();
	});

	test("bounds pending requests while allowing the host to queue work", async () => {
		const { client, messages } = setup();
		const requests = Array.from(
			{ length: MAX_WIDGET_RESOURCES_IN_FLIGHT },
			(_, i) =>
				client.resolveResource(reference(String(i))).catch(() => undefined),
		);
		await expect(
			client.resolveResource(reference("overflow")),
		).rejects.toMatchObject({ code: "busy" });
		expect(messages).toHaveLength(MAX_WIDGET_RESOURCES_IN_FLIGHT);
		client.dispose();
		await Promise.all(requests);
		expect(
			messages.filter((message) => message.type === "resource:cancel"),
		).toHaveLength(MAX_WIDGET_RESOURCES_IN_FLIGHT);
	});

	test("preserves known error codes and bounded host explanations", async () => {
		const { client, reply, request } = setup();
		const missing = client.resolveResource(reference("missing"));
		reply(request().requestId, { error: "not-found" });
		await expect(missing).rejects.toMatchObject({ code: "not-found" });
		const failed = client.resolveResource(reference("failed"));
		const explanation =
			"This file is outside the current app's upload storage.";
		reply(request().requestId, { error: explanation });
		await expect(failed).rejects.toMatchObject({
			code: "failed",
			message: explanation,
		});
		const long = client.resolveResource(reference("long"));
		reply(request().requestId, { error: `  ${"x".repeat(1024)}  ` });
		await expect(long).rejects.toMatchObject({
			code: "failed",
			message: "x".repeat(512),
		});
		const empty = client.resolveResource(reference("empty"));
		reply(request().requestId, { error: "  " });
		await expect(empty).rejects.toMatchObject({
			code: "failed",
			message: "Resource could not be loaded",
		});
		client.dispose();
	});

	test("abort cancels once and ignores late replies", async () => {
		const { client, messages, reply, request } = setup();
		const controller = new AbortController();
		const result = client.resolveResource(reference("a"), {
			signal: controller.signal,
		});
		const id = request().requestId;
		controller.abort();
		await expect(result).rejects.toMatchObject({ name: "AbortError" });
		reply(id, { ok: true, data: new ArrayBuffer(0) });
		expect(
			messages.filter((message) => message.type === "resource:cancel"),
		).toHaveLength(1);
		await expect(
			client.resolveResource(reference("b"), {
				signal: controller.signal,
			}),
		).rejects.toMatchObject({ name: "AbortError" });
		client.dispose();
	});

	test("reset cancels requests, keeps IDs unique, and permits the next connection", async () => {
		const { client, messages, reply, request } = setup();
		const first = client.resolveResource(reference("a"));
		const firstId = request().requestId;
		client.reset();
		await expect(first).rejects.toMatchObject({ code: "connection-replaced" });
		expect(messages.at(-1)?.type).toBe("resource:cancel");
		const second = client.resolveResource(reference("b"));
		const secondId = request().requestId;
		expect(secondId).not.toBe(firstId);
		reply(firstId, { ok: true, data: new ArrayBuffer(1) });
		reply(secondId, { ok: true, data: new ArrayBuffer(2) });
		expect((await second).size).toBe(2);
		client.dispose();
		await expect(client.resolveResource(reference("c"))).rejects.toMatchObject({
			code: "disposed",
		});
	});

	test("capability revocation cancels pending work", async () => {
		const { client, messages } = setup();
		const result = client.resolveResource(reference("a"));
		client.handle(
			createEnvelope("capabilities:update", {}, "nonce", "instance"),
		);
		await expect(result).rejects.toMatchObject({ code: "not-granted" });
		expect(messages.at(-1)?.type).toBe("resource:cancel");
		client.dispose();
	});

	test("times out and cancels without leaving a pending timer", async () => {
		vi.useFakeTimers();
		const { client, messages } = setup();
		try {
			const result = client.resolveResource(reference("a"));
			vi.advanceTimersByTime(WIDGET_RESOURCE_TIMEOUT_MS);
			await expect(result).rejects.toMatchObject({ code: "timeout" });
			expect(messages.at(-1)?.type).toBe("resource:cancel");
			client.dispose();
			vi.advanceTimersByTime(WIDGET_RESOURCE_TIMEOUT_MS);
			expect(
				messages.filter((message) => message.type === "resource:cancel"),
			).toHaveLength(1);
		} finally {
			client.dispose();
			vi.useRealTimers();
		}
	});

	test("cleans up when posting or cancelling throws", async () => {
		const client = createWidgetResourceClient(
			() => {
				throw new Error("disconnected");
			},
			() => ({ resources: true }),
		);
		await expect(client.resolveResource(reference("a"))).rejects.toMatchObject({
			code: "failed",
		});
		expect(() => client.dispose()).not.toThrow();
	});
});
