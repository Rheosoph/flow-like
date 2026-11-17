import type { DenEdge, GooAnchor, GooBlob } from "./goo-renderer";
import { BOUNCY, JELLY, SMOOTH, Spring, Spring2, TIGHT } from "./spring";

export interface GooPoint {
	x: number;
	y: number;
}

export interface GooRect {
	x: number;
	y: number;
	width: number;
	height: number;
}

export const DEN_AVATAR = 24;
export const PERCH_AVATAR = 32;
export const PERCH_BLOB = 21;
export const PILL_HEIGHT = 44;

export interface DenSlot extends GooPoint {
	botId: string;
}

/** A den's resting outline and where each bot sits in it. */
export interface DenShape {
	edge: DenEdge;
	/** On the right edge `rect.x + rect.width` is the edge; on the top edge `rect.y` is. */
	rect: GooRect;
	radius: number;
	slots: DenSlot[];
	/** Under the hardware notch nothing is drawn; bots sit in the ears beside it. */
	wrapsNotch: boolean;
}

/** Where a bot is, besides its slot in the den. */
export type BotPlace =
	| { kind: "drag"; x: number; y: number }
	| {
			kind: "perch";
			/** What it sits on: a window, a node, a page. */
			targetId: string;
			x: number;
			y: number;
			/** The edge it stands on; null when it sits inside its target. */
			edgeY: number | null;
			visible: boolean;
	  }
	/** A den bot whose prompt hangs beside the den. */
	| { kind: "drop"; x: number; y: number };

/** What a bot is busy with, shown in its goo: none means idle. */
export type GooMotion = "working" | "thinking" | "waiting";

export interface SceneInput {
	den: DenShape;
	places: ReadonlyMap<string, BotPlace>;
	/** The bot whose prompt is open, morphed into a pill of this width, raised by `lift`. */
	pill: { botId: string; width: number; lift?: number } | null;
	hoverBot: string | null;
	/** The pointer offset while the den is pulled. */
	pull: GooPoint | null;
	screenWidth: number;
	/** The den has no room: it shrinks into its edge and the bots at home fade with it. */
	tucked?: boolean;
	/** What each bot is busy with. */
	activity?: ReadonlyMap<string, GooMotion>;
	/** The pointer while it is near the den: the den bulges toward it. */
	pointer?: GooPoint | null;
}

export interface BotNode {
	x: number;
	y: number;
	size: number;
	opacity: number;
	/** Squash and stretch of the avatar; 1 at rest. */
	scaleX: number;
	scaleY: number;
	/** Lean into the motion, in degrees. */
	tilt: number;
	/** 0 resting, 1 held up in a drag. */
	lift: number;
	/** Where the prompt pill is once it has mostly formed. */
	pill: GooRect | null;
}

export interface SceneFrame {
	blobs: GooBlob[];
	anchor: GooAnchor;
	den: GooRect;
	bots: ReadonlyMap<string, BotNode>;
}

const NECK_REACH = 92;
const PULL_REACH = 150;
/** Below this speed (px/s) a droplet leaves no tail. */
const TAIL_SPEED = 70;
const TAIL_PER_SPEED = 0.05;
const TAIL_MAX = 58;
/** How far a landing flattens the droplet, as a share of its radius. */
const SQUASH = 0.22;
const LANDING_KICK = 19;
const TILT_PER_SPEED = 0.022;
const TILT_MAX = 16;
/** The den's recoil when a bot leaves it (px/s on its depth); a return gulps at 60 %. */
const DEN_KICK = 170;
/** A neck snaps and retracts with this spring; it reaches out again with the same one. */
const TETHER = { stiffness: 420, damping: 28 };
/** The den grows out of its edge, then its bots pop in one after another. */
const INTRO_DELAY = 0.16;
const INTRO_STAGGER = 0.085;
const SPLASH_GRAVITY = 1100;
const SPLASH_LIFE = 0.55;
/** The pointer bulges the den once it comes this close. */
const BUMP_REACH = 70;
const HEARTBEAT = 1.25;

type PlaceKind = BotPlace["kind"] | "home";

interface BotSprings {
	pos: Spring2;
	/** 0 in the den, 1 out in the open. */
	out: Spring;
	/** 0 a round blob, 1 the prompt pill. */
	morph: Spring;
	/** 1 shown, 0 hidden while its target is covered. */
	fade: Spring;
	/** A landing's squash: kicked, then it wobbles back to 0. */
	impact: Spring;
	lift: Spring;
	/** 1 while a neck ties it to the den, 0 once that neck snapped back. */
	tether: Spring;
	/** Seconds on the scene clock when it pops in during the intro; 0 once it has. */
	appearAt: number;
	/** The droplets a landing sprays out. */
	splash: Droplet[];
	/** Phase offset, so busy bots don't move in step. */
	phase: number;
	place: PlaceKind;
}

interface Droplet {
	x: number;
	y: number;
	vx: number;
	vy: number;
	radius: number;
	/** 1 when sprayed, 0 when it has merged back. */
	life: number;
}

function lerp(a: number, b: number, t: number): number {
	return a + (b - a) * t;
}

function clamp(value: number, limit: number): number {
	return Math.max(-limit, Math.min(limit, value));
}

/** Rubber band: moves freely at first, then ever harder. */
export function stretched(distance: number, reach: number): number {
	return reach * (1 - Math.exp(-Math.max(0, distance) / reach));
}

function circle(x: number, y: number, radius: number): GooBlob {
	return { x, y, hw: radius, hh: radius, radius };
}

function stroke(
	from: GooPoint,
	fromRadius: number,
	to: GooPoint,
	toRadius: number,
): GooBlob {
	return {
		x: from.x,
		y: from.y,
		hw: 0,
		hh: 0,
		radius: fromRadius,
		to: { x: to.x, y: to.y, radius: toRadius },
	};
}

/** A viscous neck: thick where it leaves, thin at its waist, swelling into what it carries. */
function neck(
	from: GooPoint,
	fromRadius: number,
	to: GooPoint,
	toRadius: number,
	waist: number,
): GooBlob[] {
	const middle = { x: lerp(from.x, to.x, 0.55), y: lerp(from.y, to.y, 0.55) };
	return [
		stroke(from, fromRadius, middle, waist),
		stroke(middle, waist, to, toRadius),
	];
}

/**
 * The den's own coordinates: `along` runs with its edge, `out` is the distance from the edge into
 * the screen. The top edge maps them to x and y; the right edge to y and the distance from its x.
 */
export class DenFrame {
	constructor(
		readonly edge: DenEdge,
		/** The edge's y (top) or x (right). */
		readonly at: number,
	) {}

	static of(den: Pick<DenShape, "edge" | "rect">): DenFrame {
		return den.edge === "top"
			? new DenFrame("top", den.rect.y)
			: new DenFrame("right", den.rect.x + den.rect.width);
	}

	point(along: number, out: number): GooPoint {
		return this.edge === "top"
			? { x: along, y: this.at + out }
			: { x: this.at - out, y: along };
	}

	along(point: GooPoint): number {
		return this.edge === "top" ? point.x : point.y;
	}

	out(point: GooPoint): number {
		return this.edge === "top" ? point.y - this.at : this.at - point.x;
	}

	/** A screen offset split into its along and outward parts. */
	split(offset: GooPoint): { along: number; out: number } {
		return this.edge === "top"
			? { along: offset.x, out: offset.y }
			: { along: offset.y, out: -offset.x };
	}

	/** A box `length` long along the edge and `depth` deep, centred at `center` along it. */
	rect(center: number, length: number, depth: number): GooRect {
		return this.edge === "top"
			? { x: center - length / 2, y: this.at, width: length, height: depth }
			: {
					x: this.at - depth,
					y: center - length / 2,
					width: depth,
					height: length,
				};
	}

	/** The den's body: it reaches `radius` past the edge so its corners there stay hidden. */
	body(center: number, length: number, depth: number, radius: number): GooBlob {
		const middle = this.point(center, (depth - radius) / 2);
		const half = (depth + radius) / 2;
		return this.edge === "top"
			? {
					...middle,
					hw: length / 2,
					hh: half,
					radius,
					anchored: true,
				}
			: {
					...middle,
					hw: half,
					hh: length / 2,
					radius,
					anchored: true,
				};
	}
}

function denLength(den: DenShape): number {
	return den.edge === "top" ? den.rect.width : den.rect.height;
}

function denDepth(den: DenShape): number {
	return den.edge === "top" ? den.rect.height : den.rect.width;
}

/** Lub-dub: two beats a short while apart, then rest. 0..~1. */
function heartbeat(clock: number, phase: number): number {
	const t = (clock + phase) % HEARTBEAT;
	const pulse = (at: number) => Math.exp(-(((t - at) / 0.07) ** 2));
	return pulse(0.05) + 0.6 * pulse(0.27);
}

/** A seeded jitter in [-1, 1], so a landing's spray differs from bot to bot but stays testable. */
function jitter(seed: number): number {
	const value = Math.sin(seed * 12.9898) * 43758.5453;
	return (value - Math.floor(value)) * 2 - 1;
}

export class DenScene {
	private readonly length = new Spring(0, BOUNCY);
	private readonly depth = new Spring(0, BOUNCY);
	/** x: along the edge, y: out of it. */
	private readonly pull = new Spring2(0, 0, BOUNCY);
	/** x: where along the edge the pointer bulges the den, y: how much. */
	private readonly bump = new Spring2(0, 0, SMOOTH);
	private readonly bots = new Map<string, BotSprings>();
	private input: SceneInput | null = null;
	private started = false;
	private clock = 0;
	private landings = 0;

	setInput(input: SceneInput): void {
		const intro = !this.started;
		this.input = input;
		this.started = true;
		this.length.set(denLength(input.den));
		this.depth.set(denDepth(input.den));
		const edge = DenFrame.of(input.den);
		const pulled = input.pull ? edge.split(input.pull) : null;
		this.pull.set(
			pulled
				? stretched(Math.abs(pulled.along), 40) * Math.sign(pulled.along)
				: 0,
			pulled ? stretched(pulled.out, PULL_REACH) : 0,
			pulled ? TIGHT : BOUNCY,
		);
		this.aimBump(input, edge);

		const known = new Set<string>();
		input.den.slots.forEach((slot, index) => {
			known.add(slot.botId);
			this.target(slot.botId, slot, input, intro ? index : null);
		});
		for (const id of this.bots.keys()) if (!known.has(id)) this.bots.delete(id);
	}

	private aimBump(input: SceneInput, edge: DenFrame): void {
		const pointer = input.pointer;
		const den = input.den.rect;
		const near =
			pointer &&
			!input.tucked &&
			pointer.x >= den.x - BUMP_REACH &&
			pointer.x <= den.x + den.width + BUMP_REACH &&
			pointer.y >= den.y - BUMP_REACH &&
			pointer.y <= den.y + den.height + BUMP_REACH;
		if (!pointer || !near) {
			this.bump.y.set(0);
			return;
		}
		const center = edge.along({
			x: den.x + den.width / 2,
			y: den.y + den.height / 2,
		});
		const half = denLength(input.den) / 2 - 12;
		const along = Math.min(
			center + half,
			Math.max(center - half, edge.along(pointer)),
		);
		const away = Math.max(0, edge.out(pointer) - denDepth(input.den));
		if (this.bump.y.value < 0.05) this.bump.x.snap(along);
		this.bump.x.set(along);
		this.bump.y.set(1 - Math.min(1, away / BUMP_REACH) * 0.6);
	}

	/** `introIndex`: its place in the den during the first input, so it pops in after the den. */
	private target(
		botId: string,
		slot: GooPoint,
		input: SceneInput,
		introIndex: number | null,
	): void {
		const place = input.places.get(botId);
		const kind: PlaceKind = place?.kind ?? "home";
		let springs = this.bots.get(botId);
		if (!springs) {
			const intro = introIndex !== null && !place;
			springs = {
				pos: new Spring2(slot.x, slot.y),
				out: new Spring(0),
				morph: new Spring(0),
				fade: new Spring(intro ? 0 : 1),
				impact: new Spring(0, JELLY),
				lift: new Spring(0),
				tether: new Spring(0, TETHER),
				appearAt: intro ? INTRO_DELAY + (introIndex ?? 0) * INTRO_STAGGER : 0,
				splash: [],
				phase: this.bots.size * 0.37,
				place: kind,
			};
			this.bots.set(botId, springs);
		}
		this.react(springs, kind, place);
		springs.morph.set(input.pill?.botId === botId ? 1 : 0, BOUNCY);
		springs.out.set(place ? 1 : 0, BOUNCY);
		springs.lift.set(kind === "drag" ? 1 : 0, BOUNCY);
		if (!place) {
			springs.pos.set(slot.x, slot.y, BOUNCY);
			if (springs.appearAt === 0) springs.fade.set(input.tucked ? 0 : 1);
			return;
		}
		const follow =
			place.kind === "drag" ? TIGHT : place.kind === "perch" ? SMOOTH : BOUNCY;
		springs.pos.set(place.x, place.y, follow);
		springs.fade.set(place.kind === "perch" && !place.visible ? 0 : 1, SMOOTH);
	}

	/**
	 * A bot that leaves the den makes it recoil, one that returns gulps; a landing squashes it and
	 * sprays a few droplets that fall back into it.
	 */
	private react(springs: BotSprings, kind: PlaceKind, place?: BotPlace): void {
		const was = springs.place;
		if (was === kind) return;
		if (was === "home") this.depth.velocity -= DEN_KICK;
		if (kind === "home") this.depth.velocity += DEN_KICK * 0.6;
		if (was === "drag") {
			springs.impact.velocity += LANDING_KICK;
			if (place && kind !== "drag") this.spray(springs, place);
		}
		springs.place = kind;
	}

	private spray(springs: BotSprings, place: { x: number; y: number }): void {
		this.landings += 1;
		const seed = this.landings * 7.31;
		springs.splash = [-1, 0, 1].map((side, index) => ({
			x: place.x + side * 6,
			y: place.y + PERCH_BLOB * 0.3,
			vx: side * 125 + jitter(seed + index) * 30,
			vy: -190 - Math.abs(side - 0.5) * 20 + jitter(seed + index + 3) * 35,
			radius: 3.6 + jitter(seed + index + 5) * 0.7,
			life: 1,
		}));
	}

	/** Whether something animates on its own, without a spring that still moves: busy bots. */
	ambient(): boolean {
		const activity = this.input?.activity;
		return Boolean(activity && activity.size > 0);
	}

	/** A neck holds while its bot is out but close to its slot; further away it snaps back. */
	private aimTethers(): void {
		const input = this.input;
		if (!input) return;
		const edge = DenFrame.of(input.den);
		const depth = this.depth.value;
		for (const slot of input.den.slots) {
			const springs = this.bots.get(slot.botId);
			if (!springs) continue;
			const lip = edge.point(edge.along(slot), depth);
			const reach = Math.hypot(
				springs.pos.x.value - lip.x,
				springs.pos.y.value - lip.y,
			);
			const free =
				springs.out.value > 0.02 &&
				input.places.get(slot.botId)?.kind !== "perch";
			springs.tether.set(free && reach < NECK_REACH ? 1 : 0);
		}
	}

	/** Advances the springs and the clock; true while anything still moves. */
	step(seconds: number): boolean {
		this.clock += Math.min(seconds, 0.1);
		this.aimTethers();
		let moving = this.length.step(seconds);
		moving = this.depth.step(seconds) || moving;
		moving = this.pull.step(seconds) || moving;
		moving = this.bump.step(seconds) || moving;
		for (const springs of this.bots.values()) {
			if (springs.appearAt > 0) {
				moving = true;
				if (this.clock >= springs.appearAt) {
					springs.appearAt = 0;
					springs.fade.set(this.input?.tucked ? 0 : 1, BOUNCY);
					springs.impact.velocity -= 7;
				}
			}
			moving = springs.pos.step(seconds) || moving;
			moving = springs.out.step(seconds) || moving;
			moving = springs.morph.step(seconds) || moving;
			moving = springs.fade.step(seconds) || moving;
			moving = springs.impact.step(seconds) || moving;
			moving = springs.lift.step(seconds) || moving;
			moving = springs.tether.step(seconds) || moving;
			if (springs.splash.length > 0) {
				moving = true;
				const dt = Math.min(seconds, 0.05);
				for (const drop of springs.splash) {
					drop.vy += SPLASH_GRAVITY * dt;
					drop.x += drop.vx * dt;
					drop.y += drop.vy * dt;
					drop.life -= dt / SPLASH_LIFE;
				}
				springs.splash = springs.splash.filter((drop) => drop.life > 0);
			}
		}
		return moving;
	}

	frame(): SceneFrame {
		const input = this.input;
		if (!input)
			return {
				blobs: [],
				anchor: { edge: "top", at: 0 },
				den: { x: 0, y: 0, width: 0, height: 0 },
				bots: new Map(),
			};
		const edge = DenFrame.of(input.den);
		const length = Math.max(0, this.length.value);
		const depth = this.depth.value;
		const center = edge.along({
			x: input.den.rect.x + input.den.rect.width / 2,
			y: input.den.rect.y + input.den.rect.height / 2,
		});
		const radius = Math.min(input.den.radius, depth / 2);
		const shown = length > 0.5 && depth > 0.5;
		const blobs: GooBlob[] = shown
			? [
					edge.body(center, length, depth, radius),
					...this.pullBlobs(edge, center, depth),
					...this.denBlobs(input, edge, center, length, depth),
				]
			: [];
		const nodes = new Map<string, BotNode>();
		for (const slot of input.den.slots) {
			const springs = this.bots.get(slot.botId);
			if (springs)
				nodes.set(
					slot.botId,
					this.botNode(slot.botId, slot, springs, input, edge, depth, blobs),
				);
		}
		return {
			blobs,
			anchor: { edge: edge.edge, at: edge.at },
			den: edge.rect(center, length, depth),
			bots: nodes,
		};
	}

	private pullBlobs(edge: DenFrame, center: number, depth: number): GooBlob[] {
		const drop = this.pull.y.value;
		if (drop < 2) return [];
		const tip = edge.point(center + this.pull.x.value, depth + drop - 10);
		const tipRadius = 12 + Math.min(drop, 60) * 0.03;
		return [
			...neck(
				edge.point(center, depth - 6),
				14,
				tip,
				tipRadius * 0.8,
				Math.max(2.5, 9 - drop * 0.045),
			),
			circle(tip.x, tip.y, tipRadius),
		];
	}

	/** The den's own life: a bulge toward the pointer, and a bead running along it while bots work. */
	private denBlobs(
		input: SceneInput,
		edge: DenFrame,
		center: number,
		length: number,
		depth: number,
	): GooBlob[] {
		const blobs: GooBlob[] = [];
		const bump = this.bump.y.value;
		if (bump > 0.03) {
			const at = edge.point(this.bump.x.value, depth - 4 + 5 * bump);
			blobs.push(circle(at.x, at.y, 8 * bump));
		}
		const busy = [...(input.activity?.values() ?? [])].some(
			(motion) => motion !== "waiting",
		);
		if (busy && !input.tucked) {
			const half = Math.max(0, length / 2 - 16);
			const along = center + half * Math.sin(this.clock * 1.7);
			const at = edge.point(along, depth - 1.5);
			blobs.push(circle(at.x, at.y, 4.6));
		}
		return blobs;
	}

	private botNode(
		botId: string,
		slot: GooPoint,
		springs: BotSprings,
		input: SceneInput,
		edge: DenFrame,
		depth: number,
		blobs: GooBlob[],
	): BotNode {
		const out = springs.out.value;
		const fade = Math.max(0, springs.fade.value);
		const morph = springs.morph.value;
		const impact = springs.impact.value;
		const lift = Math.max(0, springs.lift.value);
		const x = springs.pos.x.value;
		const y = springs.pos.y.value;
		const vx = springs.pos.x.velocity;
		const vy = springs.pos.y.velocity;
		const speed = Math.hypot(vx, vy);
		const place = input.places.get(botId);
		const motion = input.activity?.get(botId);
		const beat =
			motion === "waiting" ? heartbeat(this.clock, springs.phase) : 0;
		const size = lerp(DEN_AVATAR, PERCH_AVATAR, out) * (0.4 + 0.6 * fade);
		const pose = {
			scaleX:
				(1 + SQUASH * 0.6 * impact) * (1 + 0.08 * lift) * (1 + 0.05 * beat),
			scaleY:
				(1 - SQUASH * 0.6 * impact) * (1 + 0.08 * lift) * (1 + 0.05 * beat),
			tilt: clamp(vx * TILT_PER_SPEED, TILT_MAX) * Math.min(1, out),
			lift,
		};
		const slotAlong = edge.along(slot);

		if (input.hoverBot === botId && out < 0.5) {
			const bulge = edge.point(slotAlong, depth - 3);
			blobs.push(circle(bulge.x, bulge.y, 11));
		}
		if (beat > 0.02 && out < 0.5 && !input.tucked) {
			const bulge = edge.point(slotAlong, depth - 2);
			blobs.push(circle(bulge.x, bulge.y, 10 * beat));
		}

		const mouth = edge.point(slotAlong, depth - 6);
		const reach = Math.hypot(x - mouth.x, y - mouth.y - 6);
		const free = out > 0.02 && place?.kind !== "perch";
		const tether = Math.min(1, Math.max(0, springs.tether.value));
		if (free && tether > 0.03) {
			const stretch = Math.min(1, reach / NECK_REACH);
			const body = Math.min(1, out * 1.5) * tether;
			const end = {
				x: lerp(mouth.x, x, tether),
				y: lerp(mouth.y, y, tether),
			};
			blobs.push(
				...neck(
					mouth,
					lerp(12, 7, stretch) * body,
					end,
					PERCH_BLOB * 0.6 * body,
					lerp(8, 1.2, stretch) * body,
				),
			);
		}

		for (const drop of springs.splash)
			blobs.push(
				circle(drop.x, drop.y, drop.radius * Math.min(1, drop.life * 1.6)),
			);

		const radius = PERCH_BLOB * fade * Math.min(1, out * 1.4);
		if (input.pill?.botId === botId && morph > 0.01) {
			const pillWidth = input.pill.width;
			const pillX = Math.min(
				Math.max(x, pillWidth / 2 + 8),
				input.screenWidth - pillWidth / 2 - 8,
			);
			const cx = lerp(x, pillX, morph);
			const cy = y - (input.pill.lift ?? 0) * morph;
			const hw = lerp(Math.max(radius, 1), pillWidth / 2, morph);
			const hh = lerp(Math.max(radius, 1), PILL_HEIGHT / 2, morph);
			blobs.push({ x: cx, y: cy, hw, hh, radius: hh });
			return {
				x: lerp(x, cx - hw + PILL_HEIGHT / 2, morph),
				y: cy,
				size: lerp(size, DEN_AVATAR, morph),
				opacity: 1,
				...pose,
				tilt: pose.tilt * (1 - morph),
				pill:
					morph > 0.6
						? { x: cx - hw, y: cy - hh, width: hw * 2, height: hh * 2 }
						: null,
			};
		}
		if (radius > 0.5) {
			const swell = 1 + 0.16 * beat;
			const hw = radius * (1 + SQUASH * impact) * swell;
			const hh = radius * (1 - SQUASH * impact) * swell;
			const settled = y + (radius - hh);
			blobs.push({ x, y: settled, hw, hh, radius: Math.min(hw, hh) });
			if (out > 0.5 && speed > TAIL_SPEED) {
				const length = Math.min(
					(speed - TAIL_SPEED) * TAIL_PER_SPEED,
					TAIL_MAX,
				);
				blobs.push(
					stroke(
						{ x, y: settled },
						radius * 0.92,
						{
							x: x - (vx / speed) * length,
							y: settled - (vy / speed) * length,
						},
						Math.max(2.5, radius * 0.3),
					),
				);
			}
			if (place?.kind === "perch" && place.edgeY !== null)
				blobs.push({
					x,
					y: place.edgeY + 1.5,
					hw: (15 + 6 * Math.max(0, impact)) * fade,
					hh: 2.5,
					radius: 2.5,
				});
		}
		if (motion === "working" || motion === "thinking") {
			const orbit = (radius > 0.5 ? radius : DEN_AVATAR / 2) + 3;
			const moons =
				motion === "working"
					? [
							{ speed: 3.1, size: 5.2, offset: 0 },
							{ speed: -2.2, size: 4, offset: Math.PI },
						]
					: [{ speed: 1.3, size: 4.4, offset: 0 }];
			const cx = radius > 0.5 ? x : slot.x;
			const cy = radius > 0.5 ? y : slot.y;
			for (const moon of moons) {
				const angle = this.clock * moon.speed + moon.offset + springs.phase * 6;
				blobs.push(
					circle(
						cx + Math.cos(angle) * orbit,
						cy + Math.sin(angle) * orbit,
						moon.size * Math.max(fade, radius > 0.5 ? 0 : 1),
					),
				);
			}
		}
		return {
			x,
			y,
			size,
			opacity: Math.min(1, fade * 1.3),
			...pose,
			pill: null,
		};
	}
}
