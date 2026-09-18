import { expect, test } from "bun:test";
import {
	createEnvelope,
	type MicrophoneResultPayload,
} from "@flow-like/widget-sdk";
import { createMicroWidgetMicrophone } from "./micro-widget-microphone";
const tick = () => new Promise((resolve) => setTimeout(resolve, 0));
test("undeclared microphone capability never asks the device", () => {
	let calls = 0;
	const results: MicrophoneResultPayload[] = [];
	const controller = createMicroWidgetMicrophone({
		enabled: () => false,
		result: (v) => results.push(v),
		recording: () => {},
		getUserMedia: async () => {
			calls++;
			throw Error();
		},
	});
	controller.handle(
		createEnvelope(
			"microphone:request",
			{ requestId: "a", maxDurationMs: 1000 },
			"n",
			"i",
		),
	);
	expect(calls).toBe(0);
	expect(results[0].ok).toBe(false);
	controller.dispose();
});
test("disposal while permission is pending stops the late stream", async () => {
	let resolve!: (stream: MediaStream) => void;
	let stopped = 0;
	let recorder = 0;
	const controller = createMicroWidgetMicrophone({
		enabled: () => true,
		result: () => {
			throw Error("must not publish");
		},
		recording: () => {},
		getUserMedia: () => new Promise((r) => (resolve = r)),
		createRecorder: () => {
			recorder++;
			throw Error();
		},
	});
	controller.handle(
		createEnvelope(
			"microphone:request",
			{ requestId: "a", maxDurationMs: 1000 },
			"n",
			"i",
		),
	);
	controller.dispose();
	resolve({
		getTracks: () => [{ stop: () => stopped++ }],
	} as unknown as MediaStream);
	await tick();
	expect(stopped).toBe(1);
	expect(recorder).toBe(0);
});
test("stop returns audio bytes and releases the microphone", async () => {
	let stopped = 0;
	const results: MicrophoneResultPayload[] = [];
	const states: boolean[] = [];
	const recorder = {
		state: "inactive",
		mimeType: "audio/webm",
		ondataavailable: null as any,
		onstop: null as any,
		start() {
			this.state = "recording";
		},
		stop() {
			this.state = "inactive";
			this.ondataavailable({ data: new Blob([new Uint8Array([1, 2])]) });
			this.onstop();
		},
	};
	const controller = createMicroWidgetMicrophone({
		enabled: () => true,
		result: (v) => results.push(v),
		recording: (v) => states.push(v),
		getUserMedia: async () =>
			({
				getTracks: () => [{ stop: () => stopped++ }],
			}) as unknown as MediaStream,
		createRecorder: () => recorder as unknown as MediaRecorder,
	});
	controller.handle(
		createEnvelope(
			"microphone:request",
			{ requestId: "a", maxDurationMs: 1000 },
			"n",
			"i",
		),
	);
	await tick();
	controller.stop();
	await tick();
	expect(results[0].ok).toBe(true);
	expect(results[0].mimeType).toBe("audio/webm");
	expect(new Uint8Array(results[0].bytes!)).toEqual(new Uint8Array([1, 2]));
	expect(stopped).toBe(1);
	expect(states).toEqual([true, false]);
	controller.dispose();
});

test("authorized capture pauses host audio before requesting microphone access", async () => {
	const order: string[] = [];
	const capture = createMicroWidgetMicrophone({
		enabled: () => true,
		beforeCapture: () => order.push("pause"),
		getUserMedia: async () => {
			order.push("microphone");
			throw Error("denied");
		},
		recording: () => {},
		result: () => {},
	});
	capture.handle(
		createEnvelope(
			"microphone:request",
			{ requestId: "one", maxDurationMs: 1000 },
			"n",
			"i",
		),
	);
	await tick();
	expect(order).toEqual(["pause", "microphone"]);
	capture.dispose();
	const denied = createMicroWidgetMicrophone({
		enabled: () => false,
		beforeCapture: () => order.push("unexpected"),
		recording: () => {},
		result: () => {},
	});
	denied.handle(
		createEnvelope(
			"microphone:request",
			{ requestId: "two", maxDurationMs: 1000 },
			"n",
			"i",
		),
	);
	expect(order).toEqual(["pause", "microphone"]);
	denied.dispose();
});


test("releasing before permission resolves rejects the capture and closes the late stream", async () => {
	let resolve!: (stream: MediaStream) => void;
	let stopped = 0;
	let recorders = 0;
	const results: MicrophoneResultPayload[] = [];
	const capture = createMicroWidgetMicrophone({
		enabled: () => true,
		result: (result) => results.push(result),
		recording: () => {},
		getUserMedia: () => new Promise((done) => { resolve = done; }),
		createRecorder: () => { recorders++; throw Error("must not start"); },
	});
	capture.handle(createEnvelope("microphone:request", { requestId: "hold", maxDurationMs: 30000 }, "nonce", "instance"));
	capture.handle(createEnvelope("microphone:stop", { requestId: "unrelated" }, "nonce", "instance"));
	expect(results).toHaveLength(0);
	capture.handle(createEnvelope("microphone:stop", { requestId: "hold" }, "nonce", "instance"));
	expect(results).toHaveLength(1);
	expect(results[0].ok).toBe(false);
	resolve({ getTracks: () => [{ stop: () => stopped++ }] } as unknown as MediaStream);
	await tick();
	expect(stopped).toBe(1);
	expect(recorders).toBe(0);
	expect(results).toHaveLength(1);
	capture.dispose();
});
