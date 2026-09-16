import { describe, expect, test } from "bun:test";
import { createWidgetMicrophoneClient } from "../src/microphone";
import {
	createEnvelope,
	type FlwEnvelope,
	type InitCapabilities,
} from "../src/protocol";

function setup(capabilities: InitCapabilities = { microphone: true }) {
	const messages: FlwEnvelope[] = [];
	const client = createWidgetMicrophoneClient(
		(type, payload) => {
			messages.push(createEnvelope(type, payload, "nonce", "instance"));
		},
		() => capabilities,
	);
	return { client, messages };
}
describe("widget microphone client", () => {
	test("capture is bounded and stops through the host", async () => {
		const { client, messages } = setup();
		const result = client.captureAudio({ maxDurationMs: 90000 });
		expect(messages[0]!.type).toBe("microphone:request");
		expect(
			(messages[0]!.payload as { maxDurationMs: number }).maxDurationMs,
		).toBe(60000);
		client.stopAudioCapture();
		expect(messages[1]!.type).toBe("microphone:stop");
		client.dispose();
		await expect(result).rejects.toThrow(/disposed/);
	});
	test("rejects without the microphone capability and posts nothing", async () => {
		const { client, messages } = setup({});
		await expect(client.captureAudio()).rejects.toThrow(/not granted/);
		expect(messages.length).toBe(0);
		client.dispose();
	});
	test("resolves results and never copies error diagnostics", async () => {
		const { client, messages } = setup();
		const first = client.captureAudio();
		const a = messages[0]!.payload as { requestId: string };
		client.handle(
			createEnvelope(
				"microphone:result",
				{
					requestId: a.requestId,
					ok: true,
					bytes: new Uint8Array([7]).buffer,
					mimeType: "audio/webm",
					status: 200,
				},
				"nonce",
				"instance",
			),
		);
		const recorded = await first;
		expect(new Uint8Array(recorded.bytes)).toEqual(new Uint8Array([7]));
		expect(recorded.mimeType).toBe("audio/webm");
		expect(recorded.status).toBe(200);
		const second = client.captureAudio();
		const b = messages[1]!.payload as { requestId: string };
		client.handle(
			createEnvelope(
				"microphone:result",
				{ requestId: b.requestId, ok: false, error: "secret" },
				"nonce",
				"instance",
			),
		);
		const failure = await second.catch((error: Error) => error);
		expect(failure).toBeInstanceOf(Error);
		expect((failure as Error).message).toBe("Microphone capture failed");
		client.dispose();
	});
});
