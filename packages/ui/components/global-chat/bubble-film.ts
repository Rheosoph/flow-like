"use client";

import {
	FRAG,
	VERT,
	readTokenRGB,
	themeIsLight,
} from "./hero-variants/bubble-shader";

/**
 * The uniform block a frame callback fills in. Handed to `frame` as a single mutable object so a
 * render loop never allocates. Every default reproduces the shader's own no-op, so a caller only
 * has to touch the uniforms it actually drives.
 */
export interface BubbleFilmUniforms {
	time: number;
	focus: number;
	boxX: number;
	boxY: number;
	mouseX: number;
	mouseY: number;
	mstr: number;
	morph: number;
	round: number;
	spin: number;
	teeth: number;
	teethN: number;
	sat: number;
	pop: number;
	spinMix: number;
	/** 1 removes the body and its satellites, leaving the blob field to carry the film. */
	bodyOff: number;
	/**
	 * The mark's nominal half-extent, for the directional grade and the window streak. 0 uses
	 * `boxX`/`boxY`, which is right whenever the body is the whole mark; a pose whose body shrinks
	 * while blobs carry the silhouette has to pass the size the mark still reads as.
	 */
	gradeX: number;
	gradeY: number;
	/**
	 * Light-mode saturation punch. 0 keeps the shader's built-in 2.45, which was tuned against the
	 * 72px launcher; a film rendered much larger reads as hot magenta at that value and should pass
	 * its own — the empty-state mark uses 1.35.
	 */
	satL: number;
	/** Eight blobs, packed xy centre / radius / presence. Presence 0 is a no-op. */
	blob: Float32Array;
	/** Per-blob smooth-min radius. 0 is a hard union. */
	blobK: Float32Array;
	/** Per-blob 0 wobble space … 1 screen space. Aim at a DOM rect in screen space. */
	blobUv: Float32Array;
}

export const BUBBLE_BLOB_COUNT = 8;

/**
 * Writes one blob into the uniform block. Radii below a pixel or so are dropped rather than
 * clamped: `length(p - o) - 0` is 0 at the centre, which lights a singular bright point.
 */
export function setBubbleBlob(
	u: BubbleFilmUniforms,
	index: number,
	x: number,
	y: number,
	radius: number,
	smooth = 0,
	screenSpace = 0,
) {
	if (index < 0 || index >= BUBBLE_BLOB_COUNT || !(radius > 0.0012)) return;
	const at = index * 4;
	u.blob[at] = x;
	u.blob[at + 1] = y;
	u.blob[at + 2] = radius;
	u.blob[at + 3] = 1;
	u.blobK[index] = smooth;
	u.blobUv[index] = screenSpace;
}

/** Clears the blob field. A frame callback owns every blob it wants, so it starts from empty. */
export function clearBubbleBlobs(u: BubbleFilmUniforms) {
	u.blob.fill(0);
	u.blobK.fill(0);
	u.blobUv.fill(0);
}

export interface BubbleFilmHandle {
	destroy(): void;
}

export interface BubbleFilmOptions {
	canvas: HTMLCanvasElement;
	/** Watched for viewport intersection, so an off-screen film costs nothing. */
	host: HTMLElement;
	/**
	 * Advances the caller's simulation and fills `u`. With `reduced` set the film is not looping —
	 * this is a single still frame, so pose the orb rather than integrating `dt`.
	 */
	frame: (dt: number, u: BubbleFilmUniforms, reduced: boolean) => void;
}

function createUniforms(): BubbleFilmUniforms {
	return {
		time: 0,
		focus: 0,
		boxX: 0.48,
		boxY: 0.48,
		mouseX: 0,
		mouseY: 0,
		mstr: 0,
		morph: 0,
		round: 0,
		spin: 0,
		teeth: 0,
		teethN: 8,
		sat: 0,
		pop: 0,
		spinMix: 0,
		bodyOff: 0,
		gradeX: 0,
		gradeY: 0,
		satL: 0,
		blob: new Float32Array(BUBBLE_BLOB_COUNT * 4),
		blobK: new Float32Array(BUBBLE_BLOB_COUNT),
		blobUv: new Float32Array(BUBBLE_BLOB_COUNT),
	};
}

/**
 * Owns everything about running the soap-film shader — context, program, DPR sizing, theme
 * tracking, reduced motion, and the visibility-gated render loop — and leaves the caller with
 * nothing but a per-frame simulation. Returns null if WebGL is unavailable, so callers can fall
 * back to a static mark.
 */
export function createBubbleFilm({
	canvas,
	host,
	frame,
}: BubbleFilmOptions): BubbleFilmHandle | null {
	const gl = canvas.getContext("webgl", {
		alpha: true,
		premultipliedAlpha: true,
		antialias: false,
	});
	if (!gl) return null;

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
		if (vs) gl.deleteShader(vs);
		if (fs) gl.deleteShader(fs);
		if (prog) gl.deleteProgram(prog);
		return null;
	}
	gl.attachShader(prog, vs);
	gl.attachShader(prog, fs);
	gl.linkProgram(prog);
	if (!gl.getProgramParameter(prog, gl.LINK_STATUS)) {
		console.error(gl.getProgramInfoLog(prog));
		gl.deleteProgram(prog);
		gl.deleteShader(vs);
		gl.deleteShader(fs);
		return null;
	}
	gl.useProgram(prog);

	const buf = gl.createBuffer();
	gl.bindBuffer(gl.ARRAY_BUFFER, buf);
	gl.bufferData(
		gl.ARRAY_BUFFER,
		new Float32Array([-1, -1, 3, -1, -1, 3]),
		gl.STATIC_DRAW,
	);
	const attr = gl.getAttribLocation(prog, "p");
	gl.enableVertexAttribArray(attr);
	gl.vertexAttribPointer(attr, 2, gl.FLOAT, false, 0, 0);

	const at = (name: string) => gl.getUniformLocation(prog, name);
	const uRes = at("u_res");
	const uTime = at("u_time");
	const uFocus = at("u_focus");
	const uBox = at("u_box");
	const uMouse = at("u_mouse");
	const uMstr = at("u_mstr");
	const uMorph = at("u_morph");
	const uLight = at("u_light");
	const uPrimary = at("u_primary");
	const uRound = at("u_round");
	const uSpin = at("u_spin");
	const uTeeth = at("u_teeth");
	const uTeethN = at("u_teethN");
	const uSat = at("u_sat");
	const uPop = at("u_pop");
	const uSpinMix = at("u_spin_mix");
	const uBodyOff = at("u_body_off");
	const uGrade = at("u_grade");
	const uSatL = at("u_sat_l");
	const uBlob = at("u_blob[0]");
	const uBlobK = at("u_blob_k[0]");
	const uBlobUv = at("u_blob_uv[0]");

	const reduced = window.matchMedia("(prefers-reduced-motion: reduce)").matches;
	const u = createUniforms();

	const setPrimary = () => {
		const [r, g, b] = readTokenRGB("--primary", [242, 90, 60]);
		gl.uniform3f(uPrimary, r / 255, g / 255, b / 255);
	};
	setPrimary();

	let lightTarget = themeIsLight() ? 1 : 0;
	let light = lightTarget;

	const draw = () => {
		gl.uniform2f(uRes, canvas.width, canvas.height);
		gl.uniform1f(uTime, u.time);
		gl.uniform1f(uFocus, u.focus);
		gl.uniform2f(uBox, u.boxX, u.boxY);
		gl.uniform2f(uMouse, u.mouseX, u.mouseY);
		gl.uniform1f(uMstr, u.mstr);
		gl.uniform1f(uMorph, u.morph);
		gl.uniform1f(uLight, light);
		gl.uniform1f(uRound, u.round);
		gl.uniform1f(uSpin, u.spin);
		gl.uniform1f(uTeeth, u.teeth);
		gl.uniform1f(uTeethN, u.teethN);
		gl.uniform1f(uSat, u.sat);
		gl.uniform1f(uPop, u.pop);
		gl.uniform1f(uSpinMix, u.spinMix);
		gl.uniform1f(uBodyOff, u.bodyOff);
		gl.uniform2f(uGrade, u.gradeX, u.gradeY);
		gl.uniform1f(uSatL, u.satL);
		gl.uniform4fv(uBlob, u.blob);
		gl.uniform1fv(uBlobK, u.blobK);
		gl.uniform1fv(uBlobUv, u.blobUv);
		gl.clearColor(0, 0, 0, 0);
		gl.clear(gl.COLOR_BUFFER_BIT);
		gl.drawArrays(gl.TRIANGLES, 0, 3);
	};

	/** Reduced motion still gets a per-state still frame, not one shared pose. */
	const still = () => {
		clearBubbleBlobs(u);
		frame(0, u, true);
		draw();
	};

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
	const sizeObserver = new ResizeObserver(resize);
	sizeObserver.observe(canvas);

	let raf = 0;
	let frames = 0;
	let running = false;
	let inViewport = true;
	let documentVisible = document.visibilityState !== "hidden";
	let last = 0;

	const loop = (ms: number) => {
		if (!running) return;
		const dt = last ? Math.min((ms - last) / 1000, 0.05) : 0.016;
		last = ms;
		// The blob field is owned per frame, never accumulated: a caller that stops writing a
		// blob has removed it, and never has to remember to clear one it no longer wants.
		clearBubbleBlobs(u);
		frame(dt, u, false);
		if (++frames % 20 === 0) lightTarget = themeIsLight() ? 1 : 0;
		light += (lightTarget - light) * 0.08;
		draw();
		raf = requestAnimationFrame(loop);
	};

	const stop = () => {
		if (!running) return;
		running = false;
		cancelAnimationFrame(raf);
		raf = 0;
	};

	const sync = () => {
		const shouldAnimate = inViewport && documentVisible;
		if (reduced) {
			if (shouldAnimate) still();
			return;
		}
		if (!shouldAnimate) {
			stop();
			return;
		}
		if (running) return;
		running = true;
		// A resumed loop must not integrate the whole time it spent parked.
		last = 0;
		raf = requestAnimationFrame(loop);
	};

	const viewportObserver =
		typeof IntersectionObserver === "undefined"
			? null
			: new IntersectionObserver(([entry]) => {
					inViewport = entry?.isIntersecting ?? false;
					sync();
				});
	viewportObserver?.observe(host);

	const onVisibilityChange = () => {
		documentVisible = document.visibilityState !== "hidden";
		sync();
	};
	document.addEventListener("visibilitychange", onVisibilityChange);
	sync();

	return {
		destroy() {
			stop();
			sizeObserver.disconnect();
			viewportObserver?.disconnect();
			themeObserver.disconnect();
			document.removeEventListener("visibilitychange", onVisibilityChange);
			gl.deleteBuffer(buf);
			gl.deleteProgram(prog);
			gl.deleteShader(vs);
			gl.deleteShader(fs);
		},
	};
}
