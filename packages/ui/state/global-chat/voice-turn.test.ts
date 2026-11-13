import { describe, expect, test } from "bun:test";
import type { IMessage } from "../../components/interfaces/chat-default/chat-db";
import {
	type VoiceTurnSender,
	registerVoiceTurnSender,
	runVoiceTurn,
	voiceTurnSender,
} from "./voice-turn";

const sender: VoiceTurnSender = async () => null;

describe("voice turn registry", () => {
	test("returns the registered sender and forgets it on unregister", () => {
		const unregister = registerVoiceTurnSender("bot_a", sender);
		expect(voiceTurnSender("bot_a")).toBe(sender);
		expect(voiceTurnSender("bot_b")).toBeNull();
		unregister();
		expect(voiceTurnSender("bot_a")).toBeNull();
	});

	test("a stale unregister does not remove a newer sender", () => {
		const first = registerVoiceTurnSender("bot_a", sender);
		const next: VoiceTurnSender = async () => null;
		const second = registerVoiceTurnSender("bot_a", next);
		first();
		expect(voiceTurnSender("bot_a")).toBe(next);
		second();
		expect(voiceTurnSender("bot_a")).toBeNull();
	});
});

describe("runVoiceTurn", () => {
	const request = {
		messageId: "m1",
		text: "hi",
		transcript: { call: "c", offset_ms: 0, duration_ms: 10, mentions: [] },
	};
	const message = (error?: { kind: string }) =>
		({ id: "r1", error }) as unknown as IMessage;

	test("not taken when the chat cannot take it; nothing is sent", async () => {
		let sent = false;
		const handle = await runVoiceTurn(
			request,
			() => false,
			async () => {
				sent = true;
				return undefined;
			},
		);
		expect(handle).toBeNull();
		expect(sent).toBe(false);
	});

	test("resolves with the run id as soon as it starts and maps the outcome", async () => {
		const finish = Promise.withResolvers<IMessage>();
		const handle = await runVoiceTurn(
			request,
			() => true,
			async (voice) => {
				await Promise.resolve();
				voice.onRun("r1");
				return finish.promise;
			},
		);
		expect(handle?.runId).toBe("r1");
		finish.resolve(message({ kind: "cancelled" }));
		expect(await handle?.done).toBe("cancelled");
	});

	test("an error message is failed, a clean one completed", async () => {
		const run = (result: IMessage) =>
			runVoiceTurn(
				request,
				() => true,
				async (voice) => {
					await Promise.resolve();
					voice.onRun("r1");
					return result;
				},
			);
		expect(await (await run(message({ kind: "provider" })))?.done).toBe(
			"failed",
		);
		expect(await (await run(message()))?.done).toBe("completed");
	});

	test("null when the pipeline ends before a run starts", async () => {
		expect(
			await runVoiceTurn(
				request,
				() => true,
				async () => undefined,
			),
		).toBeNull();
		expect(
			await runVoiceTurn(
				request,
				() => true,
				async () => {
					throw new Error("x");
				},
			),
		).toBeNull();
	});
});
