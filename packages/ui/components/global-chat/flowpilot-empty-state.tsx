"use client";

import type { LucideIcon } from "lucide-react";
import {
	useCallback,
	useEffect,
	useLayoutEffect,
	useMemo,
	useRef,
	useState,
} from "react";
import {
	type IComposerActivityChannel,
	applyComposerLean,
	createTypingResponse,
	decayPerk,
} from "../../lib/composer-activity";
import { observeResize } from "../../lib/observe-resize";
import { cn } from "../../lib/utils";
import { type BubbleFilmUniforms, createBubbleFilm } from "./bubble-film";
import {
	ORB_COMPOSING_PARAMS,
	ORB_INVITING_PARAMS,
	type OrbStateParams,
	mixOrbParams,
} from "./flowpilot-orb-state";

export interface IEmptyStateSuggestion {
	readonly label: string;
	readonly icon: LucideIcon;
	readonly prompt: string;
}

interface IFlowPilotEmptyStateProps {
	readonly suggestions: readonly IEmptyStateSuggestion[];
	readonly onSelect: (prompt: string) => void;
	/** Largest film diameter to use. The mark shrinks below this to fit its container. */
	readonly maxSize?: number;
	/**
	 * Drop the film and show only the suggestions. In a narrow dock the orb crowds the composer
	 * instead of framing it, so the affordances carry the empty state on their own.
	 */
	readonly suggestionsOnly?: boolean;
	/** Play the hand-off to the live orb, then unmount. Drive with `useEmptyStateExit`. */
	readonly exiting?: boolean;
	readonly className?: string;
	/** The composer's draft channel. Without it the mark has nothing to answer. */
	readonly activity?: IComposerActivityChannel;
	/** `placeholder_typing_motion` from the chat interface's config. */
	readonly typingMotion?: boolean;
}

/** Satellite orbit relative to the film's own radius, matching the launcher's proportions. */
const SAT_ORBIT_RATIO = 0.63 / 0.48;
/** The launcher's film-to-canvas ratio, used as the canvas' lower bound. */
const FILM_BOX = 0.48;
const SAT_COUNT = 3;
/** Gap between the film and the suggestion row — `mt-7`. */
const ROW_GAP = 28;
/** Below this the mark reads as an icon rather than a mark, so it stops shrinking. */
const MIN_FILM = 84;

// Entrance choreography, ms from mount. The film inflates, buds three satellites off its own rim,
// and flies one to each suggestion — so the affordances arrive as part of the mark, not beside it.
const SAT_RELEASE = 420;
const SAT_EMERGE = 420;
const DOCK_START = 900;
const DOCK_DURATION = 700;
const DOCK_STAGGER = 130;
const COLLAPSE_DURATION = 260;
/** A suggestion materializes just before its satellite finishes collapsing into it. */
const REVEAL_LEAD = 180;
/** With no satellites to wait for, the suggestions just stagger in on their own. */
const BARE_REVEAL_STAGGER = 70;

/** The swell that greets the start of a burst of typing — a breath, not the entrance's shockwave. */
const PERK_SWELL = 0.05;

const clamp01 = (value: number) => Math.min(1, Math.max(0, value));
const smoothstep = (value: number) => value * value * (3 - 2 * value);

/**
 * Frame-rate independent damping. The per-frame constants this replaces ran at double speed on a
 * 120Hz display and stuttered whenever a frame was dropped.
 */
const approach = (current: number, target: number, rate: number, dt: number) =>
	current + (target - current) * (1 - Math.exp(-rate * dt));

/**
 * Poses the satellites for `ms` into the entrance: budding out of the rim as their orbit opens
 * from nothing, flying to their suggestion, then collapsing into it.
 */
function poseSatellites(
	ms: number,
	orbit: number,
	u: BubbleFilmUniforms,
	gone: boolean,
) {
	const emerge = smoothstep(clamp01((ms - SAT_RELEASE) / SAT_EMERGE));
	u.sat = gone ? 0 : 1;
	u.satOrbit = Math.max(1e-3, orbit * emerge);
	for (let i = 0; i < SAT_COUNT; i++) {
		const start = DOCK_START + i * DOCK_STAGGER;
		u.satDock[i] = smoothstep(clamp01((ms - start) / DOCK_DURATION));
		u.satShrink[i] = smoothstep(
			clamp01((ms - (start + DOCK_DURATION)) / COLLAPSE_DURATION),
		);
	}
}

/** Scratch for the bulge aim, reused so the render loop never allocates. */
const bulge = { x: 0, y: 0, near: 0, attention: 0 };

/**
 * Where the film should bulge: an idle wander around the rim that the pointer takes over as it
 * approaches, and that a live draft takes over from both. `pointer` is null until the pointer has
 * moved, and under reduced motion.
 */
function aimBulge(
	clock: number,
	box: number,
	pointer: { x: number; y: number } | null,
	attention: number,
	dt: number,
	params: OrbStateParams,
	typing: number,
) {
	// Reach is tuned against the launcher's film radius, so it scales with this one.
	const scale = box / FILM_BOX;
	bulge.x = Math.cos(clock * 0.9) * params.reach * scale;
	bulge.y = Math.sin(clock * 1.15) * params.reach * 0.8 * scale;
	bulge.near = 0;
	let pull = 0;
	if (pointer) {
		const distance = Math.hypot(pointer.x, pointer.y) || 1e-4;
		bulge.near = clamp01(1 - distance / Math.max(box * 4, 1e-4));
		// Clamped onto the film so a pointer far across the page still reads as a bulge on the
		// rim facing it rather than a bulge nowhere near the orb.
		const aim = Math.min(1, (box * 0.85) / distance);
		pull = 0.35 + 0.5 * bulge.near;
		bulge.x = bulge.x * (1 - pull) + pointer.x * aim * pull;
		bulge.y = bulge.y * (1 - pull) + pointer.y * aim * pull;
	}
	if (typing > 0) {
		const lean = applyComposerLean(bulge.x, bulge.y, typing, clock, box);
		bulge.x = lean.x;
		bulge.y = lean.y;
		bulge.near = Math.max(bulge.near, typing * 0.6);
		pull = Math.max(pull, typing);
	}
	bulge.attention = approach(attention, pull, 5, dt);
	return bulge;
}

function revealDelay(index: number, suggestionsOnly: boolean) {
	if (suggestionsOnly) return index * BARE_REVEAL_STAGGER;
	return Math.max(
		0,
		DOCK_START + index * DOCK_STAGGER + DOCK_DURATION - REVEAL_LEAD,
	);
}

/**
 * Keeps the empty state mounted through its exit animation. `show` is the real condition;
 * `mounted` is what you render on, and `exiting` is what you hand the component.
 */
export function useEmptyStateExit(show: boolean, durationMs = 420) {
	const [mounted, setMounted] = useState(show);

	useEffect(() => {
		if (show) {
			setMounted(true);
			return;
		}
		const timer = setTimeout(() => setMounted(false), durationMs);
		return () => clearTimeout(timer);
	}, [show, durationMs]);

	return { mounted, exiting: mounted && !show };
}

/**
 * The FlowPilot empty state: the same soap-film orb that runs in the launcher and follows the
 * assistant for the rest of the conversation, rather than a mark you never see again. It
 * inflates on mount, hands its three satellites to the suggestion buttons, rests almost still
 * while leaning toward the pointer, and collapses into the live orb when the first message goes.
 *
 * With `typingMotion` on it also answers the composer: it perks up as a burst of writing starts,
 * gathers and leans down at the draft while you keep going, and settles back the moment you stop.
 * See `ORB_COMPOSING_PARAMS`.
 */
export function FlowPilotEmptyState({
	suggestions,
	onSelect,
	maxSize = 168,
	suggestionsOnly = false,
	exiting = false,
	className,
	activity,
	typingMotion = false,
}: IFlowPilotEmptyStateProps) {
	const rootRef = useRef<HTMLDivElement>(null);
	const canvasRef = useRef<HTMLCanvasElement>(null);
	const rowRef = useRef<HTMLDivElement>(null);
	const buttonRefs = useRef<(HTMLButtonElement | null)[]>([]);
	const geomRef = useRef({
		box: FILM_BOX,
		satOrbit: FILM_BOX * SAT_ORBIT_RATIO,
		satPos: new Float32Array(SAT_COUNT * 2),
	});

	const [layout, setLayout] = useState(() => ({
		film: maxSize,
		side: Math.ceil(maxSize / FILM_BOX),
	}));
	const [failed, setFailed] = useState(false);
	const [revealed, setRevealed] = useState(false);
	const [instant, setInstant] = useState(false);

	const layoutRef = useRef(layout);
	const maxSizeRef = useRef(maxSize);
	const exitingRef = useRef(exiting);
	const suggestionsOnlyRef = useRef(suggestionsOnly);
	const typingMotionRef = useRef(typingMotion);
	typingMotionRef.current = typingMotion;
	layoutRef.current = layout;
	maxSizeRef.current = maxSize;
	exitingRef.current = exiting;
	suggestionsOnlyRef.current = suggestionsOnly;

	// The canvas is centred on the film and square, so film space and canvas space share an
	// origin — satellite targets are then the same mapping the pointer already uses.
	const measure = useCallback(() => {
		const canvas = canvasRef.current;
		const root = rootRef.current;
		const row = rowRef.current;
		const parent = root?.parentElement;
		if (!canvas || !root || !row || !parent) return;
		if (suggestionsOnlyRef.current) return;

		// The mark is an overlay on the transcript, so it has to fit the box it is given rather
		// than push the suggestion row down onto the composer in a narrow dock.
		const parentRect = parent.getBoundingClientRect();
		const style = getComputedStyle(parent);
		const availableWidth =
			parentRect.width -
			Number.parseFloat(style.paddingLeft) -
			Number.parseFloat(style.paddingRight);
		const availableHeight =
			parentRect.height -
			Number.parseFloat(style.paddingTop) -
			Number.parseFloat(style.paddingBottom);
		const rowHeight = row.offsetHeight || 48;
		const film = Math.max(
			MIN_FILM,
			Math.round(
				Math.min(
					maxSizeRef.current,
					availableWidth,
					availableHeight - rowHeight - ROW_GAP,
				),
			),
		);

		// Reach from the film's centre to whichever edge of the block is furthest — normally the
		// suggestion row — plus room for the film's bloom to fade out instead of being cut off.
		const blockHeight = film + ROW_GAP + rowHeight;
		const blockWidth = Math.max(film, row.offsetWidth);
		const reach = Math.max(film / 2, blockHeight - film / 2, blockWidth / 2);
		const side = Math.max(
			Math.ceil((reach + film * 0.3) * 2),
			Math.ceil(film / FILM_BOX),
		);
		if (film !== layoutRef.current.film || side !== layoutRef.current.side) {
			layoutRef.current = { film, side };
			setLayout({ film, side });
		}

		const rect = canvas.getBoundingClientRect();
		if (!rect.height) return;
		const geom = geomRef.current;
		geom.box = film / rect.height;
		geom.satOrbit = geom.box * SAT_ORBIT_RATIO;
		for (let i = 0; i < SAT_COUNT; i++) {
			const button = buttonRefs.current[i];
			if (!button) continue;
			const b = button.getBoundingClientRect();
			const cx = b.left + b.width / 2;
			const cy = b.top + b.height / 2;
			geom.satPos[i * 2] = ((cx - rect.left) * 2 - rect.width) / rect.height;
			geom.satPos[i * 2 + 1] =
				(rect.height - 2 * (cy - rect.top)) / rect.height;
		}
	}, []);

	useLayoutEffect(() => {
		if (suggestionsOnly) return;
		measure();
		const root = rootRef.current;
		const canvas = canvasRef.current;
		const parent = root?.parentElement;
		if (!root || !canvas || !parent) return;
		// `measure` sizes the canvas it observes, so the callback has to land a
		// frame later — a same-frame write leaves the browser with undelivered
		// resize notifications.
		const unobserve = observeResize([parent, canvas], measure);
		window.addEventListener("resize", measure);
		return () => {
			unobserve();
			window.removeEventListener("resize", measure);
		};
	}, [measure, suggestionsOnly]);

	useEffect(() => {
		if (window.matchMedia("(prefers-reduced-motion: reduce)").matches) {
			setInstant(true);
			setRevealed(true);
			return;
		}
		// Two frames so the pre-transition style is painted before the class flips.
		let second = 0;
		const first = requestAnimationFrame(() => {
			second = requestAnimationFrame(() => setRevealed(true));
		});
		return () => {
			cancelAnimationFrame(first);
			cancelAnimationFrame(second);
		};
	}, []);

	useEffect(() => {
		if (suggestionsOnly) return;
		const canvas = canvasRef.current;
		const root = rootRef.current;
		if (!canvas || !root) return;

		let elapsed = 0;
		let clock = 0;
		let spin = 0;
		let grow = 0;
		let growVelocity = 0;
		let growTarget = 1;
		let pop = 0;
		let released = false;
		let departed = false;
		// Pointer attention is read from the whole viewport, not just the film, so the orb
		// notices you crossing the page and leans at whatever you are about to click.
		const pointer = { x: 0, y: 0 };
		let hasPointer = false;
		let attention = 0;
		let mx = 0;
		let my = 0;
		const response = activity ? createTypingResponse(activity) : null;
		let typing = 0;
		let fullness = 0;
		let perk = 0;

		const onPointerMove = (event: PointerEvent) => {
			const rect = canvas.getBoundingClientRect();
			if (!rect.height) return;
			pointer.x = ((event.clientX - rect.left) * 2 - rect.width) / rect.height;
			pointer.y = (rect.height - 2 * (event.clientY - rect.top)) / rect.height;
			hasPointer = true;
		};
		window.addEventListener("pointermove", onPointerMove, { passive: true });

		const film = createBubbleFilm({
			canvas,
			host: root,
			frame: (dt, u, reduced) => {
				const geom = geomRef.current;

				// How hard the composer is being worked. Eased rather than switched, so turning
				// the interface's setting off mid-draft settles the mark instead of snapping it.
				if (response) {
					const step = response.advance(
						typingMotionRef.current && !reduced && !exitingRef.current,
						dt,
					);
					typing = step.typing;
					fullness = step.fullness;
					perk = decayPerk(perk, step.perked, dt);
				}
				const params =
					typing > 1e-3
						? mixOrbParams(
								ORB_INVITING_PARAMS,
								ORB_COMPOSING_PARAMS,
								smoothstep(typing),
							)
						: ORB_INVITING_PARAMS;

				if (reduced) {
					clock = 6 * params.rate;
					grow = 1;
					pop = 0;
					u.sat = 0;
					u.satDock.fill(0);
					u.satShrink.fill(0);
				} else {
					elapsed += dt;
					if (exitingRef.current && !departed) {
						departed = true;
						growTarget = 0.05;
						pop = 1;
					}
					clock += dt * params.rate * (1 + pop * 1.2);
					spin += dt * params.spin;
					// A real spring in seconds, so the film lands the same way at 60 and 120Hz.
					growVelocity += ((growTarget - grow) * 190 - growVelocity * 9) * dt;
					grow += growVelocity * dt;

					const ms = elapsed * 1000;
					if (!released && ms >= SAT_RELEASE) {
						released = true;
						pop = 1;
					}
					pop *= Math.exp(-2.76 * dt);
					poseSatellites(ms, geom.satOrbit, u, departed);
				}

				// A longer draft fills the mark out very slightly — enough to notice between a
				// blank composer and a written one, not enough to watch it happen.
				const box =
					geom.box *
					grow *
					params.scale *
					(1 + fullness * 0.02 + perk * PERK_SWELL) *
					(1 + Math.sin(clock * 1.6) * params.breathe);
				const aim = aimBulge(
					clock,
					box,
					hasPointer && !reduced ? pointer : null,
					attention,
					dt,
					params,
					typing,
				);
				attention = aim.attention;
				mx = approach(mx, aim.x, 7.7, dt);
				my = approach(my, aim.y, 7.7, dt);

				u.time = clock;
				u.focus =
					params.focus +
					aim.near * 0.3 +
					pop * 0.6 +
					fullness * 0.12 +
					perk * 0.35;
				u.boxX = box;
				u.boxY = box;
				u.mouseX = mx;
				u.mouseY = my;
				u.mstr = Math.min(
					1.6,
					params.bulge + attention * 0.8 + pop * 0.5 + typing * 0.35,
				);
				u.round = params.round;
				u.spin = spin;
				u.spinMix = params.spinMix;
				u.pop = pop;
				u.satPos.set(geom.satPos);
			},
		});

		if (!film) {
			window.removeEventListener("pointermove", onPointerMove);
			setFailed(true);
			return;
		}

		return () => {
			film.destroy();
			window.removeEventListener("pointermove", onPointerMove);
		};
	}, [suggestionsOnly, activity]);

	const canvasStyle = useMemo(
		() => ({
			width: layout.side,
			height: layout.side,
			left: "50%",
			top: layout.film / 2,
			transform: "translate(-50%, -50%)",
		}),
		[layout],
	);

	return (
		<div
			ref={rootRef}
			className={cn(
				"relative flex flex-col items-center transition-all duration-420 ease-[cubic-bezier(0.32,0,0.2,1)] motion-reduce:transition-none",
				exiting ? "-translate-y-3 scale-95 opacity-0" : "opacity-100",
				className,
			)}
		>
			{!suggestionsOnly &&
				(failed ? (
					<span
						aria-hidden="true"
						className="absolute top-0 rounded-full bg-linear-to-br from-primary/40 via-primary/20 to-purple-600/20 ring-1 ring-primary/40"
						style={{ width: layout.film, height: layout.film }}
					/>
				) : (
					<canvas
						ref={canvasRef}
						className="pointer-events-none absolute block"
						style={canvasStyle}
					/>
				))}
			{!suggestionsOnly && (
				<div
					aria-hidden="true"
					style={{ width: layout.film, height: layout.film }}
				/>
			)}
			<div
				ref={rowRef}
				className={cn(
					"pointer-events-auto flex flex-wrap items-center justify-center gap-3",
					!suggestionsOnly && "mt-7",
				)}
			>
				{suggestions.map((suggestion, index) => (
					<button
						key={suggestion.label}
						ref={(element) => {
							buttonRefs.current[index] = element;
						}}
						type="button"
						aria-label={suggestion.label}
						title={suggestion.label}
						onClick={() => onSelect(suggestion.prompt)}
						style={{
							borderColor: "var(--fl-chat-rule, var(--border))",
							transitionDelay:
								instant || exiting
									? "0ms"
									: `${revealDelay(index, suggestionsOnly)}ms`,
						}}
						className={cn(
							"group relative flex size-12 items-center justify-center rounded-2xl border text-muted-foreground outline-none transition-all duration-420 ease-[cubic-bezier(0.22,1,0.36,1)] hover:border-primary/45 hover:bg-background hover:text-primary focus-visible:ring-2 focus-visible:ring-primary/40 motion-safe:hover:-translate-y-0.5 motion-reduce:transition-none",
							revealed
								? "translate-y-0 scale-100 opacity-100"
								: "translate-y-3 scale-75 opacity-0",
						)}
					>
						<suggestion.icon className="size-5" />
						<span className="pointer-events-none absolute top-full mt-2 whitespace-nowrap text-[11px] font-medium text-muted-foreground opacity-0 transition-opacity group-hover:opacity-100 group-focus-visible:opacity-100">
							{suggestion.label}
						</span>
					</button>
				))}
			</div>
		</div>
	);
}
