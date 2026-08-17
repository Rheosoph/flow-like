/**
 * Fission — the FlowPilot empty state's entrance.
 *
 * The mark does not spawn satellites. It loses its waist, breaks into three bubbles of equal
 * volume that swing out across the mark, hang, then rush back and fuse; the coalescence, not the
 * split, is what hands over the suggestions.
 *
 * The break is not keyframed. Three blobs take over the body's volume while still sharing one
 * smooth-min field, so at high blending they read as a single lumpy mass; letting that blend decay
 * is what thins the neck into a thread and snaps it. Real inertial-capillary pinch-off accelerates
 * as it goes and leaves a satellite drop at the break, and both of those are the difference
 * between fluid and a blend parameter animating.
 *
 * Every function here is a pure function of elapsed seconds, so the pose can be evaluated at any
 * instant — which is what gives `prefers-reduced-motion` a real resting pose rather than a frozen
 * first frame, and what keeps the timings identical at 60 and 120Hz.
 */

import {
	type BubbleFilmUniforms,
	clearBubbleBlobs,
	setBubbleBlob,
} from "./bubble-film";
import { lerp, segment, smoothstep, springStep } from "./film-motion";

// Seconds from mount. The whole entrance has to land before the user has finished reading the
// suggestions, so the hang is deliberately short — it is the pinch and the fusion that carry it.
const INFLATE = 0;
const SPLIT_FROM = 0.38;
const SPLIT_TO = 0.72;
const NECK_FROM = 0.58;
const NECK_TO = 1.02;
const APART_FROM = 0.55;
const APART_TO = 1.2;
const RETURN_FROM = 1.32;
/** The children meet. Everything downstream of the entrance hangs off this instant. */
export const FISSION_COALESCE = 1.72;
export const FISSION_ENTRANCE_END = 2.05;
/** How long after the coalescence each suggestion arrives. */
export const FISSION_CHIP_STAGGER = 0.08;

/**
 * Two dimensions splitting into three, so each child keeps a third of the area rather than a third
 * of the volume — 1/sqrt(3). Getting this wrong is what makes a split read as three new bubbles
 * appearing rather than as one bubble becoming three.
 */
const CHILD_RADIUS = 0.6;
/** What is left of the body once the children have taken its area, before it is switched off. */
const SPLIT_SCALE = 0.08;
/** Furthest the children travel, as a fraction of the mark's own radius. */
const ORBIT = 1.3;
/** Smooth-min radius at full merge, as a fraction of the mark's radius. */
const MERGED_SMOOTH = 0.62;
const SEPARATE_SMOOTH = 0.03;
/** The drop left behind at the break, reabsorbed as the children swing away. */
const RESIDUE_RADIUS = 0.1;
const RESIDUE_FROM = 0.86;
const RESIDUE_TO = 1.3;

export interface FissionBody {
	/** Body radius as a fraction of the mark's radius. */
	readonly scale: number;
	/** 0 body present … 1 body gone, the children carrying the film. */
	readonly bodyOff: number;
	/** Anisotropy: positive stretches x and squashes y by the same amount. */
	readonly waist: number;
	/** Shockwave amplitude, 1 at the instant it fires. */
	readonly pop: number;
	/** Extra focus while the film is working, above the pose's own. */
	readonly effort: number;
}

const BODY: {
	scale: number;
	bodyOff: number;
	waist: number;
	pop: number;
	effort: number;
} = { scale: 0, bodyOff: 0, waist: 0, pop: 0, effort: 0 };

/**
 * How far through the break the film is: 1 while the three children are still one mass, 0 once
 * they are fully separate. Raised to a power because inertial-capillary pinch-off accelerates —
 * the thread thins slowly and then runs away, where a linear fade reads as a blend animating.
 */
function neckThickness(t: number) {
	const apart = smoothstep(segment(t, NECK_FROM, NECK_TO));
	const back = smoothstep(segment(t, RETURN_FROM, FISSION_COALESCE));
	const merged = 1 - apart * (1 - back);
	return merged * merged * Math.sqrt(merged);
}

/** Where child `index` sits, as a fraction of the mark's radius from its centre. */
function childOrbit(t: number) {
	const out = smoothstep(segment(t, APART_FROM, APART_TO));
	const back = smoothstep(segment(t, RETURN_FROM, FISSION_COALESCE));
	return ORBIT * out * (1 - back);
}

/**
 * Poses the body for `t` seconds into the entrance, and writes the children into `u`.
 *
 * `radius` is the mark's own radius in film space, so every distance here scales with the mark.
 * `pointer`, when the pointer is over the page, drags whichever child is nearest it off its orbit.
 * The returned object is shared and valid only until the next call.
 */
export function poseFission(
	t: number,
	radius: number,
	u: BubbleFilmUniforms,
	pointer?: { readonly x: number; readonly y: number } | null,
): FissionBody {
	const split = smoothstep(segment(t, SPLIT_FROM, SPLIT_TO));
	const reformed = smoothstep(
		segment(t, FISSION_COALESCE - 0.12, FISSION_COALESCE + 0.06),
	);
	const rebound = springStep(t - FISSION_COALESCE, 2.7, 0.3);

	// u_body_off is a switch, not a fade: the shader pushes the body's whole field away, so it
	// vanishes as soon as the value clears roughly box/8. Flipping it has to happen late, once
	// the body has already given its area to the children and is buried inside their blend —
	// otherwise a full-size mark is deleted in one frame partway through the ramp.
	const hidden = smoothstep(segment(t, SPLIT_TO, SPLIT_TO + 0.14));
	BODY.bodyOff = hidden * (1 - reformed);
	// The area is handed over across the same window the children grow in, never at a branch
	// boundary — a step here reads as the mark snapping smaller, the opposite of a split. And it
	// is handed over as AREA, not as radius: the body keeps sqrt(1 - split) of its radius while
	// each child takes sqrt(split) of its own, so the three of them plus what is left of the
	// parent always add up to one bubble. Ramping both radii linearly instead makes the mark
	// visibly deflate through the middle of the split and swell back out, which is not division.
	const inflate = 0.055 + 0.945 * springStep(t - INFLATE, 2.8, 0.5);
	BODY.scale =
		t < FISSION_COALESCE
			? inflate * Math.sqrt(1 - split) + SPLIT_SCALE * split
			: lerp(SPLIT_SCALE, 1, rebound);
	// Past the entrance the mark keeps almost splitting: a slow stretch on a fifteen-second period
	// that never reaches the pinch, so the rest loop is the same idea at an amplitude you can sit
	// in front of. Ramped in, or the waist would appear the instant the rebound lands.
	BODY.waist =
		0.055 *
		Math.sin((t - FISSION_ENTRANCE_END) * 0.42) *
		smoothstep(segment(t, FISSION_ENTRANCE_END, FISSION_ENTRANCE_END + 1.2));
	BODY.pop =
		t >= FISSION_COALESCE
			? Math.exp(-(t - FISSION_COALESCE) * 3.4)
			: t >= 0.24
				? Math.exp(-(t - 0.24) * 4.4) * 0.5
				: 0;
	// The film works hardest while the neck is thinning, and settles once the children are clear.
	BODY.effort =
		0.3 *
		smoothstep(segment(t, SPLIT_FROM + 0.1, NECK_FROM + 0.2)) *
		(1 - smoothstep(segment(t, NECK_TO, NECK_TO + 0.4)));

	if (split <= 0.001 || t > FISSION_COALESCE + 0.22) return BODY;

	const orbit = childOrbit(t) * radius;
	const thickness = neckThickness(t);
	const smooth = radius * lerp(SEPARATE_SMOOTH, MERGED_SMOOTH, thickness);
	// sqrt for the same reason as the body: a child takes up area linearly, so its radius grows as
	// the square root. The two curves are inverses of each other and the total never dips.
	const born = Math.sqrt(split);
	const gone = smoothstep(
		segment(t, FISSION_COALESCE - 0.06, FISSION_COALESCE + 0.12),
	);
	const childRadius = radius * CHILD_RADIUS * born * (1 - gone);

	for (let i = 0; i < 3; i++) {
		const angle = t * 0.5 + (i * Math.PI * 2) / 3;
		// Each child breathes on its own phase, so three equal bubbles never read as one rigid
		// object rotating.
		const reach = orbit * (1 + 0.06 * Math.sin(t * 1.3 + i * 2.1));
		let x = Math.cos(angle) * reach;
		let y = Math.sin(angle) * reach;
		if (pointer) {
			const dx = pointer.x - x;
			const dy = pointer.y - y;
			const pull =
				Math.exp(-((dx * dx + dy * dy) / (radius * radius)) * 0.9) * 0.18;
			x += dx * pull;
			y += dy * pull;
		}
		setBubbleBlob(u, i, x, y, childRadius, smooth);
	}

	// The residue: a break leaves a drop behind, and the parent takes it back. Without it the
	// three children read as having been placed rather than torn. Dropped while it is still a few
	// pixels across — a shell shrinking the whole way to nothing ends as a singular bright point.
	const shrink = smoothstep(segment(t, RESIDUE_FROM, RESIDUE_TO));
	if (t > RESIDUE_FROM && shrink < 0.8) {
		setBubbleBlob(
			u,
			3,
			0,
			0,
			radius * RESIDUE_RADIUS * (1 - shrink),
			radius * 0.16 * (1 - shrink),
		);
	}

	return BODY;
}

/** The pose the mark rests in — what reduced motion is shown, and where the loop settles. */
export function poseFissionAtRest(radius: number, u: BubbleFilmUniforms) {
	clearBubbleBlobs(u);
	return poseFission(FISSION_ENTRANCE_END + 4, radius, u, null);
}
