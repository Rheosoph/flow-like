import { expect, test } from "bun:test";
import type { Utterance } from "./call-listening";
import { createCallTranscription } from "./call-transcription";

function deferred<T>() {
	let resolve!: (value: T) => void;
	let reject!: (error: unknown) => void;
	const promise = new Promise<T>((yes, no) => {
		resolve = yes;
		reject = no;
	});
	return { promise, resolve, reject };
}

function utterance(offsetMs = 0, durationMs = 1000): Utterance {
	return { offsetMs, durationMs, samples: new Float32Array(durationMs * 16) };
}

async function settle() {
	for (let index = 0; index < 12; index++) await Promise.resolve();
}

function harness(
	overrides: Partial<
		Parameters<typeof createCallTranscription<Utterance>>[0]
	> = {},
) {
	const requests: {
		utterance: Utterance;
		result: ReturnType<typeof deferred<string>>;
	}[] = [];
	const partials: string[] = [];
	const finals: { text: string; utterance: Utterance }[] = [];
	const errors: unknown[] = [];
	const queue = createCallTranscription({
		transcribe: async (utterance: Utterance) => {
			const result = deferred<string>();
			requests.push({ utterance, result });
			return result.promise;
		},
		onPartial: (text) => partials.push(text),
		onFinal: async (text, utterance) => {
			finals.push({ text, utterance });
		},
		onError: (error) => errors.push(error),
		...overrides,
	});
	return { queue, requests, partials, finals, errors };
}

test("previews update one draft while only the final result is delivered", async () => {
	const h = harness();
	h.queue.partial(utterance());
	h.requests[0].result.resolve("  Hello  ");
	await settle();
	expect(h.partials).toEqual(["Hello"]);
	expect(h.finals).toEqual([]);
	h.queue.partial(utterance(0, 2000));
	h.requests[1].result.resolve("Hello there");
	await settle();
	expect(h.partials).toEqual(["Hello", "Hello there"]);
	const complete = utterance(0, 2200);
	h.queue.final(complete);
	h.requests[2].result.resolve("Hello there.");
	await settle();
	expect(h.finals).toEqual([{ text: "Hello there.", utterance: complete }]);
	expect(h.partials).toEqual(["Hello", "Hello there", ""]);
	h.queue.final(complete);
	h.queue.partial(complete);
	await settle();
	expect(h.requests).toHaveLength(3);
});

test("a slow model skips superseded previews and finals never wait for one", async () => {
	const h = harness();
	h.queue.partial(utterance(0, 1000));
	h.queue.partial(utterance(0, 2000));
	h.queue.partial(utterance(0, 3000));
	h.queue.final(utterance(0, 3500));
	h.queue.partial(utterance(5000, 1000));
	h.queue.partial(utterance(5000, 2000));
	expect(h.requests.map(({ utterance }) => utterance.durationMs)).toEqual([
		1000, 3500,
	]);
	h.requests[1].result.resolve("First utterance.");
	await settle();
	expect(h.finals.map(({ text }) => text)).toEqual(["First utterance."]);
	h.requests[0].result.resolve("old draft");
	await settle();
	expect(h.partials).toEqual([]);
	expect(h.requests.map(({ utterance }) => utterance.durationMs)).toEqual([
		1000, 3500, 2000,
	]);
	h.requests[2].result.resolve("Second utterance");
	await settle();
	expect(h.partials).toEqual(["Second utterance"]);
});

test("finals run one at a time, in order, beside an in-flight preview", async () => {
	const h = harness();
	h.queue.partial(utterance(0, 1000));
	h.queue.final(utterance(0, 1200));
	h.queue.final(utterance(5000, 1200));
	expect(h.requests.map(({ utterance }) => utterance.offsetMs)).toEqual([0, 0]);
	h.requests[1].result.resolve("one");
	await settle();
	expect(h.requests.map(({ utterance }) => utterance.offsetMs)).toEqual([
		0, 0, 5000,
	]);
	h.requests[2].result.resolve("two");
	await settle();
	expect(h.finals.map(({ text }) => text)).toEqual(["one", "two"]);
});

function settled(offsetMs: number, durationMs: number): Utterance {
	return { ...utterance(offsetMs, durationMs), settled: true };
}

test("a final reuses the settled preview of the same audio, in flight or done", async () => {
	for (const done of [false, true]) {
		const h = harness();
		h.queue.partial(settled(0, 2000));
		if (done) {
			h.requests[0].result.resolve("Hello there");
			await settle();
		}
		const complete = utterance(0, 2000);
		h.queue.final(complete);
		await settle();
		expect(h.requests).toHaveLength(1);
		if (!done) h.requests[0].result.resolve("Hello there");
		await settle();
		expect(h.finals).toEqual([{ text: "Hello there", utterance: complete }]);
	}
});

test("a settled preview is not reused once speech resumed, or after it failed", async () => {
	const resumed = harness();
	resumed.queue.partial(settled(0, 2000));
	resumed.requests[0].result.resolve("Hello");
	await settle();
	resumed.queue.partial(utterance(0, 3000));
	resumed.queue.final(utterance(0, 2500));
	await settle();
	expect(resumed.requests.map(({ utterance }) => utterance.durationMs)).toEqual(
		[2000, 3000, 2500],
	);

	const failed = harness();
	failed.queue.partial(settled(0, 2000));
	failed.queue.final(utterance(0, 2000));
	failed.requests[0].result.reject(new Error("model busy"));
	await settle();
	expect(failed.requests).toHaveLength(2);
	failed.requests[1].result.resolve("Recovered");
	await settle();
	expect(failed.finals.map(({ text }) => text)).toEqual(["Recovered"]);
	expect(failed.errors).toHaveLength(1);
});

test("a final whose settled preview has not started transcribes itself", async () => {
	const h = harness();
	h.queue.partial(utterance(0, 1000));
	h.queue.partial(settled(0, 2000));
	h.queue.final(utterance(0, 2000));
	expect(h.requests.map(({ utterance }) => utterance.durationMs)).toEqual([
		1000, 2000,
	]);
	h.requests[1].result.resolve("Done");
	await settle();
	expect(h.finals.map(({ text }) => text)).toEqual(["Done"]);
	h.requests[0].result.resolve("old");
	await settle();
	expect(h.requests).toHaveLength(2);
});

test("a probe transcribes outside the lanes and is empty after close", async () => {
	const h = harness();
	h.queue.partial(utterance(0, 1000));
	h.queue.final(utterance(0, 1200));
	const probing = h.queue.probe(utterance(9000, 700));
	expect(h.requests).toHaveLength(3);
	h.requests[2].result.resolve("  stop  ");
	expect(await probing).toBe("stop");
	h.queue.close();
	expect(await h.queue.probe(utterance(9000, 700))).toBe("");
});

test("an in-flight preview remains useful while a newer snapshot of the same utterance waits", async () => {
	const h = harness();
	h.queue.partial(utterance());
	h.queue.partial(utterance(0, 2000));
	h.queue.partial(utterance(0, 3000));
	h.requests[0].result.resolve("Hello");
	await settle();
	expect(h.partials).toEqual(["Hello"]);
	expect(h.requests[1].utterance.durationMs).toBe(3000);
	h.requests[1].result.resolve("   ");
	await settle();
	expect(h.partials).toEqual(["Hello"]);
});

test("final delivery does not clear the next utterance's draft", async () => {
	const delivery = deferred<void>();
	const h = harness({ onFinal: async () => delivery.promise });
	h.queue.partial(utterance());
	h.requests[0].result.resolve("First");
	await settle();
	h.queue.final(utterance(0, 1500));
	h.requests[1].result.resolve("First.");
	await settle();
	h.queue.partial(utterance(3000));
	expect(h.partials).toEqual(["First", ""]);
	delivery.resolve();
	await settle();
	h.requests[2].result.resolve("Second");
	await settle();
	expect(h.partials).toEqual(["First", "", "Second"]);
});

test("reset drops previews and their errors while retaining completed-utterance work", async () => {
	const h = harness();
	h.queue.partial(utterance());
	h.requests[0].result.resolve("Draft");
	await settle();
	h.queue.partial(utterance(0, 2000));
	h.queue.partial(utterance(0, 3000));
	h.queue.final(utterance(0, 3500));
	h.queue.resetPartial();
	expect(h.partials).toEqual(["Draft", ""]);
	h.requests[1].result.reject(new Error("stale preview failed"));
	await settle();
	expect(h.errors).toEqual([]);
	expect(h.requests[2].utterance.durationMs).toBe(3500);
	h.requests[2].result.resolve("Final.");
	await settle();
	expect(h.finals.map(({ text }) => text)).toEqual(["Final."]);
	expect(h.partials).toEqual(["Draft", ""]);
});

test("reset invalidates in-flight results even if the next draft has the same offset", async () => {
	const h = harness();
	h.queue.partial(utterance());
	h.queue.resetPartial();
	h.queue.partial(utterance(0, 2000));
	h.requests[0].result.resolve("stale");
	await settle();
	expect(h.partials).toEqual([]);
	h.requests[1].result.resolve("current");
	await settle();
	expect(h.partials).toEqual(["current"]);
});

test("closing clears a visible draft and discards every late result and queued item", async () => {
	for (const kind of ["partial", "final"] as const) {
		const h = harness();
		h.queue.partial(utterance());
		h.requests[0].result.resolve("Draft");
		await settle();
		h.queue[kind](utterance(0, 2000));
		h.queue.final(utterance(3000));
		h.queue.close();
		h.queue.close();
		h.queue.partial(utterance(5000));
		h.queue.final(utterance(6000));
		h.requests[1].result.resolve("late");
		await settle();
		expect(h.partials).toEqual(["Draft", ""]);
		expect(h.finals).toEqual([]);
		expect(h.errors).toEqual([]);
		expect(h.requests).toHaveLength(kind === "partial" ? 3 : 2);
	}
});

test("an error reports once and later final utterances continue", async () => {
	const h = harness();
	const error = new Error("speech model unavailable");
	h.queue.partial(utterance());
	h.queue.final(utterance(0, 2000));
	h.queue.final(utterance(3000));
	h.requests[0].result.reject(error);
	await settle();
	expect(h.errors).toEqual([error]);
	h.requests[1].result.resolve("Recovered.");
	await settle();
	h.requests[2].result.resolve("Next.");
	await settle();
	expect(h.finals.map(({ text }) => text)).toEqual(["Recovered.", "Next."]);
});

test("older final requests are bounded while the current transcription finishes", async () => {
	const h = harness();
	h.queue.final(utterance(0));
	for (const offset of [2000, 4000, 6000, 8000, 10000])
		h.queue.final(utterance(offset));
	for (let index = 0; index < 4; index++) {
		h.requests[index].result.resolve(`Final ${index}`);
		await settle();
	}
	expect(h.requests.map(({ utterance }) => utterance.offsetMs)).toEqual([
		0, 6000, 8000, 10000,
	]);
	expect(h.finals).toHaveLength(4);
});
