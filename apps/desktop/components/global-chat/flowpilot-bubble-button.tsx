"use client";

import { AnimatePresence, motion } from "framer-motion";
import { usePathname } from "next/navigation";
import { type RefObject, useEffect, useRef, useState } from "react";
import { createPortal } from "react-dom";
import { useGlobalChatStore } from "../../lib/global-chat-store";
import {
	FRAG,
	VERT,
	readTokenRGB,
	themeIsLight,
} from "./hero-variants/bubble-shader";

// Round soap-film bubble (square u_box + full radius, no composer morph) reusing the exact
// start-page shader, so the launcher is a shrunk copy of the hero bubble.
const BOX = 0.48;
// Hit area of the launcher; the canvas is drawn larger and unclipped so the film's outer bloom
// fades to true transparency instead of being cut into a square by the button box. That square cut
// is the WebKit/Tauri halo (border-radius does not clip a <canvas>), so there is deliberately NO
// opaque/clipped backing — the premultiplied film composites straight onto the page, like the hero.
const BUTTON_PX = 72;
const CANVAS_PX = 152;
const CANVAS_OFFSET = (CANVAS_PX - BUTTON_PX) / 2;

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

		// hover physics: the film swells and bulges toward the pointer. Pointer events are read from
		// the host button because the oversized canvas is pointer-events:none (so its bloom, which
		// overhangs the button, never eats clicks meant for the page).
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
		const onMove = (e: PointerEvent) => {
			const r = canvas.getBoundingClientRect();
			if (!r.height) return;
			mxTarget = ((e.clientX - r.left) * 2 - r.width) / r.height;
			myTarget = (r.height - 2 * (e.clientY - r.top)) / r.height;
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
			const w = canvas.clientWidth;
			const h = canvas.clientHeight;
			if (!w || !h) return;
			canvas.width = Math.round(w * dpr);
			canvas.height = Math.round(h * dpr);
			gl.viewport(0, 0, canvas.width, canvas.height);
			if (reduced) draw(4200);
		};
		resize();
		const observer = new ResizeObserver(resize);
		observer.observe(canvas);

		let raf = 0;
		let frame = 0;
		if (reduced) {
			draw(4200);
		} else {
			const loop = (ms: number) => {
				focus += (focusTarget - focus) * 0.06;
				mstr += (mstrTarget - mstr) * 0.08;
				mx += (mxTarget - mx) * 0.12;
				my += (myTarget - my) * 0.12;
				if (++frame % 20 === 0) lightTarget = themeIsLight() ? 1 : 0;
				light += (lightTarget - light) * 0.08;
				draw(ms);
				raf = requestAnimationFrame(loop);
			};
			raf = requestAnimationFrame(loop);
		}

		return () => {
			cancelAnimationFrame(raf);
			observer.disconnect();
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
				width: CANVAS_PX,
				height: CANVAS_PX,
				left: -CANVAS_OFFSET,
				top: -CANVAS_OFFSET,
			}}
		/>
	);
}

// Routes that already surface FlowPilot themselves, so the floating launcher would be redundant.
// The board/widget builders deliberately use the bubble instead of their own in-interface button, so
// they are NOT listed here — only the full chat view is.
const HIDDEN_ROUTE_PREFIXES = ["/chat"];

/**
 * The small round FlowPilot launcher docked bottom-right on every page except the start page (which
 * hosts the full hero bubble) and /chat (which IS the assistant). Clicking it opens the docked
 * overlay — the same conversation the hero bubble and /chat share — which balloons out of this corner
 * so the bubble reads as morphing into the chat. Desktop only.
 */
export function FlowPilotBubbleButton() {
	const pathname = usePathname();
	const mode = useGlobalChatStore((s) => s.mode);
	const openOverlay = useGlobalChatStore((s) => s.openOverlay);
	const hostRef = useRef<HTMLButtonElement>(null);

	// Hide where a FlowPilot entry point already exists: the hero bubble (start page), the full chat
	// view, the board/widget builders, and while the docked overlay itself is open.
	const hidden =
		pathname === "/" ||
		mode === "overlay" ||
		HIDDEN_ROUTE_PREFIXES.some(
			(prefix) => pathname === prefix || pathname.startsWith(`${prefix}/`),
		);

	const content = (
		<AnimatePresence>
			{!hidden && (
				<motion.button
					ref={hostRef}
					type="button"
					onClick={openOverlay}
					aria-label="Ask FlowPilot"
					title="Ask FlowPilot"
					initial={{ opacity: 0, scale: 0.3 }}
					animate={{ opacity: 1, scale: 1 }}
					exit={{ opacity: 0, scale: 0.5, transition: { duration: 0.12 } }}
					transition={{ type: "spring", stiffness: 360, damping: 22 }}
					whileHover={{ scale: 1.08 }}
					whileTap={{ scale: 0.92 }}
					style={{ width: BUTTON_PX, height: BUTTON_PX }}
					className="fixed bottom-6 right-6 z-9998 hidden rounded-full outline-none focus-visible:ring-2 focus-visible:ring-primary/40 md:block"
				>
					<BubbleOrbCanvas hostRef={hostRef} />
					<span className="sr-only">Ask FlowPilot</span>
				</motion.button>
			)}
		</AnimatePresence>
	);

	if (typeof document === "undefined") return content;
	return createPortal(content, document.body);
}
