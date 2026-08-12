/**
 * How a chat composer's draft drives the empty-state mark.
 *
 * Two pieces: a channel the composer writes its draft into, and a response that turns those edits
 * into the smooth 0…1 signals a render loop wants. Deliberately outside React — this changes on
 * every keystroke and its only consumers are canvas loops that already run once per frame, so a
 * store here would re-render a whole chat surface to feed an animation that samples it anyway.
 *
 * One channel per surface, never a module singleton: an app chat and the FlowPilot overlay can be
 * open at once, and typing in one must not stir the other's mark.
 */

export interface IComposerActivity {
	/** Bumped on every edit, so a reader spots a keystroke without carrying timestamps. */
	readonly revision: number;
	/** Characters in the draft. */
	readonly length: number;
}

export interface IComposerActivityChannel {
	report(content: string): void;
	/** Returns the shared record itself — read the fields, never hold it across frames. */
	read(): IComposerActivity;
}

export function createComposerActivity(): IComposerActivityChannel {
	const activity = { revision: 0, length: 0 };
	let current = "";
	return {
		report(content: string) {
			if (content === current) return;
			current = content;
			activity.revision += 1;
			activity.length = content.length;
		},
		read: () => activity,
	};
}

// `typing` is an accumulator: every edit adds TYPING_GAIN, every second sheds TYPING_DECAY of what
// is left. Steady typing therefore pins it near 1, a slow hunt-and-peck only lifts it partway, and
// a pause drains it in about two seconds — the mark tracks how hard the composer is being worked
// rather than reacting to individual keys.
const TYPING_GAIN = 0.34;
const TYPING_DECAY = 1.5;
/** Quiet, in seconds, after which the next keystroke reads as "starting to write" again. */
const PERK_GAP = 0.9;
/** Draft length at which the mark reads as full. */
const FULL_DRAFT = 220;

/**
 * One step of the typing accumulator, exported because the tuning *is* the feature: steady writing
 * has to pin it near 1, a single key must barely register, and a pause has to drain it inside a
 * couple of seconds however fast the display runs.
 */
export function stepTypingResponse(typing: number, keyed: boolean, dt: number) {
	const gained = keyed ? Math.min(1, typing + TYPING_GAIN) : typing;
	return gained * Math.exp(-TYPING_DECAY * dt);
}

export interface ITypingResponse {
	/** 0…1 — how hard the composer is being worked right now. */
	readonly typing: number;
	/** 0…1 — how much text is sitting in the draft. */
	readonly fullness: number;
	/** True only on the frame a burst of writing begins, after a pause. */
	readonly perked: boolean;
}

/**
 * Advances the typing signals for one frame. Owns its own state, so each mark gets its own; the
 * returned object is reused every frame and must not be held.
 */
export function createTypingResponse(channel: IComposerActivityChannel) {
	const out = { typing: 0, fullness: 0, perked: false };
	let quiet = PERK_GAP;
	let lastRevision = channel.read().revision;

	return {
		/**
		 * @param enabled false eases every signal back to rest rather than freezing it, so
		 * switching the setting off mid-draft settles the mark instead of snapping it.
		 */
		advance(enabled: boolean, dt: number): ITypingResponse {
			let keyed = false;
			out.perked = false;
			if (enabled) {
				const activity = channel.read();
				keyed = activity.revision !== lastRevision;
				if (keyed) {
					lastRevision = activity.revision;
					// The first keystroke after a pause is the one that reads as "it noticed".
					out.perked = quiet >= PERK_GAP && activity.length > 0;
					quiet = 0;
				} else {
					quiet += dt;
				}
				out.fullness = approach(
					out.fullness,
					Math.min(1, activity.length / FULL_DRAFT),
					2.2,
					dt,
				);
			} else {
				out.fullness = approach(out.fullness, 0, 2.2, dt);
			}
			out.typing = stepTypingResponse(out.typing, keyed, dt);
			return out;
		},
	};
}

/** Frame-rate independent damping, so the response settles the same at 60 and 120Hz. */
function approach(current: number, target: number, rate: number, dt: number) {
	return current + (target - current) * (1 - Math.exp(-rate * dt));
}

/** How far the lean pulls the bulge toward the composer, as a fraction of the mark's radius. */
const LEAN_REACH = 0.9;
/** Scratch for `applyComposerLean`, reused so a render loop never allocates. */
const LEAN = { x: 0, y: 0 };

/**
 * Pulls a mark's bulge onto its lower rim while the user writes. The composer always sits below
 * the mark, so writing turns it to face the text rather than leaving it aimed at an abandoned
 * pointer. Shared by both soap-film loops; the returned object is valid until the next call.
 *
 * @param x,y where the bulge would otherwise sit, in film space (y up).
 * @param radius the film's own radius, so the lean scales with the mark.
 */
export function applyComposerLean(
	x: number,
	y: number,
	typing: number,
	clock: number,
	radius: number,
) {
	if (typing <= 0) {
		LEAN.x = x;
		LEAN.y = y;
		return LEAN;
	}
	// A slow sway keeps the lean alive — a bulge pinned dead-centre-bottom reads as frozen.
	const sway = Math.sin(clock * 2.1) * radius * 0.22;
	LEAN.x = x * (1 - typing) + sway * typing;
	LEAN.y = y * (1 - typing) + -radius * LEAN_REACH * typing;
	return LEAN;
}

/**
 * Decays the one-frame `perked` flag into a smooth 0…1 impulse.
 *
 * Deliberately *not* the shader's `u_pop`: that uniform is a ring's remaining lifetime, not its
 * amplitude — seeding it below 1 starts the ring most of the way through its travel, which renders
 * as a thin halo detached from the rim instead of a quieter pulse. A burst of typing is a much
 * smaller event than an entrance anyway, so it gets a swell rather than a shockwave.
 */
export function decayPerk(perk: number, perked: boolean, dt: number) {
	return perked ? 1 : perk * Math.exp(-3.4 * dt);
}
