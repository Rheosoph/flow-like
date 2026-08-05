"use client";

import { type RefObject, useEffect, useRef, useState } from "react";
import {
	type IComposerActivityChannel,
	applyComposerLean,
	createTypingResponse,
	decayPerk,
} from "../../lib/composer-activity";
import { createBubbleFilm } from "./bubble-film";
import {
	type FlowPilotOrbState,
	ORB_COMPOSING_PARAMS,
	ORB_STATE_PARAMS,
	ORB_TEETH_COUNT,
} from "./flowpilot-orb-state";

// Round soap-film bubble (square u_box + full radius, no composer morph) reusing the exact
// start-page shader, so every orb is a scaled copy of the hero bubble.
const BOX = 0.48;

// The canvas is drawn larger and unclipped so the film's outer bloom fades to true transparency
// instead of being cut into a square by the button box. Preserve the launcher's original 72:152
// ratio as percentages so callers can resize the button with ordinary className/style props.
const BUTTON_PX = 72;
const CANVAS_PX = 152;
const CANVAS_SCALE_PERCENT = (CANVAS_PX / BUTTON_PX) * 100;
const CANVAS_OFFSET_PERCENT = ((CANVAS_PX - BUTTON_PX) / 2 / BUTTON_PX) * 100;

/** The swell that greets the start of a burst of typing — a breath, not the entrance's shockwave. */
const PERK_SWELL = 0.05;

const smoothstep = (value: number) => value * value * (3 - 2 * value);

/**
 * The soap-film canvas, drawn oversized + unclipped so its bloom never hits a hard edge.
 *
 * Purely presentational: the host element supplies the size and position, and is what pointer and
 * viewport-visibility are read from. Callers range from the 72px launcher button to a large
 * non-interactive mark, and every one renders the identical film.
 */
export function BubbleOrbFilm({
	hostRef,
	state,
	ackNonce,
	activity,
	typingMotion = false,
}: {
	hostRef: RefObject<HTMLElement | null>;
	state: FlowPilotOrbState;
	ackNonce: number;
	/** The composer whose draft the film answers. Omit for a mark with no composer of its own. */
	activity?: IComposerActivityChannel;
	typingMotion?: boolean;
}) {
	const canvasRef = useRef<HTMLCanvasElement>(null);
	const [failed, setFailed] = useState(false);
	// Read by the render loop. Kept in refs so changing state never tears down the GL context.
	const stateRef = useRef(state);
	const ackRef = useRef(0);
	const typingMotionRef = useRef(typingMotion);
	stateRef.current = state;
	typingMotionRef.current = typingMotion;

	useEffect(() => {
		if (ackNonce > 0) ackRef.current = 1;
	}, [ackNonce]);

	useEffect(() => {
		const canvas = canvasRef.current;
		const host = hostRef.current;
		if (!canvas || !host) return;

		// Hover physics: the film swells and bulges toward the pointer. Pointer events are read from
		// the host button because the oversized canvas is pointer-events:none, so its bloom never eats
		// clicks meant for nearby content.
		let mstr = 0;
		let mstrTarget = 0;
		let mx = 0;
		let my = 0;
		let mxTarget = 0;
		let myTarget = 0;
		let focus = 0;
		let focusTarget = 0;
		const onEnter = () => {
			mstrTarget = 1;
			focusTarget = 0.55;
		};
		const onLeave = () => {
			mstrTarget = 0;
			focusTarget = 0;
		};
		const onMove = (event: PointerEvent) => {
			const rect = canvas.getBoundingClientRect();
			if (!rect.height) return;
			mxTarget = ((event.clientX - rect.left) * 2 - rect.width) / rect.height;
			myTarget = (rect.height - 2 * (event.clientY - rect.top)) / rect.height;
		};
		host.addEventListener("pointerenter", onEnter);
		host.addEventListener("pointerleave", onLeave);
		host.addEventListener("pointermove", onMove);

		// The orb runs its own clock, advanced at the active state's rate, and its own spin.
		// Everything the shader churns comes off u_time, so the rate is what makes idle sit
		// nearly still while thinking boils.
		const cur = { ...ORB_STATE_PARAMS[stateRef.current] };
		let clock = 0;
		let spin = 0;
		let ack = 0;
		const response = activity ? createTypingResponse(activity) : null;
		let typing = 0;
		let fullness = 0;
		let perk = 0;

		const film = createBubbleFilm({
			canvas,
			host,
			frame: (dt, u, reduced) => {
				if (reduced) {
					Object.assign(cur, ORB_STATE_PARAMS[stateRef.current]);
					clock = 6 * cur.rate;
					spin = 0.6;
					ack = 0;
					typing = 0;
					fullness = 0;
					perk = 0;
				} else {
					if (response) {
						const step = response.advance(typingMotionRef.current, dt);
						typing = step.typing;
						fullness = step.fullness;
						perk = decayPerk(perk, step.perked, dt);
					}
					// The configured pose is where the orb rests; writing carries it toward the
					// composing pose. Typing only ever *adds* energy — a pose already busier than
					// composing (the cog's teeth, thinking's satellites) keeps its own value
					// instead of being flattened into something the author did not pick.
					const target = ORB_STATE_PARAMS[stateRef.current];
					const lift = smoothstep(typing);
					for (const key of Object.keys(cur) as (keyof typeof cur)[]) {
						const to =
							target[key] +
							Math.max(0, ORB_COMPOSING_PARAMS[key] - target[key]) * lift;
						cur[key] += (to - cur[key]) * 0.07;
					}
					clock += dt * cur.rate;
					spin += dt * cur.spin;
					if (ackRef.current > 0) {
						ack = ackRef.current;
						ackRef.current = 0;
					}
					ack *= 0.955;
					focus += (focusTarget - focus) * 0.06;
					mstr += (mstrTarget - mstr) * 0.08;
					// With no pointer on it the bulge travels an ellipse, so the film always has
					// somewhere to go; the pointer takes over smoothly as you hover.
					const ox = Math.cos(clock * 0.9) * cur.reach;
					const oy = Math.sin(clock * 1.15) * cur.reach * 0.8;
					const lean = applyComposerLean(
						ox * (1 - mstr) + mxTarget * mstr,
						oy * (1 - mstr) + myTarget * mstr,
						typing,
						clock,
						BOX,
					);
					mx += (lean.x - mx) * 0.12;
					my += (lean.y - my) * 0.12;
				}

				u.time = clock;
				u.focus = cur.focus + ack * 1.3 + focus + fullness * 0.12 + perk * 0.35;
				const box =
					BOX *
					cur.scale *
					(1 + fullness * 0.02 + perk * PERK_SWELL) *
					(1 + Math.sin(clock * 1.6) * cur.breathe + ack * 0.1);
				u.boxX = box;
				u.boxY = box;
				u.mouseX = mx;
				u.mouseY = my;
				// Hover reads in every state, damped so it can never flatten the cog.
				u.mstr = Math.min(
					1.6,
					cur.bulge * (1 - mstr * 0.5) +
						mstr * 0.75 +
						ack * 0.5 +
						typing * 0.35,
				);
				u.round = cur.round;
				u.spin = spin;
				u.teeth = cur.teeth;
				u.teethN = ORB_TEETH_COUNT;
				u.sat = cur.sat;
				u.pop = ack;
				u.spinMix = cur.spinMix;
			},
		});

		if (!film) {
			host.removeEventListener("pointerenter", onEnter);
			host.removeEventListener("pointerleave", onLeave);
			host.removeEventListener("pointermove", onMove);
			setFailed(true);
			return;
		}

		return () => {
			film.destroy();
			host.removeEventListener("pointerenter", onEnter);
			host.removeEventListener("pointerleave", onLeave);
			host.removeEventListener("pointermove", onMove);
		};
		// stateRef/ackRef are refs on purpose: the GL context must survive state changes.
	}, [hostRef, activity]);

	if (failed) {
		return (
			<span
				aria-hidden="true"
				className="absolute inset-0 rounded-full bg-linear-to-br from-primary/40 via-primary/20 to-purple-600/20 ring-1 ring-primary/40"
			/>
		);
	}

	return (
		<canvas
			ref={canvasRef}
			className="pointer-events-none absolute"
			style={{
				width: `${CANVAS_SCALE_PERCENT}%`,
				height: `${CANVAS_SCALE_PERCENT}%`,
				left: `${-CANVAS_OFFSET_PERCENT}%`,
				top: `${-CANVAS_OFFSET_PERCENT}%`,
			}}
		/>
	);
}
