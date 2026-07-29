"use client";

import {
	type ComponentPropsWithoutRef,
	type RefObject,
	forwardRef,
	useEffect,
	useRef,
	useState,
} from "react";
import { useForwardedRef } from "../../lib/use-forwarded-ref";
import { cn } from "../../lib/utils";
import {
	type FlowPilotOrbState,
	ORB_STATE_PARAMS,
	ORB_TEETH_COUNT,
} from "./flowpilot-orb-state";
import {
	FRAG,
	VERT,
	readTokenRGB,
	themeIsLight,
} from "./hero-variants/bubble-shader";

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

/** The soap-film canvas, drawn oversized + unclipped so its bloom never hits a hard edge. */
function BubbleOrbCanvas({
	hostRef,
	state,
	ackNonce,
}: {
	hostRef: RefObject<HTMLButtonElement | null>;
	state: FlowPilotOrbState;
	ackNonce: number;
}) {
	const canvasRef = useRef<HTMLCanvasElement>(null);
	const [failed, setFailed] = useState(false);
	// Read by the render loop. Kept in refs so changing state never tears down the GL context.
	const stateRef = useRef(state);
	const ackRef = useRef(0);
	stateRef.current = state;

	useEffect(() => {
		if (ackNonce > 0) ackRef.current = 1;
	}, [ackNonce]);

	useEffect(() => {
		const canvas = canvasRef.current;
		const host = hostRef.current;
		if (!canvas || !host) return;

		const gl = canvas.getContext("webgl", {
			alpha: true,
			premultipliedAlpha: true,
			antialias: false,
		});
		if (!gl) {
			setFailed(true);
			return;
		}

		const compile = (type: number, src: string) => {
			const shader = gl.createShader(type);
			if (!shader) return null;
			gl.shaderSource(shader, src);
			gl.compileShader(shader);
			if (!gl.getShaderParameter(shader, gl.COMPILE_STATUS)) {
				console.error(gl.getShaderInfoLog(shader));
				gl.deleteShader(shader);
				return null;
			}
			return shader;
		};

		const vs = compile(gl.VERTEX_SHADER, VERT);
		const fs = compile(gl.FRAGMENT_SHADER, FRAG);
		const prog = vs && fs ? gl.createProgram() : null;
		if (!vs || !fs || !prog) {
			setFailed(true);
			return;
		}
		gl.attachShader(prog, vs);
		gl.attachShader(prog, fs);
		gl.linkProgram(prog);
		if (!gl.getProgramParameter(prog, gl.LINK_STATUS)) {
			console.error(gl.getProgramInfoLog(prog));
			setFailed(true);
			gl.deleteProgram(prog);
			return;
		}
		gl.useProgram(prog);

		const buf = gl.createBuffer();
		gl.bindBuffer(gl.ARRAY_BUFFER, buf);
		gl.bufferData(
			gl.ARRAY_BUFFER,
			new Float32Array([-1, -1, 3, -1, -1, 3]),
			gl.STATIC_DRAW,
		);
		const loc = gl.getAttribLocation(prog, "p");
		gl.enableVertexAttribArray(loc);
		gl.vertexAttribPointer(loc, 2, gl.FLOAT, false, 0, 0);

		const uRes = gl.getUniformLocation(prog, "u_res");
		const uTime = gl.getUniformLocation(prog, "u_time");
		const uFocus = gl.getUniformLocation(prog, "u_focus");
		const uBox = gl.getUniformLocation(prog, "u_box");
		const uMouse = gl.getUniformLocation(prog, "u_mouse");
		const uMstr = gl.getUniformLocation(prog, "u_mstr");
		const uMorph = gl.getUniformLocation(prog, "u_morph");
		const uLight = gl.getUniformLocation(prog, "u_light");
		const uPrimary = gl.getUniformLocation(prog, "u_primary");
		const uRound = gl.getUniformLocation(prog, "u_round");
		const uSpin = gl.getUniformLocation(prog, "u_spin");
		const uTeeth = gl.getUniformLocation(prog, "u_teeth");
		const uTeethN = gl.getUniformLocation(prog, "u_teethN");
		const uSat = gl.getUniformLocation(prog, "u_sat");
		const uPop = gl.getUniformLocation(prog, "u_pop");
		const uSpinMix = gl.getUniformLocation(prog, "u_spin_mix");

		const reduced = window.matchMedia(
			"(prefers-reduced-motion: reduce)",
		).matches;

		const setPrimary = () => {
			const [r, g, b] = readTokenRGB("--primary", [242, 90, 60]);
			gl.uniform3f(uPrimary, r / 255, g / 255, b / 255);
		};
		setPrimary();

		let lightTarget = themeIsLight() ? 1 : 0;
		let light = lightTarget;
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

		const draw = () => {
			gl.uniform2f(uRes, canvas.width, canvas.height);
			gl.uniform1f(uTime, clock);
			gl.uniform1f(uFocus, cur.focus + ack * 1.3 + focus);
			const box =
				BOX * cur.scale * (1 + Math.sin(clock * 1.6) * cur.breathe + ack * 0.1);
			gl.uniform2f(uBox, box, box);
			gl.uniform2f(uMouse, mx, my);
			// Hover reads in every state, damped so it can never flatten the cog.
			gl.uniform1f(
				uMstr,
				Math.min(1.6, cur.bulge * (1 - mstr * 0.5) + mstr * 0.75 + ack * 0.5),
			);
			gl.uniform1f(uMorph, 0);
			gl.uniform1f(uLight, light);
			gl.uniform1f(uRound, cur.round);
			gl.uniform1f(uSpin, spin);
			gl.uniform1f(uTeeth, cur.teeth);
			gl.uniform1f(uTeethN, ORB_TEETH_COUNT);
			gl.uniform1f(uSat, cur.sat);
			gl.uniform1f(uPop, ack);
			gl.uniform1f(uSpinMix, cur.spinMix);
			gl.clearColor(0, 0, 0, 0);
			gl.clear(gl.COLOR_BUFFER_BIT);
			gl.drawArrays(gl.TRIANGLES, 0, 3);
		};

		// Reduced motion still gets a distinct still frame per state, not a shared one.
		const still = () => {
			Object.assign(cur, ORB_STATE_PARAMS[stateRef.current]);
			clock = 6 * cur.rate;
			spin = 0.6;
			ack = 0;
			draw();
		};

		// Registered after still() exists so the callback never forward-references it.
		const themeObserver = new MutationObserver(() => {
			lightTarget = themeIsLight() ? 1 : 0;
			setPrimary();
			if (reduced) {
				light = lightTarget;
				still();
			}
		});
		themeObserver.observe(document.documentElement, {
			attributes: true,
			attributeFilter: ["class", "style", "data-theme"],
		});

		const resize = () => {
			const dpr = Math.min(window.devicePixelRatio || 1, 2);
			const width = canvas.clientWidth;
			const height = canvas.clientHeight;
			if (!width || !height) return;
			canvas.width = Math.round(width * dpr);
			canvas.height = Math.round(height * dpr);
			gl.viewport(0, 0, canvas.width, canvas.height);
			if (reduced) still();
		};
		resize();
		const observer = new ResizeObserver(resize);
		observer.observe(canvas);

		let raf = 0;
		let frame = 0;
		let running = false;
		let inViewport = true;
		let documentVisible = document.visibilityState !== "hidden";

		let last = 0;
		const loop = (ms: number) => {
			if (!running) return;
			const dt = last ? Math.min((ms - last) / 1000, 0.05) : 0.016;
			last = ms;
			const target = ORB_STATE_PARAMS[stateRef.current];
			for (const key of Object.keys(cur) as (keyof typeof cur)[]) {
				cur[key] += (target[key] - cur[key]) * 0.07;
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
			mx += (ox * (1 - mstr) + mxTarget * mstr - mx) * 0.12;
			my += (oy * (1 - mstr) + myTarget * mstr - my) * 0.12;
			if (++frame % 20 === 0) lightTarget = themeIsLight() ? 1 : 0;
			light += (lightTarget - light) * 0.08;
			draw();
			raf = requestAnimationFrame(loop);
		};

		const stopAnimation = () => {
			if (!running) return;
			running = false;
			cancelAnimationFrame(raf);
			raf = 0;
		};

		const syncAnimation = () => {
			const shouldAnimate = inViewport && documentVisible;
			if (reduced) {
				if (shouldAnimate) still();
				return;
			}
			if (!shouldAnimate) {
				stopAnimation();
				return;
			}
			if (running) return;
			running = true;
			raf = requestAnimationFrame(loop);
		};

		const viewportObserver =
			typeof IntersectionObserver === "undefined"
				? null
				: new IntersectionObserver(([entry]) => {
						inViewport = entry?.isIntersecting ?? false;
						syncAnimation();
					});
		viewportObserver?.observe(host);

		const onVisibilityChange = () => {
			documentVisible = document.visibilityState !== "hidden";
			syncAnimation();
		};
		document.addEventListener("visibilitychange", onVisibilityChange);
		syncAnimation();

		return () => {
			stopAnimation();
			observer.disconnect();
			viewportObserver?.disconnect();
			document.removeEventListener("visibilitychange", onVisibilityChange);
			host.removeEventListener("pointerenter", onEnter);
			host.removeEventListener("pointerleave", onLeave);
			host.removeEventListener("pointermove", onMove);
			themeObserver.disconnect();
			gl.deleteBuffer(buf);
			gl.deleteProgram(prog);
			gl.deleteShader(vs);
			gl.deleteShader(fs);
		};
		// stateRef/ackRef are refs on purpose: the GL context must survive state changes.
	}, [hostRef]);

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

export type FlowPilotBubbleOrbProps = ComponentPropsWithoutRef<"button"> & {
	/** What the assistant is doing. Defaults to `idle` for callers that don't track it. */
	orbState?: FlowPilotOrbState;
	/** Bump to fire the acknowledge burst (e.g. on a state change). */
	ackNonce?: number;
};

/**
 * Pure, inline FlowPilot orb button. It owns only the canonical shader and pointer interaction;
 * callers supply positioning, animation, click behavior, and any surrounding assistant state.
 */
export const FlowPilotBubbleOrb = forwardRef<
	HTMLButtonElement,
	FlowPilotBubbleOrbProps
>(function FlowPilotBubbleOrb(
	{
		className,
		children,
		type = "button",
		orbState = "idle",
		ackNonce = 0,
		"aria-label": ariaLabel = "Ask FlowPilot",
		...props
	},
	forwardedRef,
) {
	const hostRef = useForwardedRef(forwardedRef);

	return (
		<button
			ref={hostRef}
			type={type}
			aria-label={ariaLabel}
			className={cn(
				"relative inline-flex size-[72px] shrink-0 items-center justify-center overflow-visible rounded-full border-0 bg-transparent p-0 outline-none focus-visible:ring-2 focus-visible:ring-primary/40",
				className,
			)}
			{...props}
		>
			<BubbleOrbCanvas hostRef={hostRef} state={orbState} ackNonce={ackNonce} />
			{children ?? <span className="sr-only">{ariaLabel}</span>}
		</button>
	);
});
