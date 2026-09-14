import { afterEach, beforeEach, describe, expect, mock, test } from "bun:test";
import {
	getFrontendAudioEpoch,
	playFrontendAudio,
	stopAllFrontendAudio,
	validateFrontendAudio,
} from "./frontend-audio";

class FakeAudio extends EventTarget {
	src = "";
	volume = 1;
	muted = false;
	loop = false;
	preload = "";
	duration = 1.25;
	currentTime = 1.25;
	error: { code: number } | null = null;
	play = mock(async () => {});
	pause = mock(() => {});
	load = mock(() => {});
	removeAttribute(name: string) {
		if (name === "src") this.src = "";
	}
}

const origin = "https://flow-like.test";
const context = { appId: "app", executionTarget: "remote" as const };
let doc: EventTarget & { visibilityState: string };
let win: EventTarget;
let originals: (readonly [string, PropertyDescriptor | undefined])[];
const flush = () => new Promise<void>((resolve) => setTimeout(resolve, 0));

beforeEach(() => {
	originals = ["document", "window", "location"].map(
		(key) => [key, Object.getOwnPropertyDescriptor(globalThis, key)] as const,
	);
	doc = Object.assign(new EventTarget(), { visibilityState: "visible" });
	win = new EventTarget();
	Object.defineProperties(globalThis, {
		document: { configurable: true, value: doc },
		window: { configurable: true, value: win },
		location: { configurable: true, value: { origin } },
	});
});
afterEach(() => {
	stopAllFrontendAudio();
	for (const [key, descriptor] of originals)
		descriptor
			? Object.defineProperty(globalThis, key, descriptor)
			: Reflect.deleteProperty(globalThis, key);
});

const dependencies = (audio: FakeAudio) => ({
	createAudio: () => audio as unknown as HTMLAudioElement,
});

describe("foreground audio URL contract", () => {
	test("accepts network audio and restricts local resources to local execution", () => {
		expect(
			validateFrontendAudio(
				{ url: "https://storage.example/clip.wav?signature=token" },
				context,
			),
		).toMatchObject({ volume: 1 });
		for (const url of [
			"asset://localhost/audio.wav",
			"http://asset.localhost/audio.wav",
			"https://tauri.localhost/audio.wav",
			`blob:${origin}/audio-id`,
		]) {
			expect(
				validateFrontendAudio({ url }, { executionTarget: "local" }).url,
			).toBe(url);
			expect(() => validateFrontendAudio({ url }, context)).toThrow();
		}
	});
	test("rejects foreign blobs, executable URLs, URL credentials and invalid options", () => {
		for (const url of [
			"blob:https://other.example/id",
			"blob:null/id",
			"data:audio/wav;base64,AA==",
			"file:///tmp/audio.wav",
			"javascript:alert(1)",
			"tauri://localhost/audio.wav",
			"asset://other/audio.wav",
			"asset://localhost./audio.wav",
			"https://user:password@example.com/audio.wav",
			"https://example.com/audio\n.wav",
			"/relative.wav",
			"",
		]) {
			expect(() =>
				validateFrontendAudio({ url }, { executionTarget: "local" }),
			).toThrow();
		}
		expect(() =>
			validateFrontendAudio(
				{ url: "http://asset.localhost./audio.wav" },
				context,
			),
		).toThrow();
		for (const volume of [
			-1,
			1.01,
			"0.5",
			Number.NaN,
			Number.POSITIVE_INFINITY,
		])
			expect(() =>
				validateFrontendAudio(
					{ url: "https://example.com/a.wav", volume },
					context,
				),
			).toThrow();
		expect(() =>
			validateFrontendAudio(
				{ url: `https://example.com/${"é".repeat(4096)}` },
				context,
			),
		).toThrow();
	});
});

test("playback resolves only on ended and releases the media resource", async () => {
	const audio = new FakeAudio();
	let completed = false;
	const playing = playFrontendAudio(
		{ url: "https://example.com/clip.wav", volume: 0.5 },
		context,
		dependencies(audio),
	);
	void playing.then(() => {
		completed = true;
	});
	await flush();
	expect(completed).toBe(false);
	expect(audio.volume).toBe(0.5);
	expect(audio.play).toHaveBeenCalledTimes(1);
	audio.dispatchEvent(new Event("ended"));
	expect(await playing).toEqual({ status: "finished", durationSeconds: 1.25 });
	expect(audio.src).toBe("");
	expect(audio.pause).toHaveBeenCalledTimes(1);
	expect(audio.load).toHaveBeenCalledTimes(1);
	audio.dispatchEvent(new Event("error"));
	win.dispatchEvent(new Event("pagehide"));
	expect(audio.pause).toHaveBeenCalledTimes(1);
});

test("volume zero mutes and refused attenuation fails before playing", async () => {
	const muted = new FakeAudio();
	const playing = playFrontendAudio(
		{ url: "https://example.com/clip.wav", volume: 0 },
		context,
		dependencies(muted),
	);
	expect(muted.muted).toBe(true);
	muted.dispatchEvent(new Event("ended"));
	await playing;
	const fixedVolume = new FakeAudio();
	Object.defineProperty(fixedVolume, "volume", { get: () => 1, set: () => {} });
	await expect(
		playFrontendAudio(
			{ url: "https://example.com/clip.wav", volume: 0.5 },
			context,
			dependencies(fixedVolume),
		),
	).rejects.toMatchObject({ code: "unsupported_volume" });
	expect(fixedVolume.play).not.toHaveBeenCalled();
});

test("autoplay denial waits for an explicit synchronous Play sound action", async () => {
	const audio = new FakeAudio();
	audio.play.mockImplementationOnce(async () => {
		throw new DOMException("Activation needed", "NotAllowedError");
	});
	let play!: () => void;
	const dismiss = mock(() => {});
	const prompt = mock(async (action: () => void) => {
		play = action;
		return dismiss;
	});
	const playing = playFrontendAudio(
		{ url: "https://example.com/clip.wav" },
		context,
		{ ...dependencies(audio), prompt },
	);
	await flush();
	expect(prompt).toHaveBeenCalledTimes(1);
	expect(audio.play).toHaveBeenCalledTimes(1);
	play();
	expect(audio.play).toHaveBeenCalledTimes(2);
	await flush();
	audio.dispatchEvent(new Event("ended"));
	await playing;
	expect(dismiss).toHaveBeenCalledTimes(1);
});

test("gesture denial is not retried and generic decode failures never show a prompt", async () => {
	const audio = new FakeAudio();
	audio.play.mockImplementation(async () => {
		throw new DOMException("Denied", "NotAllowedError");
	});
	let play!: () => void;
	const prompt = mock(async (action: () => void) => {
		play = action;
		return () => {};
	});
	const playing = playFrontendAudio(
		{ url: "https://example.com/clip.wav" },
		context,
		{ ...dependencies(audio), prompt },
	);
	await flush();
	play();
	await expect(playing).rejects.toMatchObject({ code: "permission_required" });
	expect(prompt).toHaveBeenCalledTimes(1);
	const broken = new FakeAudio();
	broken.play.mockImplementation(async () => {
		throw new DOMException("Format", "NotSupportedError");
	});
	await expect(
		playFrontendAudio({ url: "https://example.com/broken.wav" }, context, {
			...dependencies(broken),
			prompt,
		}),
	).rejects.toMatchObject({ code: "unsupported_format" });
	expect(prompt).toHaveBeenCalledTimes(1);
});

test("network errors stop playback without an autoplay prompt", async () => {
	const audio = new FakeAudio();
	const prompt = mock(async () => () => {});
	const playing = playFrontendAudio(
		{ url: "https://example.com/missing.wav" },
		context,
		{ ...dependencies(audio), prompt },
	);
	audio.error = { code: 2 };
	audio.dispatchEvent(new Event("error"));
	await expect(playing).rejects.toMatchObject({ code: "playback_failed" });
	expect(prompt).not.toHaveBeenCalled();
});

test("new audio interrupts only the preceding playback for the same app", async () => {
	const oldAudio = new FakeAudio();
	const otherAudio = new FakeAudio();
	const nextAudio = new FakeAudio();
	const old = playFrontendAudio(
		{ url: "https://example.com/old.wav" },
		context,
		dependencies(oldAudio),
	);
	const other = playFrontendAudio(
		{ url: "https://example.com/other.wav" },
		{ ...context, appId: "other" },
		dependencies(otherAudio),
	);
	const next = playFrontendAudio(
		{ url: "https://example.com/next.wav" },
		context,
		dependencies(nextAudio),
	);
	await expect(old).rejects.toMatchObject({ code: "interrupted" });
	expect(oldAudio.src).toBe("");
	expect(otherAudio.pause).not.toHaveBeenCalled();
	nextAudio.dispatchEvent(new Event("ended"));
	otherAudio.dispatchEvent(new Event("ended"));
	await Promise.all([next, other]);
});

test.each(["pagehide", "flow-like:device-inactive", "visibilitychange"])(
	"%s stops sound and removes its source",
	async (event) => {
		const audio = new FakeAudio();
		const playing = playFrontendAudio(
			{ url: "https://example.com/clip.wav" },
			context,
			dependencies(audio),
		);
		if (event === "visibilitychange") {
			doc.visibilityState = "hidden";
			doc.dispatchEvent(new Event(event));
		} else win.dispatchEvent(new Event(event));
		await expect(playing).rejects.toMatchObject({ code: "screen_closed" });
		expect(audio.src).toBe("");
		expect(audio.pause).toHaveBeenCalledTimes(1);
	},
);

test("navigation also rejects audio commands delayed in an earlier run", async () => {
	const epoch = getFrontendAudioEpoch();
	stopAllFrontendAudio();
	const audio = new FakeAudio();
	await expect(
		playFrontendAudio(
			{ url: "https://example.com/clip.wav" },
			{ ...context, frontendAudioEpoch: epoch },
			dependencies(audio),
		),
	).rejects.toMatchObject({ code: "screen_closed" });
	expect(audio.play).not.toHaveBeenCalled();
});

test("run cancellation removes the prompt and a late gesture cannot restart playback", async () => {
	const audio = new FakeAudio();
	audio.play.mockImplementationOnce(async () => {
		throw new DOMException("Activation", "NotAllowedError");
	});
	const controller = new AbortController();
	let play!: () => void;
	const dismiss = mock(() => {});
	const playing = playFrontendAudio(
		{ url: "https://example.com/clip.wav" },
		{ ...context, signal: controller.signal },
		{
			...dependencies(audio),
			prompt: async (action) => {
				play = action;
				return dismiss;
			},
		},
	);
	await flush();
	controller.abort();
	await expect(playing).rejects.toMatchObject({ code: "cancelled" });
	play();
	expect(audio.play).toHaveBeenCalledTimes(1);
	expect(dismiss).toHaveBeenCalledTimes(1);
});

test("a prompt loading after cancellation cannot leave a live toast", async () => {
	const audio = new FakeAudio();
	audio.play.mockImplementationOnce(async () => {
		throw new DOMException("Activation", "NotAllowedError");
	});
	const controller = new AbortController();
	let finish!: (dismiss: () => void) => void;
	let live!: () => boolean;
	const dismiss = mock(() => {});
	const prompt = new Promise<() => void>((resolve) => {
		finish = resolve;
	});
	const playing = playFrontendAudio(
		{ url: "https://example.com/clip.wav" },
		{ ...context, signal: controller.signal },
		{
			...dependencies(audio),
			prompt: async (_play, _cancel, isLive) => {
				live = isLive;
				return prompt;
			},
		},
	);
	await flush();
	controller.abort();
	await expect(playing).rejects.toMatchObject({ code: "cancelled" });
	expect(live()).toBe(false);
	finish(dismiss);
	await flush();
	expect(dismiss).toHaveBeenCalledTimes(1);
});

test("the request deadline includes time waiting for user activation", async () => {
	const audio = new FakeAudio();
	audio.play.mockImplementationOnce(async () => {
		throw new DOMException("Activation", "NotAllowedError");
	});
	const dismiss = mock(() => {});
	const playing = playFrontendAudio(
		{ url: "https://example.com/clip.wav" },
		{ ...context, deadline: Date.now() + 30 },
		{ ...dependencies(audio), prompt: async () => dismiss },
	);
	await expect(playing).rejects.toMatchObject({ code: "timeout" });
	expect(dismiss).toHaveBeenCalledTimes(1);
	expect(audio.src).toBe("");
});
