import { describe, expect, test } from "bun:test";
import type { IBotComputerView } from "../../state/backend-state/bot-state";
import type { ScreenStream } from "./screen-stream";
import { ScreenViewer } from "./screen-viewer";

const text = (value: string) => new TextEncoder().encode(value);

function message(kind: number, payload: Uint8Array): Uint8Array {
	const bytes = new Uint8Array(5 + payload.length);
	bytes[0] = kind;
	new DataView(bytes.buffer).setUint32(1, payload.length);
	bytes.set(payload, 5);
	return bytes;
}

const state = (json: object) => message(1, text(JSON.stringify(json)));

function frame(kind: "browser" | "computer", ageMs = 0): Uint8Array {
	const meta = text(
		JSON.stringify({
			width: 10,
			height: 6,
			kind,
			caption: "cap",
			age_ms: ageMs,
		}),
	);
	const payload = new Uint8Array(2 + meta.length + 2);
	payload[1] = meta.length;
	payload.set(meta, 2);
	payload.set([0xff, 0xd8], 2 + meta.length);
	return message(2, payload);
}

class FakeStream implements ScreenStream {
	written: Uint8Array[] = [];
	resets = 0;
	private queue: Array<Uint8Array | null> = [];
	private waiting: ((chunk: Uint8Array | null) => void) | null = null;

	feed(chunk: Uint8Array | null) {
		if (this.waiting) {
			this.waiting(chunk);
			this.waiting = null;
		} else this.queue.push(chunk);
	}
	read() {
		if (this.queue.length) return Promise.resolve(this.queue.shift() ?? null);
		return new Promise<Uint8Array | null>((resolve) => {
			this.waiting = resolve;
		});
	}
	async write(bytes: Uint8Array) {
		this.written.push(bytes);
	}
	reset() {
		this.resets++;
		this.feed(null);
	}
}

const tick = () => new Promise((resolve) => setTimeout(resolve, 5));

describe("ScreenViewer", () => {
	test("starts asleep", () => {
		const viewer = new ScreenViewer(new FakeStream());
		expect(viewer.view).toMatchObject({
			state: "asleep",
			image: null,
			taken_over: false,
		});
		viewer.close();
	});

	test("maps a computer frame with a display to a controllable active view", async () => {
		const stream = new FakeStream();
		const viewer = new ScreenViewer(stream, { now: () => 1000 });
		const seen: IBotComputerView[] = [];
		viewer.subscribe((view) => seen.push(view));
		stream.feed(
			state({
				taken_over: true,
				asleep: false,
				display: "virtual",
				kind: "computer",
			}),
		);
		stream.feed(frame("computer", 30));
		await tick();
		expect(viewer.view).toMatchObject({
			state: "active",
			kind: "computer",
			caption: "cap",
			image: "data:image/jpeg;base64,/9g=",
			taken_over: true,
			can_control: true,
			width: 10,
			height: 6,
			age_ms: 30,
		});
		expect(seen.length).toBe(2);
		viewer.close();
	});

	test("browser frames and display none are watch only", async () => {
		const stream = new FakeStream();
		const viewer = new ScreenViewer(stream);
		stream.feed(
			state({ taken_over: false, asleep: false, display: "virtual" }),
		);
		stream.feed(frame("browser"));
		await tick();
		expect(viewer.view.can_control).toBe(false);
		stream.feed(state({ taken_over: false, asleep: false, display: "none" }));
		stream.feed(frame("computer"));
		await tick();
		expect(viewer.view.can_control).toBe(false);
		viewer.close();
	});

	test("an asleep state or a frame older than 180 s shows asleep", async () => {
		let now = 0;
		const stream = new FakeStream();
		const viewer = new ScreenViewer(stream, { now: () => now });
		stream.feed(
			state({ taken_over: false, asleep: false, display: "session" }),
		);
		stream.feed(frame("computer", 100));
		await tick();
		expect(viewer.view.state).toBe("active");
		now = 180_000 - 99;
		expect(viewer.view.state).toBe("asleep");
		now = 0;
		stream.feed(state({ taken_over: false, asleep: true, display: "session" }));
		await tick();
		expect(viewer.view.state).toBe("asleep");
		viewer.close();
	});

	test("a notice stays for 8 s or until the next control", async () => {
		let now = 0;
		const stream = new FakeStream();
		const viewer = new ScreenViewer(stream, { now: () => now });
		stream.feed(message(3, text('{"code":"input","message":"no display"}')));
		await tick();
		expect(viewer.view.notice).toBe("input: no display");
		now = 7_999;
		expect(viewer.view.notice).toBe("input: no display");
		now = 8_000;
		expect(viewer.view.notice).toBeNull();
		now = 0;
		stream.feed(message(3, text('{"code":"a","message":"b"}')));
		await tick();
		await viewer.send({ type: "stop" });
		expect(viewer.view.notice).toBeNull();
		viewer.close();
	});

	test("send writes encoded controls in order and fails after the end", async () => {
		const stream = new FakeStream();
		const viewer = new ScreenViewer(stream);
		await viewer.send({ type: "take_over" });
		await viewer.send({ type: "hand_back" });
		expect(stream.written.map((bytes) => bytes[0])).toEqual([16, 16]);
		viewer.close();
		expect(await viewer.ended).toBe("closed");
		expect(stream.resets).toBe(1);
		await expect(viewer.send({ type: "stop" })).rejects.toThrow("ended");
	});

	test("ends when the stream ends or sends garbage", async () => {
		const closed = new FakeStream();
		const first = new ScreenViewer(closed);
		closed.feed(null);
		expect(await first.ended).toBe("closed");

		const bad = new FakeStream();
		const second = new ScreenViewer(bad);
		bad.feed(Uint8Array.of(9, 0, 0, 0, 0));
		expect(await second.ended).toContain("playground");
		expect(bad.resets).toBe(1);
	});
});
