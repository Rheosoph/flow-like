import { describe, expect, it } from "bun:test";
import { BUBBLE_BLOB_COUNT, type BubbleFilmUniforms } from "./bubble-film";
import { springStep } from "./film-motion";
import {
	FISSION_CHIP_STAGGER,
	FISSION_COALESCE,
	FISSION_ENTRANCE_END,
	poseFission,
	poseFissionAtRest,
} from "./flowpilot-fission";

/** The mark's radius in film space, matching what the empty state measures at full size. */
const R = 0.48;

function emptyUniforms(): BubbleFilmUniforms {
	return {
		time: 0,
		focus: 0,
		boxX: 0.48,
		boxY: 0.48,
		mouseX: 0,
		mouseY: 0,
		mstr: 0,
		morph: 0,
		round: 0,
		spin: 0,
		teeth: 0,
		teethN: 8,
		sat: 0,
		pop: 0,
		spinMix: 0,
		bodyOff: 0,
		gradeX: 0,
		gradeY: 0,
		satL: 0,
		blob: new Float32Array(BUBBLE_BLOB_COUNT * 4),
		blobK: new Float32Array(BUBBLE_BLOB_COUNT),
		blobUv: new Float32Array(BUBBLE_BLOB_COUNT),
	};
}

function children(u: BubbleFilmUniforms) {
	const out: { x: number; y: number; r: number; k: number }[] = [];
	for (let i = 0; i < BUBBLE_BLOB_COUNT; i++) {
		if (u.blob[i * 4 + 3] <= 0) continue;
		out.push({
			x: u.blob[i * 4],
			y: u.blob[i * 4 + 1],
			r: u.blob[i * 4 + 2],
			k: u.blobK[i],
		});
	}
	return out;
}

// poseFission returns a shared scratch object valid only until the next call, so every sample has
// to be copied — comparing two live handles would compare one object with itself.
function poseAt(t: number) {
	const u = emptyUniforms();
	const body = { ...poseFission(t, R, u, null) };
	return { u, body, blobs: children(u) };
}

describe("poseFission entrance", () => {
	it("starts as a seed with nothing split off", () => {
		const { body, blobs } = poseAt(0);
		expect(body.scale).toBeLessThan(0.1);
		expect(body.bodyOff).toBe(0);
		expect(blobs).toHaveLength(0);
	});

	it("reaches full size before it starts to split", () => {
		const { body } = poseAt(0.37);
		expect(body.scale).toBeGreaterThan(0.95);
		expect(body.bodyOff).toBeLessThan(0.05);
	});

	it("hands the whole film to three children while split", () => {
		const { body, blobs } = poseAt(0.95);
		expect(body.bodyOff).toBeGreaterThan(0.95);
		expect(blobs.length).toBeGreaterThanOrEqual(3);
	});

	// Three children of equal size, 120 degrees apart, is what makes this read as one bubble
	// dividing rather than as three bubbles being placed.
	it("keeps the three children equal and evenly spaced", () => {
		const { blobs } = poseAt(1.1);
		const kids = blobs.slice(0, 3);
		expect(kids).toHaveLength(3);
		for (const kid of kids) expect(kid.r).toBeCloseTo(kids[0].r, 6);

		const angles = kids.map((k) => Math.atan2(k.y, k.x)).sort((a, b) => a - b);
		const gaps = [
			angles[1] - angles[0],
			angles[2] - angles[1],
			angles[0] + Math.PI * 2 - angles[2],
		];
		for (const gap of gaps) expect(gap).toBeCloseTo((Math.PI * 2) / 3, 3);
	});

	// A third of the area each, not a third of the volume: the film reads as a 2D surface.
	it("conserves area across the split", () => {
		const { blobs } = poseAt(1.1);
		const area = blobs.slice(0, 3).reduce((sum, kid) => sum + kid.r * kid.r, 0);
		expect(area / (R * R)).toBeGreaterThan(0.9);
		expect(area / (R * R)).toBeLessThan(1.3);
	});

	// And conserves it at every instant in between, not just at the ends. Ramping the body and
	// the children linearly in radius makes the mark deflate through the middle of the split and
	// swell back out — the single most obvious way for a division to read as a size change.
	it("never deflates while the split is in progress", () => {
		for (let t = 0.4; t <= 1.6; t += 0.005) {
			const { body, blobs } = poseAt(t);
			const area =
				body.scale * body.scale +
				blobs.slice(0, 3).reduce((sum, kid) => sum + (kid.r / R) ** 2, 0);
			expect(area).toBeGreaterThan(0.8);
			expect(area).toBeLessThan(1.25);
		}
	});

	// The neck is the whole gesture: heavily blended while the mass is one, essentially gone once
	// the children are clear of each other.
	it("thins the neck as the children separate", () => {
		const joined = poseAt(0.75).blobs[0].k;
		const parting = poseAt(0.9).blobs[0].k;
		const clear = poseAt(1.15).blobs[0].k;
		expect(joined).toBeGreaterThan(parting);
		expect(parting).toBeGreaterThan(clear);
		expect(clear).toBeLessThan(R * 0.1);
	});

	it("leaves a residue drop at the break and takes it back", () => {
		expect(poseAt(0.95).blobs.length).toBe(4);
		expect(poseAt(1.4).blobs.length).toBe(3);
	});

	it("fires its shockwave on the coalescence, not on the split", () => {
		expect(poseAt(FISSION_COALESCE).body.pop).toBeCloseTo(1, 3);
		expect(poseAt(FISSION_COALESCE - 0.3).body.pop).toBeLessThan(0.3);
	});

	it("returns to one whole bubble after the coalescence", () => {
		const { body, blobs } = poseAt(FISSION_ENTRANCE_END + 0.4);
		expect(body.bodyOff).toBe(0);
		expect(blobs).toHaveLength(0);
		expect(body.scale).toBeGreaterThan(0.85);
	});

	// Nothing may ever be invisible: if the body is gone, the children have to be carrying it.
	it("always has something on screen", () => {
		for (let t = 0; t <= 4; t += 0.01) {
			const { body, blobs } = poseAt(t);
			const visible = body.bodyOff < 0.99 || blobs.length > 0;
			expect(visible).toBe(true);
		}
	});

	// The mark's size may never step. A branch boundary that hands the body straight to a smaller
	// constant reads as the mark snapping smaller, which is the opposite of a bubble dividing.
	//
	// Asserted as a limit rather than against a threshold, because the inflate and the rebound are
	// both deliberately fast: halving the timestep halves the largest per-frame change of a
	// continuous curve, and does nothing at all to a step.
	it("never changes size in a single frame", () => {
		const largestStep = (dt: number) => {
			let worst = 0;
			let previous = poseAt(0).body.scale;
			for (let t = dt; t <= 4; t += dt) {
				const scale = poseAt(t).body.scale;
				worst = Math.max(worst, Math.abs(scale - previous));
				previous = scale;
			}
			return worst;
		};
		expect(largestStep(1 / 240)).toBeLessThan(largestStep(1 / 120) * 0.75);
		expect(largestStep(1 / 960)).toBeLessThan(largestStep(1 / 240) * 0.75);
	});

	// u_body_off is a switch: the shader deletes the body once it clears roughly box/8, so it may
	// only be raised after the children are big enough to be carrying the film themselves.
	it("only switches the body off once the children can carry it", () => {
		const full = R * 0.6;
		for (let t = 0; t <= 4; t += 0.005) {
			const { body, blobs } = poseAt(t);
			if (body.bodyOff <= 0.02) continue;
			const largest = Math.max(0, ...blobs.map((b) => b.r));
			expect(largest / full).toBeGreaterThan(0.4);
			// And the body it is deleting has to be a lump inside their blend, not the whole mark.
			expect(body.scale).toBeLessThan(0.35);
		}
	});

	// A pure function of time is what lets reduced motion pose the mark at rest rather than freeze
	// its first frame, and what keeps 60Hz and 120Hz on the same curve.
	it("is a pure function of elapsed time", () => {
		const a = poseAt(0.83);
		const b = poseAt(0.83);
		expect(b.body).toEqual(a.body);
		expect(b.blobs).toEqual(a.blobs);
	});
});

describe("poseFissionAtRest", () => {
	it("is one whole bubble with no split in progress", () => {
		const u = emptyUniforms();
		const body = poseFissionAtRest(R, u);
		expect(children(u)).toHaveLength(0);
		expect(body.bodyOff).toBe(0);
		expect(body.scale).toBeGreaterThan(0.9);
		expect(body.pop).toBeLessThan(0.01);
	});

	it("keeps the waist below the pinch", () => {
		for (let t = FISSION_ENTRANCE_END; t < 40; t += 0.05) {
			expect(Math.abs(poseAt(t).body.waist)).toBeLessThan(0.06);
		}
	});
});

describe("suggestion timing", () => {
	it("arrives on the coalescence, staggered", () => {
		const first = FISSION_COALESCE * 1000;
		const last = (FISSION_COALESCE + 2 * FISSION_CHIP_STAGGER) * 1000;
		expect(first).toBeGreaterThan(1000);
		expect(last - first).toBeLessThan(400);
		// The whole entrance must be over before the last suggestion is expected to be readable.
		expect(FISSION_ENTRANCE_END * 1000).toBeGreaterThan(last);
	});
});

describe("springStep", () => {
	it("starts at rest and settles at one", () => {
		expect(springStep(0, 2.8, 0.5)).toBe(0);
		expect(springStep(-1, 2.8, 0.5)).toBe(0);
		expect(springStep(4, 2.8, 0.5)).toBeCloseTo(1, 4);
	});

	it("overshoots when underdamped", () => {
		let peak = 0;
		for (let t = 0; t < 2; t += 0.005) {
			peak = Math.max(peak, springStep(t, 2.7, 0.3));
		}
		expect(peak).toBeGreaterThan(1.05);
	});
});
