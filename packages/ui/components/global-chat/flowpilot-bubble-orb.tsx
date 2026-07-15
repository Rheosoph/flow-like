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
}: {
	hostRef: RefObject<HTMLButtonElement | null>;
}) {
	const canvasRef = useRef<HTMLCanvasElement>(null);
	const [failed, setFailed] = useState(false);

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
		const themeObserver = new MutationObserver(() => {
			lightTarget = themeIsLight() ? 1 : 0;
			setPrimary();
			if (reduced) {
				light = lightTarget;
				draw(4200);
			}
		});
		themeObserver.observe(document.documentElement, {
			attributes: true,
			attributeFilter: ["class", "style", "data-theme"],
		});

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

		const draw = (ms: number) => {
			gl.uniform2f(uRes, canvas.width, canvas.height);
			gl.uniform1f(uTime, ms / 1000);
			gl.uniform1f(uFocus, focus);
			gl.uniform2f(uBox, BOX, BOX);
			gl.uniform2f(uMouse, mx, my);
			gl.uniform1f(uMstr, mstr);
			gl.uniform1f(uMorph, 0);
			gl.uniform1f(uLight, light);
			gl.clearColor(0, 0, 0, 0);
			gl.clear(gl.COLOR_BUFFER_BIT);
			gl.drawArrays(gl.TRIANGLES, 0, 3);
		};

		const resize = () => {
			const dpr = Math.min(window.devicePixelRatio || 1, 2);
			const width = canvas.clientWidth;
			const height = canvas.clientHeight;
			if (!width || !height) return;
			canvas.width = Math.round(width * dpr);
			canvas.height = Math.round(height * dpr);
			gl.viewport(0, 0, canvas.width, canvas.height);
			if (reduced) draw(4200);
		};
		resize();
		const observer = new ResizeObserver(resize);
		observer.observe(canvas);

		let raf = 0;
		let frame = 0;
		let running = false;
		let inViewport = true;
		let documentVisible = document.visibilityState !== "hidden";

		const loop = (ms: number) => {
			if (!running) return;
			focus += (focusTarget - focus) * 0.06;
			mstr += (mstrTarget - mstr) * 0.08;
			mx += (mxTarget - mx) * 0.12;
			my += (myTarget - my) * 0.12;
			if (++frame % 20 === 0) lightTarget = themeIsLight() ? 1 : 0;
			light += (lightTarget - light) * 0.08;
			draw(ms);
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
				if (shouldAnimate) draw(4200);
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

export type FlowPilotBubbleOrbProps = ComponentPropsWithoutRef<"button">;

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
			<BubbleOrbCanvas hostRef={hostRef} />
			{children ?? <span className="sr-only">{ariaLabel}</span>}
		</button>
	);
});
