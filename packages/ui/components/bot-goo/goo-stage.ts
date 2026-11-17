"use client";

import { useCallback, useEffect, useRef } from "react";
import {
	type GooLook,
	type GooSurface,
	INK_GOO,
	createCssGoo,
	createWebGlGoo,
} from "../../lib/bot-goo/goo-renderer";
import {
	type BotPlace,
	DenScene,
	type GooPoint,
	type GooRect,
	PERCH_AVATAR,
	type SceneFrame,
	type SceneInput,
} from "../../lib/bot-goo/scene";

const HITBOX_MARGIN = 6;
/** Busy bots animate on their own; 60 frames a second keeps their moons smooth and spares a 120 Hz GPU half its work. */
const AMBIENT_FRAME_MS = 16;
const HITBOX_INTERVAL_MS = 33;
const CARD_GAP = 10;

export type StageModel = Omit<SceneInput, "pull">;

/** Told where the pointer may interact, for a window that clicks through everywhere else. */
export type HitboxSink = (rects: GooRect[], capture: boolean) => void;

interface Overrides {
	drag: { botId: string; point: GooPoint } | null;
	/** Where a dropped bot goes (null: home) until the React model says the same. */
	landing: { botId: string; place: BotPlace | null } | null;
	pull: GooPoint | null;
	capture: boolean;
}

function agrees(model: StageModel, landing: Overrides["landing"]): boolean {
	if (!landing) return true;
	const place = model.places.get(landing.botId);
	if (!landing.place) return place === undefined;
	return (
		place?.kind === "perch" &&
		landing.place.kind === "perch" &&
		place.targetId === landing.place.targetId
	);
}

function rounded(rect: GooRect): GooRect {
	return {
		x: Math.floor(rect.x),
		y: Math.floor(rect.y),
		width: Math.ceil(rect.width),
		height: Math.ceil(rect.height),
	};
}

function grown(rect: GooRect, margin: number): GooRect {
	return {
		x: rect.x - margin,
		y: rect.y - margin,
		width: rect.width + margin * 2,
		height: rect.height + margin * 2,
	};
}

function boxOf(node: Element): GooRect | null {
	const box = node.getBoundingClientRect();
	return box.width > 0
		? { x: box.x, y: box.y, width: box.width, height: box.height }
		: null;
}

/**
 * A den's animation loop: steps the scene while it moves, draws the goo, and places the DOM nodes
 * registered as `den`, `bot:<id>`, `pill:<id>` and `card:<id>`. With a hitbox sink it also reports
 * where the pointer may interact (`overlay:` nodes included), so everything else clicks through.
 */
export class GooStage {
	private readonly scene = new DenScene();
	private readonly nodes = new Map<string, HTMLElement>();
	private surface: GooSurface | null = null;
	private look: GooLook = INK_GOO;
	private model: StageModel | null = null;
	private overrides: Overrides = {
		drag: null,
		landing: null,
		pull: null,
		capture: false,
	};
	private frameId = 0;
	private last = 0;
	/** Something changed that the next frame must draw, though no spring moves. */
	private dirty = true;
	private drawnAt = 0;
	private lastFrame: SceneFrame | null = null;
	private sentHitboxes = "";
	private hitboxTimer: number | undefined;
	private hitboxesAt = 0;
	/** Runs before each frame; true keeps the loop going, e.g. while perches follow moving targets. */
	beforeFrame: (() => boolean) | null = null;

	constructor(private readonly hitboxSink: HitboxSink | null = null) {}

	attach(surface: GooSurface | null): void {
		this.surface?.dispose();
		this.surface = surface;
		this.kick();
	}

	resize(width: number, height: number): void {
		this.surface?.resize(width, height, window.devicePixelRatio || 1);
		this.kick();
	}

	setLook(look: GooLook): void {
		if (look === this.look) return;
		this.look = look;
		this.dirty = true;
		this.kick();
	}

	register(id: string, node: HTMLElement | null): void {
		if (node) this.nodes.set(id, node);
		else this.nodes.delete(id);
		this.kick();
	}

	setModel(model: StageModel): void {
		this.model = model;
		if (agrees(model, this.overrides.landing)) this.overrides.landing = null;
		this.apply();
	}

	setDrag(drag: Overrides["drag"]): void {
		this.overrides.drag = drag;
		this.apply();
	}

	/** Ends a drag: the bot heads for `place` (null: its den slot) right away. */
	land(botId: string, place: BotPlace | null): void {
		this.overrides.drag = null;
		const landing = { botId, place };
		this.overrides.landing =
			this.model && agrees(this.model, landing) ? null : landing;
		this.apply();
	}

	setPull(pull: GooPoint | null): void {
		this.overrides.pull = pull;
		this.apply();
	}

	/** The whole window takes the pointer, for drags and pulls that leave every shape. */
	setCapture(capture: boolean): void {
		this.overrides.capture = capture;
		this.sendHitboxes(true);
	}

	frame(): SceneFrame | null {
		return this.lastFrame;
	}

	node(id: string): HTMLElement | undefined {
		return this.nodes.get(id);
	}

	/** Places a node again after React changed its content, without waiting for motion. */
	kick(): void {
		this.dirty = true;
		if (this.frameId || typeof window === "undefined") return;
		this.last = performance.now();
		this.frameId = requestAnimationFrame(this.tick);
	}

	stop(): void {
		if (this.frameId) cancelAnimationFrame(this.frameId);
		this.frameId = 0;
		window.clearTimeout(this.hitboxTimer);
	}

	private apply(): void {
		if (!this.model) return;
		const places = new Map<string, BotPlace>(this.model.places);
		const landing = this.overrides.landing;
		if (landing?.place) places.set(landing.botId, landing.place);
		else if (landing) places.delete(landing.botId);
		const drag = this.overrides.drag;
		if (drag) places.set(drag.botId, { kind: "drag", ...drag.point });
		this.scene.setInput({ ...this.model, places, pull: this.overrides.pull });
		this.kick();
	}

	private readonly tick = (now: number) => {
		const seconds = Math.max(0, (now - this.last) / 1000);
		this.last = now;
		const following = this.beforeFrame?.() ?? false;
		const moving = this.scene.step(seconds);
		const ambient = this.scene.ambient();
		if (
			moving ||
			this.dirty ||
			(ambient && now - this.drawnAt >= AMBIENT_FRAME_MS)
		) {
			this.dirty = false;
			this.drawnAt = now;
			const frame = this.scene.frame();
			this.lastFrame = frame;
			this.surface?.draw(frame.blobs, frame.anchor, this.look);
			this.place(frame);
		}
		this.sendHitboxes(false);
		this.frameId =
			moving || following || ambient ? requestAnimationFrame(this.tick) : 0;
	};

	private place(frame: SceneFrame): void {
		const den = this.nodes.get("den");
		if (den) {
			den.style.transform = `translate3d(${frame.den.x}px, ${frame.den.y}px, 0)`;
			den.style.width = `${frame.den.width}px`;
			den.style.height = `${frame.den.height}px`;
		}
		const screenWidth = this.model?.screenWidth ?? window.innerWidth;
		for (const [botId, bot] of frame.bots) {
			const avatar = this.nodes.get(`bot:${botId}`);
			if (avatar) {
				const half = PERCH_AVATAR / 2;
				const base = bot.size / PERCH_AVATAR;
				avatar.style.transform = `translate3d(${bot.x - half}px, ${bot.y - half}px, 0) rotate(${bot.tilt}deg) scale(${base * bot.scaleX}, ${base * bot.scaleY})`;
				avatar.style.opacity = String(bot.opacity);
				avatar.style.pointerEvents = bot.opacity < 0.2 ? "none" : "auto";
				avatar.style.setProperty("--lift", bot.lift.toFixed(3));
			}
			const pill = this.nodes.get(`pill:${botId}`);
			if (pill) {
				pill.style.opacity = bot.pill ? "1" : "0";
				pill.style.pointerEvents = bot.pill ? "auto" : "none";
				if (bot.pill) {
					pill.style.transform = `translate3d(${bot.pill.x}px, ${bot.pill.y}px, 0)`;
					pill.style.width = `${bot.pill.width}px`;
					pill.style.height = `${bot.pill.height}px`;
				}
			}
			const card = this.nodes.get(`card:${botId}`);
			if (card) {
				const width = card.offsetWidth;
				const below = bot.pill
					? bot.pill.y + bot.pill.height
					: bot.y + bot.size / 2;
				const x = Math.min(
					Math.max(bot.x - width / 2, 8),
					screenWidth - width - 8,
				);
				card.style.transform = `translate3d(${x}px, ${below + CARD_GAP}px, 0)`;
				card.style.opacity = String(bot.opacity);
			}
		}
	}

	private hitboxes(): GooRect[] {
		const frame = this.lastFrame;
		if (!frame) return [];
		const rects: GooRect[] = [];
		if (frame.den.width > 0) rects.push(grown(frame.den, HITBOX_MARGIN));
		for (const [botId, bot] of frame.bots) {
			if (bot.opacity < 0.2) continue;
			const half = bot.size / 2 + HITBOX_MARGIN;
			rects.push({
				x: bot.x - half,
				y: bot.y - half,
				width: half * 2,
				height: half * 2,
			});
			if (bot.pill) rects.push(grown(bot.pill, 2));
			const card = this.nodes.get(`card:${botId}`);
			const box = card ? boxOf(card) : null;
			if (box) rects.push(box);
		}
		for (const [id, node] of this.nodes) {
			if (!id.startsWith("overlay:")) continue;
			const box = boxOf(node);
			if (box) rects.push(box);
		}
		return rects.map(rounded);
	}

	private sendHitboxes(force: boolean): void {
		const sink = this.hitboxSink;
		if (!sink) return;
		const now = performance.now();
		if (!force && now - this.hitboxesAt < HITBOX_INTERVAL_MS) {
			window.clearTimeout(this.hitboxTimer);
			this.hitboxTimer = window.setTimeout(
				() => this.sendHitboxes(true),
				HITBOX_INTERVAL_MS,
			);
			return;
		}
		const rects = this.hitboxes();
		const capture = this.overrides.capture;
		const key = `${capture}|${JSON.stringify(rects)}`;
		if (key === this.sentHitboxes) return;
		this.sentHitboxes = key;
		this.hitboxesAt = now;
		sink(rects, capture);
	}
}

/** One stage per den, with its canvas covering the window. */
export function useGooStage(hitboxSink: HitboxSink | null = null): {
	stage: GooStage;
	canvasRef: (canvas: HTMLCanvasElement | null) => void;
	nodeRef: (id: string) => (node: HTMLElement | null) => void;
} {
	const stage = useRef<GooStage | null>(null);
	if (!stage.current) stage.current = new GooStage(hitboxSink);
	const refs = useRef(new Map<string, (node: HTMLElement | null) => void>());
	const unwatchCanvas = useRef<(() => void) | null>(null);

	const canvasRef = useCallback((canvas: HTMLCanvasElement | null) => {
		const current = stage.current;
		unwatchCanvas.current?.();
		unwatchCanvas.current = null;
		if (!current) return;
		if (!canvas) {
			current.attach(null);
			return;
		}
		const host = canvas.parentElement;
		const plain = () => (host ? createCssGoo(host) : null);
		const connect = () => {
			current.attach(createWebGlGoo(canvas) ?? plain());
			current.resize(window.innerWidth, window.innerHeight);
		};
		// The GPU process can drop the context (sleep, driver reset, too many contexts): plain
		// shapes stand in until the browser restores it.
		const lost = (event: Event) => {
			event.preventDefault();
			current.attach(plain());
		};
		canvas.addEventListener("webglcontextlost", lost);
		canvas.addEventListener("webglcontextrestored", connect);
		unwatchCanvas.current = () => {
			canvas.removeEventListener("webglcontextlost", lost);
			canvas.removeEventListener("webglcontextrestored", connect);
		};
		connect();
	}, []);

	const nodeRef = useCallback((id: string) => {
		let ref = refs.current.get(id);
		if (!ref) {
			ref = (node: HTMLElement | null) => stage.current?.register(id, node);
			refs.current.set(id, ref);
		}
		return ref;
	}, []);

	useEffect(() => {
		const current = stage.current;
		const onResize = () =>
			current?.resize(window.innerWidth, window.innerHeight);
		// Moving the window to a display with another scale fires no resize.
		let scale: MediaQueryList | null = null;
		const watchScale = () => {
			scale?.removeEventListener("change", onScale);
			scale = window.matchMedia(
				`(resolution: ${window.devicePixelRatio || 1}dppx)`,
			);
			scale.addEventListener("change", onScale);
		};
		const onScale = () => {
			onResize();
			watchScale();
		};
		watchScale();
		window.addEventListener("resize", onResize);
		return () => {
			window.removeEventListener("resize", onResize);
			scale?.removeEventListener("change", onScale);
			current?.stop();
		};
	}, []);

	return { stage: stage.current, canvasRef, nodeRef };
}
