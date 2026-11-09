import { useEffect } from "react";
import type { BotFrame } from "./page-bot-scene";
import "./page-bot-helpers.css";

/** The crew from the bots showcase: Flux, Byte, Post, Scout and Loop. */
const helpers = {
	hero: { kind: "flux", tool: "wrench" },
	apps: { kind: "chip", tool: "cable" },
	platform: { kind: "mochi", tool: "driver" },
	control: { kind: "pip", tool: "inspect" },
	book: { kind: "orbi", tool: "pencil" },
} as const satisfies Record<string, Pick<BotFrame, "kind" | "tool">>;

type HelperName = keyof typeof helpers;
interface Anchor {
	id: string;
	element: HTMLElement;
	canvas: HTMLCanvasElement;
	context: CanvasRenderingContext2D;
	rail: HTMLElement;
	scrollScope: HTMLElement | null;
	name: HelperName;
	x: number;
	xVelocity: number;
	targetX: number;
	minX: number;
	maxX: number;
	ground: number;
	markerX: number;
	width: number;
	height: number;
	visible: boolean;
	walk: number;
	work: number;
	reach: number;
	lookX: number;
	lookY: number;
	lean: number;
}
interface BotScene {
	render(frames: BotFrame[], width: number, height: number, dpr: number): void;
	dispose(): void;
}

const clamp = (value: number, min: number, max: number) =>
	Math.max(min, Math.min(max, value));
const damp = (current: number, target: number, speed: number, dt: number) =>
	current + (target - current) * (1 - Math.exp(-speed * dt));

/** DOM sprites follow their borders; one detached WebGL canvas supplies their poses. */
export default function PageBotHelpers() {
	useEffect(() => {
		const anchors = Array.from(
			document.querySelectorAll<HTMLElement>("[data-bot-helper]"),
		).flatMap((element, index): Anchor[] => {
			const name = element.dataset.botHelper;
			if (!name || !Object.hasOwn(helpers, name) || !element.parentElement)
				return [];
			let rail = element.parentElement;
			let scrollScope: HTMLElement | null = null;
			if (element.dataset.botRail) {
				try {
					rail = element.closest<HTMLElement>(element.dataset.botRail) ?? rail;
				} catch {
					// An invalid optional selector falls back to the positioning parent.
				}
			}
			if (element.dataset.botScrollScope) {
				try {
					scrollScope = element.closest<HTMLElement>(
						element.dataset.botScrollScope,
					);
				} catch {
					// A malformed optional scope uses the marker's viewport progress.
				}
			}
			const canvas = document.createElement("canvas");
			const context = canvas.getContext("2d", { alpha: true });
			if (!context) return [];
			canvas.className = "page-bot-sprite";
			canvas.setAttribute("aria-hidden", "true");
			canvas.hidden = true;
			element.append(canvas);
			return [
				{
					id: `${name}-${index}`,
					element,
					canvas,
					context,
					rail,
					scrollScope,
					name: name as HelperName,
					x: Number.NaN,
					xVelocity: 0,
					targetX: 0,
					minX: 0,
					maxX: 0,
					ground: 0,
					markerX: 0,
					width: 120,
					height: 150,
					visible: false,
					walk: 0,
					work: 0,
					reach: 0,
					lookX: 0,
					lookY: 0,
					lean: 0,
				},
			];
		});
		if (!anchors.length) return;
		const makeCanvas = () => {
			const element = document.createElement("canvas");
			return element;
		};
		// An effect always owns a new canvas; disposed WebGL contexts cannot survive HMR.
		let canvas = makeCanvas();

		const media = window.matchMedia("(prefers-reduced-motion: reduce)");
		let scene: BotScene | undefined;
		let loading = false;
		let failed = false;
		let disposed = false;
		let frame = 0;
		let dirty = true;
		let width = window.innerWidth;
		let height = window.innerHeight;
		let dpr = Math.min(window.devicePixelRatio || 1, 1.5);
		let paused = document.documentElement.dataset.botMotionPaused === "true";
		let lastDraw = 0;
		let lastTick = 0;
		let elapsed = 0;
		let lastScrollY = window.scrollY;
		let lastScrollAt = performance.now() - 1000;
		let scrollVelocity = 0;
		let smoothScrollVelocity = 0;
		let pointer: { x: number; y: number } | undefined;
		let measureUntil = 0;
		let contextLost = false;
		let retries = 0;
		let retryTimer = 0;
		const warn = (stage: string, error: unknown) => {
			if (import.meta.env.DEV) console.warn(`[Page bots] ${stage}`, error);
		};

		const animated = () => !paused && !media.matches && !document.hidden;
		const measure = () => {
			dirty = false;
			width = window.innerWidth;
			height = window.innerHeight;
			dpr = Math.min(window.devicePixelRatio || 1, 1.5);
			for (const anchor of anchors) {
				const bounds = anchor.element.getBoundingClientRect();
				const rail = anchor.rail.getBoundingClientRect();
				anchor.width = bounds.width;
				anchor.height = bounds.height * 1.39;
				anchor.ground = bounds.top + bounds.height * 0.8;
				anchor.markerX = bounds.left + bounds.width / 2;
				const botTop = anchor.ground - anchor.height * 0.82;
				anchor.visible =
					bounds.width > 0 &&
					bounds.height > 0 &&
					botTop < height &&
					botTop + anchor.height > 80 &&
					rail.right > 0 &&
					rail.left < width;
				if (!anchor.visible) continue;
				const pixelWidth = Math.max(1, Math.round(anchor.width * dpr));
				const pixelHeight = Math.max(1, Math.round(anchor.height * dpr));
				if (anchor.canvas.width !== pixelWidth)
					anchor.canvas.width = pixelWidth;
				if (anchor.canvas.height !== pixelHeight)
					anchor.canvas.height = pixelHeight;

				// Keep the short walking rail on the empty border beside the marker.
				const half = anchor.width / 2;
				const viewportMin = half + 6;
				const viewportMax = width - half - 6;
				const min = Math.max(viewportMin, rail.left + half + 4);
				const max = Math.min(viewportMax, rail.right - half - 4);
				anchor.minX =
					min <= max
						? min
						: clamp(rail.left + rail.width / 2, viewportMin, viewportMax);
				anchor.maxX = min <= max ? max : anchor.minX;
				const home = clamp(bounds.left + half, anchor.minX, anchor.maxX);
				const travel = clamp(
					Number(anchor.element.dataset.botTravel ?? 92) || 0,
					0,
					width < 800 ? 48 : 120,
				);
				const rightSide = home > rail.left + rail.width / 2;
				const away = clamp(
					home + (rightSide ? -travel : travel),
					anchor.minX,
					anchor.maxX,
				);
				const scope = anchor.scrollScope?.getBoundingClientRect();
				const progress = clamp(
					scope
						? (height - scope.top) / Math.max(1, scope.height + height)
						: (height - anchor.ground) / Math.max(1, height - 80),
					0,
					1,
				);
				anchor.targetX = away + (home - away) * progress;
				if (!Number.isFinite(anchor.x)) {
					anchor.x = anchor.targetX;
					anchor.xVelocity = 0;
				} else {
					const bounded = clamp(anchor.x, anchor.minX, anchor.maxX);
					if (bounded !== anchor.x || !animated()) anchor.xVelocity = 0;
					anchor.x = bounded;
				}
			}
		};

		const requestFrame = () => {
			if (!frame && !disposed && !failed && !contextLost && !document.hidden)
				frame = requestAnimationFrame(tick);
		};
		const disposeScene = () => {
			const previous = scene;
			scene = undefined;
			try {
				previous?.dispose();
			} catch (error) {
				warn("Renderer cleanup failed", error);
			}
		};
		const detachCanvas = () => {
			canvas.removeEventListener("webglcontextlost", lostContext);
			canvas.removeEventListener("webglcontextrestored", restoredContext);
			disposeScene();
			canvas.remove();
		};
		const restart = () => {
			if (disposed) return;
			detachCanvas();
			canvas = makeCanvas();
			canvas.addEventListener("webglcontextlost", lostContext);
			canvas.addEventListener("webglcontextrestored", restoredContext);
			failed = false;
			contextLost = false;
			lastTick = 0;
			dirty = true;
			requestFrame();
		};
		const retry = (stage: string, error: unknown) => {
			warn(stage, error);
			failed = true;
			for (const anchor of anchors) anchor.canvas.hidden = true;
			cancelAnimationFrame(frame);
			frame = 0;
			window.clearTimeout(retryTimer);
			if (retries >= 2) {
				detachCanvas();
				return;
			}
			retries += 1;
			retryTimer = window.setTimeout(restart, retries * 1000);
		};
		const startScene = async () => {
			if (scene || loading || disposed || failed) return;
			loading = true;
			try {
				const { createPageBotScene } = await import("./page-bot-scene");
				if (disposed) return;
				scene = createPageBotScene(canvas);
				requestFrame();
			} catch (error) {
				if (!disposed) retry("Renderer initialization failed", error);
			} finally {
				loading = false;
			}
		};

		function tick(now: number) {
			frame = 0;
			if (disposed || failed || contextLost || document.hidden) return;
			if (dirty || now < measureUntil) measure();
			const visible = anchors.filter((anchor) => anchor.visible);
			if (!visible.length) {
				lastTick = 0;
				return;
			}
			if (!scene) {
				void startScene();
				return;
			}
			const moving = animated();
			const frameDuration = 1000 / 30;
			if (moving && now - lastDraw < frameDuration - 0.5) {
				requestFrame();
				return;
			}
			const dt = lastTick ? Math.min((now - lastTick) / 1000, 0.05) : 1 / 30;
			lastTick = moving ? now : 0;
			// Preserve the deadline phase so fractional RAF timestamps do not drop us to 20 fps.
			lastDraw = moving
				? lastDraw +
					Math.max(1, Math.round((now - lastDraw) / frameDuration)) *
						frameDuration
				: now;
			if (moving) elapsed += dt;
			const settling = Math.max(0, (now - lastScrollAt - 180) / 1000);
			const currentVelocity = settling > 0 ? 0 : scrollVelocity;
			smoothScrollVelocity = moving
				? damp(smoothScrollVelocity, currentVelocity, 7, dt)
				: 0;
			let atlasHeight = 0;
			const atlasWidth = Math.max(...visible.map((anchor) => anchor.width));
			const frames: BotFrame[] = visible.map((anchor) => {
				if (moving) {
					const delta = anchor.targetX - anchor.x;
					anchor.xVelocity += (delta * 110 - anchor.xVelocity * 21) * dt;
					const nextX = anchor.x + anchor.xVelocity * dt;
					anchor.x = clamp(nextX, anchor.minX, anchor.maxX);
					if (anchor.x !== nextX) anchor.xVelocity = 0;
					anchor.walk += Math.abs(anchor.xVelocity) * dt * 0.16;
					const walking =
						Math.abs(anchor.xVelocity) > 2 || Math.abs(delta) > 0.5;
					const ready = !walking && settling > 0;
					anchor.reach = damp(anchor.reach, ready ? 1 : 0, ready ? 3.8 : 9, dt);
					anchor.work = damp(
						anchor.work,
						ready && anchor.reach > 0.7 ? 1 : 0,
						ready ? 3 : 10,
						dt,
					);
					anchor.lean = damp(
						anchor.lean,
						clamp(smoothScrollVelocity / 2200 + anchor.xVelocity / 170, -1, 1),
						7,
						dt,
					);
					const centerY = anchor.ground - anchor.height * 0.58;
					const near =
						pointer &&
						Math.hypot(pointer.x - anchor.x, pointer.y - centerY) < 230;
					anchor.lookX = damp(
						anchor.lookX,
						near && pointer ? clamp((pointer.x - anchor.x) / 130, -1, 1) : 0.2,
						5,
						dt,
					);
					anchor.lookY = damp(
						anchor.lookY,
						near && pointer ? clamp((centerY - pointer.y) / 120, -1, 1) : -0.25,
						5,
						dt,
					);
				}
				anchor.canvas.style.setProperty(
					"--bot-offset",
					`${anchor.x - anchor.markerX}px`,
				);
				const helper = helpers[anchor.name];
				const atlasY = atlasHeight;
				atlasHeight += anchor.height;
				return {
					id: anchor.id,
					x: 0,
					y: atlasY,
					width: anchor.width,
					height: anchor.height,
					kind: helper.kind,
					tool: helper.tool,
					walk: anchor.walk,
					work: anchor.work,
					reach: anchor.reach,
					lookX: anchor.lookX,
					lookY: anchor.lookY,
					lean: anchor.lean,
					time: elapsed,
				};
			});
			try {
				scene.render(frames, atlasWidth, atlasHeight, dpr);
				// Read the shared GPU atlas before yielding; DOM canvases carry scrolling themselves.
				for (let index = 0; index < visible.length; index++) {
					const anchor = visible[index];
					const bot = frames[index];
					anchor.context.clearRect(
						0,
						0,
						anchor.canvas.width,
						anchor.canvas.height,
					);
					anchor.context.drawImage(
						canvas,
						0,
						bot.y * dpr,
						bot.width * dpr,
						bot.height * dpr,
						0,
						0,
						anchor.canvas.width,
						anchor.canvas.height,
					);
					anchor.canvas.hidden = false;
				}
			} catch (error) {
				retry("Rendering failed", error);
				return;
			}
			if (moving) requestFrame();
		}

		const invalidate = () => {
			dirty = true;
			requestFrame();
		};
		const scroll = () => {
			const now = performance.now();
			const delta = window.scrollY - lastScrollY;
			const duration = clamp((now - lastScrollAt) / 1000, 1 / 120, 0.1);
			if (delta) {
				scrollVelocity = clamp(delta / duration, -3500, 3500);
				lastScrollY = window.scrollY;
				lastScrollAt = now;
			}
			invalidate();
		};
		const motionChange = (event?: Event) => {
			if (event?.type === "flowlike:bot-motion")
				paused =
					(event as CustomEvent<{ paused: boolean }>).detail?.paused === true;
			cancelAnimationFrame(frame);
			frame = 0;
			lastTick = 0;
			invalidate();
		};
		const point = (event: PointerEvent) => {
			if (event.pointerType === "touch") return;
			pointer = { x: event.clientX, y: event.clientY };
		};
		const leave = () => {
			pointer = undefined;
		};
		const railAnimation = (event: Event) => {
			const target = event.target;
			if (
				!(target instanceof HTMLElement) ||
				!anchors.some(
					(anchor) => target === anchor.rail || target.contains(anchor.element),
				)
			)
				return;
			measureUntil =
				performance.now() + (event.type.endsWith("start") ? 750 : 50);
			invalidate();
		};
		const lostContext = (event: Event) => {
			event.preventDefault();
			if (failed || disposed) return;
			contextLost = true;
			cancelAnimationFrame(frame);
			frame = 0;
			for (const anchor of anchors) anchor.canvas.hidden = true;
			warn("WebGL context lost; waiting for restoration", event);
			window.clearTimeout(retryTimer);
			retryTimer = window.setTimeout(() => {
				if (contextLost && !disposed)
					retry("WebGL context did not restore", event);
			}, 2500);
		};
		const restoredContext = () => {
			if (disposed || failed) return;
			window.clearTimeout(retryTimer);
			contextLost = false;
			lastTick = 0;
			dirty = true;
			requestFrame();
		};
		const observer = new IntersectionObserver(invalidate, {
			rootMargin: "100px",
		});
		const resizeObserver = new ResizeObserver(invalidate);
		const mutations = new MutationObserver(() => {
			measureUntil = performance.now() + 750;
			invalidate();
		});
		for (const anchor of anchors) {
			observer.observe(anchor.element);
			resizeObserver.observe(anchor.rail);
			if (anchor.scrollScope) resizeObserver.observe(anchor.scrollScope);
			mutations.observe(anchor.rail, {
				attributes: true,
				attributeFilter: ["class", "style", "hidden"],
			});
		}
		canvas.addEventListener("webglcontextlost", lostContext);
		canvas.addEventListener("webglcontextrestored", restoredContext);
		window.addEventListener("scroll", scroll, { passive: true });
		window.addEventListener("resize", invalidate);
		window.addEventListener("pageshow", invalidate);
		window.addEventListener("pointermove", point, { passive: true });
		document.addEventListener("pointerleave", leave);
		document.addEventListener("animationstart", railAnimation, true);
		document.addEventListener("animationend", railAnimation, true);
		document.addEventListener("transitionend", railAnimation, true);
		document.addEventListener("visibilitychange", motionChange);
		window.addEventListener("flowlike:bot-motion", motionChange);
		media.addEventListener("change", motionChange);
		invalidate();
		return () => {
			disposed = true;
			cancelAnimationFrame(frame);
			window.clearTimeout(retryTimer);
			observer.disconnect();
			resizeObserver.disconnect();
			mutations.disconnect();
			window.removeEventListener("scroll", scroll);
			window.removeEventListener("resize", invalidate);
			window.removeEventListener("pageshow", invalidate);
			window.removeEventListener("pointermove", point);
			document.removeEventListener("pointerleave", leave);
			document.removeEventListener("animationstart", railAnimation, true);
			document.removeEventListener("animationend", railAnimation, true);
			document.removeEventListener("transitionend", railAnimation, true);
			document.removeEventListener("visibilitychange", motionChange);
			window.removeEventListener("flowlike:bot-motion", motionChange);
			media.removeEventListener("change", motionChange);
			detachCanvas();
			for (const anchor of anchors) anchor.canvas.remove();
		};
	}, []);

	return null;
}
