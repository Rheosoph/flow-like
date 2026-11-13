import { describe, expect, test } from "bun:test";
import { CUE_INTERVAL_MS, CUE_PEAK_GAIN, createCallCues } from "./call-cues";

function fixture() {
	const destination = {} as AudioNode;
	const oscillators: {
		frequency: { value: number };
		connections: unknown[];
		starts: number[];
		stops: number[];
		onended: (() => void) | null;
		disconnect: () => void;
	}[] = [];
	const gains: {
		connections: unknown[];
		ramps: number[];
		gain: {
			setValueAtTime: (value: number, at: number) => void;
			linearRampToValueAtTime: (value: number, at: number) => void;
		};
		disconnect: () => void;
	}[] = [];
	const context = {
		state: "running",
		currentTime: 5,
		createOscillator() {
			const oscillator = {
				type: "",
				frequency: { value: 0 },
				connections: [] as unknown[],
				starts: [] as number[],
				stops: [] as number[],
				onended: null as (() => void) | null,
				connect(node: unknown) {
					this.connections.push(node);
				},
				disconnect() {},
				start(at: number) {
					this.starts.push(at);
				},
				stop(at: number) {
					this.stops.push(at);
				},
			};
			oscillators.push(oscillator);
			return oscillator;
		},
		createGain() {
			const gain = {
				connections: [] as unknown[],
				ramps: [] as number[],
				gain: {
					setValueAtTime() {},
					linearRampToValueAtTime(value: number) {
						gain.ramps.push(value);
					},
				},
				connect(node: unknown) {
					this.connections.push(node);
				},
				disconnect() {},
			};
			gains.push(gain);
			return gain;
		},
	};
	const pending = new Map<number, { callback: () => void; ms: number }>();
	let next = 0;
	const timers = {
		set: (callback: () => void, ms: number) => {
			pending.set(++next, { callback, ms });
			return next;
		},
		clear: (handle: unknown) => {
			pending.delete(handle as number);
		},
	};
	const fire = () => {
		const [id, timer] = [...pending][0] ?? [];
		if (id === undefined || !timer) return;
		pending.delete(id);
		timer.callback();
	};
	const cues = createCallCues(
		context as unknown as AudioContext,
		destination,
		timers,
	);
	return { cues, context, destination, oscillators, gains, pending, fire };
}

describe("createCallCues", () => {
	test("a soft two-tone tick goes only to the given destination", () => {
		const f = fixture();
		f.cues.thinking.start();
		expect(f.oscillators).toHaveLength(2);
		expect(f.oscillators.map((o) => o.frequency.value)).toEqual([660, 880]);
		expect(f.oscillators[1]?.starts[0]).toBeGreaterThan(
			f.oscillators[0]?.starts[0] ?? 0,
		);
		for (const [index, gain] of f.gains.entries()) {
			expect(f.oscillators[index]?.connections).toEqual([gain]);
			expect(gain.connections).toEqual([f.destination]);
			expect(Math.max(...gain.ramps)).toBeLessThanOrEqual(0.05);
		}
		expect(CUE_PEAK_GAIN).toBeLessThanOrEqual(0.05);
	});

	test("repeats every 2.4 s while active and stops cleanly", () => {
		const f = fixture();
		f.cues.thinking.start();
		f.cues.thinking.start();
		expect([...f.pending.values()].map((timer) => timer.ms)).toEqual([
			CUE_INTERVAL_MS,
		]);
		f.fire();
		expect(f.oscillators).toHaveLength(4);
		f.cues.thinking.stop();
		expect(f.pending.size).toBe(0);
		f.cues.thinking.stop();
		expect(f.oscillators).toHaveLength(4);
		f.cues.thinking.start();
		expect(f.oscillators).toHaveLength(6);
	});

	test("nothing sounds while the context is suspended or after disposal", () => {
		const f = fixture();
		f.context.state = "suspended";
		f.cues.thinking.start();
		expect(f.oscillators).toHaveLength(0);
		f.cues.dispose();
		f.cues.thinking.start();
		expect(f.pending.size).toBe(0);
	});
});
