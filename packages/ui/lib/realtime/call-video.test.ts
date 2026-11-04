import { describe, expect, test } from "bun:test";
import { type VideoCaptureState, createVideoCapture } from "./call-video";

class Track extends EventTarget {
	readyState = "live";
	stops = 0;
	stop() {
		this.stops++;
		this.readyState = "ended";
	}
	end() {
		this.readyState = "ended";
		this.dispatchEvent(new Event("ended"));
	}
}

function captured(id: string) {
	const track = new Track();
	const stream = {
		id,
		getTracks: () => [track],
		getVideoTracks: () => [track],
	} as unknown as MediaStream;
	return { track, stream };
}

describe("camera and screen capture", () => {
	test("camera and screen coexist, and stopping one preserves the other", async () => {
		const camera = captured("camera");
		const screen = captured("screen");
		const snapshots: VideoCaptureState[] = [];
		const capture = createVideoCapture(
			(state) => snapshots.push(state),
			async (kind) => (kind === "camera" ? camera.stream : screen.stream),
		);
		await capture.toggle("camera");
		await capture.toggle("screen");
		expect(capture.state.camera).toBe(camera.stream);
		expect(capture.state.screen).toBe(screen.stream);
		await capture.toggle("camera");
		expect(camera.track.stops).toBe(1);
		expect(screen.track.stops).toBe(0);
		expect(capture.state.screen).toBe(screen.stream);
		capture.dispose();
		expect(screen.track.stops).toBe(1);
		expect(snapshots.at(-1)?.camera).toBeNull();
	});

	test("browser Stop sharing releases the screen and updates controls", async () => {
		const screen = captured("screen");
		const snapshots: VideoCaptureState[] = [];
		const capture = createVideoCapture(
			(state) => snapshots.push(state),
			async () => screen.stream,
		);
		await capture.toggle("screen");
		screen.track.end();
		expect(capture.state.screen).toBeNull();
		expect(snapshots.at(-1)?.screen).toBeNull();
		expect(screen.track.stops).toBe(1);
	});

	test("a permission prompt resolving after leave immediately stops its tracks", async () => {
		const camera = captured("camera");
		let resolve!: (stream: MediaStream) => void;
		const snapshots: VideoCaptureState[] = [];
		const capture = createVideoCapture(
			(state) => snapshots.push(state),
			() =>
				new Promise((done) => {
					resolve = done;
				}),
		);
		const pending = capture.toggle("camera");
		expect(capture.state.pending).toBe("camera");
		capture.dispose();
		const count = snapshots.length;
		resolve(camera.stream);
		await pending;
		expect(camera.track.stops).toBe(1);
		expect(snapshots).toHaveLength(count);
		expect(capture.state.camera).toBeNull();
	});

	test("duplicate clicks cannot open concurrent permission prompts", async () => {
		const screen = captured("screen");
		let resolve!: (stream: MediaStream) => void;
		let requests = 0;
		const capture = createVideoCapture(
			() => {},
			() => {
				requests++;
				return new Promise((done) => {
					resolve = done;
				});
			},
		);
		const pending = capture.toggle("screen");
		await capture.toggle("screen");
		await capture.toggle("camera");
		expect(requests).toBe(1);
		resolve(screen.stream);
		await pending;
		capture.dispose();
	});

	test("denied and unavailable capture errors do not stop an active camera", async () => {
		const camera = captured("camera");
		for (const [name, reason] of [
			["NotAllowedError", "denied"],
			["NotSupportedError", "unavailable"],
		] as const) {
			const capture = createVideoCapture(
				() => {},
				async (kind) => {
					if (kind === "camera") return camera.stream;
					throw new DOMException("Capture rejected", name);
				},
			);
			await capture.toggle("camera");
			await capture.toggle("screen");
			expect(capture.state.error).toEqual({ source: "screen", reason });
			expect(capture.state.camera).toBe(camera.stream);
			expect(capture.state.pending).toBeNull();
		}
	});
});
