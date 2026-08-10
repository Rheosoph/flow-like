"use client";

import { useEffect, useRef, useState } from "react";
import { cn } from "../../lib";

/* ── Theme sampling ───────────────────────────────────────────────── */

type RGB = readonly [number, number, number];

interface ShaderTheme {
	background: RGB;
	foreground: RGB;
	primary: RGB;
	accent: RGB;
	tertiary: RGB;
}

const DEFAULT_SHADER_THEME: ShaderTheme = {
	background: [1, 1, 1],
	foreground: [0.1, 0.1, 0.1],
	primary: [0.91, 0.18, 0.13],
	accent: [0.94, 0.42, 0.3],
	tertiary: [0.92, 0.55, 0.18],
};

function clamp01(value: number): number {
	return Math.min(Math.max(value, 0), 1);
}

function parseCssNumber(value: string): number {
	const trimmed = value.trim();
	if (trimmed === "none") return 0;
	if (trimmed.endsWith("%")) return Number.parseFloat(trimmed) / 100;
	return Number.parseFloat(trimmed);
}

function parseHue(value: string): number {
	const trimmed = value.trim();
	if (trimmed === "none") return 0;
	if (trimmed.endsWith("turn")) return Number.parseFloat(trimmed) * 360;
	if (trimmed.endsWith("rad"))
		return (Number.parseFloat(trimmed) * 180) / Math.PI;
	if (trimmed.endsWith("grad")) return Number.parseFloat(trimmed) * 0.9;
	return Number.parseFloat(trimmed);
}

function linearToSrgb(value: number): number {
	const clamped = Math.max(value, 0);
	return clamped <= 0.0031308
		? clamped * 12.92
		: 1.055 * clamped ** (1 / 2.4) - 0.055;
}

function oklchToRgb(lightness: number, chroma: number, hue: number): RGB {
	const radians = (hue * Math.PI) / 180;
	const a = chroma * Math.cos(radians);
	const b = chroma * Math.sin(radians);
	const lPrime = lightness + 0.3963377774 * a + 0.2158037573 * b;
	const mPrime = lightness - 0.1055613458 * a - 0.0638541728 * b;
	const sPrime = lightness - 0.0894841775 * a - 1.291485548 * b;
	const l = lPrime ** 3;
	const m = mPrime ** 3;
	const s = sPrime ** 3;

	return [
		clamp01(
			linearToSrgb(4.0767416621 * l - 3.3077115913 * m + 0.2309699292 * s),
		),
		clamp01(
			linearToSrgb(-1.2684380046 * l + 2.6097574011 * m - 0.3413193965 * s),
		),
		clamp01(
			linearToSrgb(-0.0041960863 * l - 0.7034186147 * m + 1.707614701 * s),
		),
	];
}

function parseOklchColor(value: string): RGB | null {
	const match = value.match(/^oklch\((.+)\)$/i);
	if (!match) return null;

	const [main] = match[1].split("/");
	const parts = main.trim().split(/\s+/);
	if (parts.length < 3) return null;

	let lightness = parseCssNumber(parts[0]);
	if (!parts[0].endsWith("%") && lightness > 1) lightness /= 100;
	const chroma = parseCssNumber(parts[1]);
	const hue = parseHue(parts[2]);

	if (![lightness, chroma, hue].every(Number.isFinite)) return null;
	return oklchToRgb(clamp01(lightness), Math.max(chroma, 0), hue);
}

function parseRgbColor(value: string): RGB | null {
	const match = value.match(/^rgba?\((.+)\)$/i);
	if (!match) return null;

	const [main] = match[1].split("/");
	const parts = main.replaceAll(",", " ").trim().split(/\s+/);
	if (parts.length < 3) return null;

	const channels = parts
		.slice(0, 3)
		.map((part) =>
			clamp01(
				part.endsWith("%")
					? Number.parseFloat(part) / 100
					: Number.parseFloat(part) / 255,
			),
		);
	if (channels.some((channel) => !Number.isFinite(channel))) return null;
	return channels as unknown as RGB;
}

function parseHexColor(value: string): RGB | null {
	const match = value.match(
		/^#([0-9a-f]{3}|[0-9a-f]{4}|[0-9a-f]{6}|[0-9a-f]{8})$/i,
	);
	if (!match) return null;

	const hex = match[1];
	const expand = (char: string) => `${char}${char}`;
	const components =
		hex.length <= 4
			? [expand(hex[0]), expand(hex[1]), expand(hex[2])]
			: [hex.slice(0, 2), hex.slice(2, 4), hex.slice(4, 6)];
	return components.map(
		(component) => Number.parseInt(component, 16) / 255,
	) as unknown as RGB;
}

function resolveCanvasColor(value: string): RGB | null {
	if (typeof document === "undefined") return null;

	const canvas = document.createElement("canvas");
	canvas.width = 1;
	canvas.height = 1;
	const context = canvas.getContext("2d", { willReadFrequently: true });
	if (!context) return null;

	context.fillStyle = "#010203";
	context.fillStyle = value;
	if (
		context.fillStyle === "#010203" &&
		!["#010203", "rgb(1, 2, 3)", "rgba(1, 2, 3, 1)"].includes(value)
	) {
		return null;
	}

	context.fillRect(0, 0, 1, 1);
	const [red, green, blue, alpha] = context.getImageData(0, 0, 1, 1).data;
	if (alpha === 0) return null;
	return [red / 255, green / 255, blue / 255];
}

function parseCssColor(value: string): RGB | null {
	const color = value.trim();
	if (!color) return null;
	return (
		parseOklchColor(color) ??
		parseRgbColor(color) ??
		parseHexColor(color) ??
		resolveCanvasColor(color)
	);
}

function luminance([red, green, blue]: RGB): number {
	return red * 0.2126 + green * 0.7152 + blue * 0.0722;
}

function mixColor(from: RGB, to: RGB, amount: number): RGB {
	const t = clamp01(amount);
	return [
		from[0] + (to[0] - from[0]) * t,
		from[1] + (to[1] - from[1]) * t,
		from[2] + (to[2] - from[2]) * t,
	];
}

function ensureVisible(color: RGB, background: RGB, fallback: RGB): RGB {
	if (Math.abs(luminance(color) - luminance(background)) > 0.12) return color;
	return mixColor(color, fallback, 0.7);
}

function readShaderTheme(): ShaderTheme {
	if (typeof window === "undefined") return DEFAULT_SHADER_THEME;

	const styles = getComputedStyle(document.documentElement);
	const readColor = (name: string, fallback: RGB) =>
		parseCssColor(styles.getPropertyValue(name)) ?? fallback;
	const background = readColor("--background", DEFAULT_SHADER_THEME.background);
	const foreground = ensureVisible(
		readColor("--foreground", DEFAULT_SHADER_THEME.foreground),
		background,
		DEFAULT_SHADER_THEME.foreground,
	);
	const primary = ensureVisible(
		readColor("--primary", DEFAULT_SHADER_THEME.primary),
		background,
		foreground,
	);
	const accent = ensureVisible(
		readColor("--accent", DEFAULT_SHADER_THEME.accent),
		background,
		primary,
	);
	const tertiary = ensureVisible(
		readColor("--tertiary", DEFAULT_SHADER_THEME.tertiary),
		background,
		primary,
	);

	return { background, foreground, primary, accent, tertiary };
}

/* ── Shader ───────────────────────────────────────────────────────── */

const VERTEX_SHADER_SOURCE = `
attribute vec2 aPosition;

void main() {
	gl_Position = vec4(aPosition, 0.0, 1.0);
}
`;

const GLSL_PRELUDE = `
precision mediump float;

uniform vec2 uResolution;
uniform float uTime;
uniform vec3 uBackground;
uniform vec3 uForeground;
uniform vec3 uPrimary;
uniform vec3 uAccent;
uniform vec3 uTertiary;

vec2 fl_uv() {
	return (gl_FragCoord.xy - 0.5 * uResolution.xy) / max(uResolution.y, 1.0);
}

float fl_hash(vec2 p) {
	return fract(sin(dot(p, vec2(127.1, 311.7))) * 43758.5453123);
}

float fl_noise(vec2 p) {
	vec2 i = floor(p);
	vec2 f = fract(p);
	vec2 u = f * f * (3.0 - 2.0 * f);
	return mix(
		mix(fl_hash(i), fl_hash(i + vec2(1.0, 0.0)), u.x),
		mix(fl_hash(i + vec2(0.0, 1.0)), fl_hash(i + vec2(1.0, 1.0)), u.x),
		u.y
	);
}

float fl_ease(float t) {
	return t * t * (3.0 - 2.0 * t);
}
`;

/**
 * Flux Lattice: a field of junctions wired by hashed filaments, crossed by
 * wavefronts that ignite whatever they pass. Junctions sample the wave at their
 * own centre so ignition snaps to the graph; filaments sample it continuously so
 * the flow stays smooth.
 */
const FLUX_LATTICE_FRAGMENT = `
const float FXL_PITCH_PX = 40.0;

float fxl_extent(vec2 dir) {
	float halfAspect = 0.5 * uResolution.x / max(uResolution.y, 1.0);
	return abs(dir.x) * halfAspect + abs(dir.y) * 0.5;
}

float fxl_front(vec2 p, float ang, float speed, float width, float phase) {
	vec2 dir = vec2(cos(ang), sin(ang));
	vec2 perp = vec2(-dir.y, dir.x);
	float span = 2.0 * (fxl_extent(dir) + width * 2.4);
	float u = fract(uTime * speed / span + phase);
	float pos = (u - 0.5) * span;
	float bend = width * 0.34 * sin(dot(p, perp) * 2.2 + phase * 21.7 + uTime * 0.21);
	float s = dot(p, dir) - pos + bend;

	float crest = exp(-min(s * s / (width * width), 24.0));
	float behind = 1.0 - smoothstep(-width * 0.2, width * 0.5, s);
	float tail = behind * exp(min(s, 0.0) / (width * 1.5));
	float life = smoothstep(0.0, 0.1, u) * (1.0 - smoothstep(0.9, 1.0, u));

	return (crest + tail * 0.22) * life;
}

float fxl_field(vec2 p) {
	float w = fxl_front(p, 0.10, 0.40, 0.105, 0.50);
	w += fxl_front(p, -0.62, 0.25, 0.150, 0.13) * 0.48;
	w += fxl_front(p, 2.75, 0.32, 0.080, 0.78) * 0.52;
	w += fxl_front(p, 1.42, 0.22, 0.145, 0.31) * 0.40;
	return w / (1.0 + 0.45 * w);
}

vec3 fxl_ember(float e) {
	vec3 warm = mix(uAccent, uPrimary, smoothstep(0.05, 0.45, e));
	return mix(warm, uTertiary, smoothstep(0.55, 1.05, e));
}

float fxl_link(vec2 cell, vec2 seed, float lo, float hi) {
	return smoothstep(lo, hi, fl_hash(cell * 0.1373 + seed));
}

float fxl_seg(float d, float presence, float halfWidth, float soft) {
	return presence * (1.0 - smoothstep(halfWidth, halfWidth + soft, d));
}

void main() {
	vec2 uv = fl_uv();
	float px = 1.0 / max(uResolution.y, 1.0);
	float halfAspect = max(0.5 * uResolution.x / max(uResolution.y, 1.0), 0.001);
	vec2 frame = uv / vec2(halfAspect, 0.5);

	float cells = clamp(uResolution.y / FXL_PITCH_PX, 7.5, 20.0);
	float pitch = 1.0 / cells;
	vec2 drift = vec2(sin(uTime * 0.017), sin(uTime * 0.0113 + 1.7)) * 0.07;
	vec2 grid = (uv + drift) * cells;
	vec2 id = floor(grid);
	vec2 local = (fract(grid) - 0.5) * pitch;
	vec2 junction = (id + vec2(0.5)) * pitch - drift;

	float wSnap = fxl_field(junction);
	float wFlow = fxl_field(uv);
	float igniteJ = fl_ease(clamp((wSnap - 0.09) * 1.55, 0.0, 1.0));
	float igniteF = fl_ease(clamp((wFlow - 0.09) * 1.35, 0.0, 1.0));

	float filWidth = max(pitch * 0.014, px * 0.62);
	float soft = px * 1.3;
	vec2 seedH = vec2(11.3, 4.7);
	vec2 seedV = vec2(2.9, 27.1);

	/* min(local.x, 0.0) measures distance to the ray leaving this junction to the
	   RIGHT (and max to the one arriving from the left), so the outgoing halves
	   hash this cell and the incoming halves hash the neighbour behind them. Both
	   halves of an edge then resolve to one hash and meet without a seam. */
	float mFil = fxl_seg(
		length(vec2(min(local.x, 0.0), local.y)),
		fxl_link(id, seedH, 0.34, 0.52),
		filWidth,
		soft
	);
	mFil = max(mFil, fxl_seg(
		length(vec2(max(local.x, 0.0), local.y)),
		fxl_link(id - vec2(1.0, 0.0), seedH, 0.34, 0.52),
		filWidth,
		soft
	));
	mFil = max(mFil, fxl_seg(
		length(vec2(local.x, min(local.y, 0.0))),
		fxl_link(id, seedV, 0.44, 0.62),
		filWidth,
		soft
	));
	mFil = max(mFil, fxl_seg(
		length(vec2(local.x, max(local.y, 0.0))),
		fxl_link(id - vec2(0.0, 1.0), seedV, 0.44, 0.62),
		filWidth,
		soft
	));

	float hub = step(0.93, fl_hash(id * 0.2113 + vec2(3.7, 8.2)));
	float radius = max(pitch * 0.070, px * 1.3) * (1.0 + hub * 0.6);
	float d = length(local);
	float mJunction = 1.0 - smoothstep(radius, radius + px * 1.6, d);
	float ringWidth = px * 0.75;
	float mRing = hub * (1.0 - smoothstep(ringWidth, ringWidth + px * 1.5, abs(d - radius * 2.6)));
	float haloRadius = pitch * 0.26;
	float mHalo = exp(-min(dot(local, local) / (haloRadius * haloRadius), 16.0));
	float ringGate = smoothstep(0.4, 0.85, igniteJ);

	float breath = 0.6 + 0.8 * fl_noise(uv * 1.35 + vec2(uTime * 0.009, uTime * -0.006));

	vec3 emberJ = fxl_ember(wSnap);
	vec3 emberF = fxl_ember(wFlow);
	vec3 inkFil = mix(uBackground, uForeground, 0.50);
	vec3 inkJ = mix(uBackground, uForeground, 0.62);

	vec3 color = uBackground;
	color = mix(color, emberF, max(wFlow - 0.12, 0.0) * 0.055);
	color = mix(color, emberJ, mHalo * igniteJ * 0.15);
	color = mix(
		color,
		mix(inkFil, emberF, igniteF),
		clamp(mFil * (0.125 * breath + 0.55 * igniteF), 0.0, 1.0)
	);
	color = mix(
		color,
		mix(inkJ, emberJ, igniteJ),
		clamp(mJunction * (0.21 * breath + 0.74 * igniteJ), 0.0, 1.0)
	);
	color = mix(color, emberJ, mRing * ringGate * 0.62);

	/* Additive bloom only where the theme is dark enough to carry it, so the
	   light theme reads the wavefront as warm ink instead of blowing out. */
	float darkness = clamp(1.0 - dot(uBackground, vec3(0.299, 0.587, 0.114)), 0.0, 1.0);
	vec3 bloom =
		emberJ * (mJunction * 0.42 + mHalo * 0.14 + mRing * ringGate * 0.30) * igniteJ +
		emberF * mFil * igniteF * 0.16;
	color += bloom * darkness * 0.75;

	float vignette = 1.0 - smoothstep(0.46, 1.14, length(frame * vec2(0.86, 0.78)));
	float calm = mix(0.44, 1.0, smoothstep(0.14, 0.58, length(frame * vec2(0.62, 1.25))));
	color = mix(uBackground, color, vignette * calm);
	color += (fl_hash(fract(gl_FragCoord.xy * vec2(0.0173, 0.0219))) - 0.5) * 0.0045;

	gl_FragColor = vec4(color, 1.0);
}
`;

function compileShader(
	gl: WebGLRenderingContext,
	type: number,
	source: string,
): WebGLShader | null {
	const shader = gl.createShader(type);
	if (!shader) return null;

	gl.shaderSource(shader, source);
	gl.compileShader(shader);
	if (!gl.getShaderParameter(shader, gl.COMPILE_STATUS)) {
		console.warn(
			"Page loading shader compile failed:",
			gl.getShaderInfoLog(shader),
		);
		gl.deleteShader(shader);
		return null;
	}
	return shader;
}

function createShaderProgram(gl: WebGLRenderingContext): WebGLProgram | null {
	const vertexShader = compileShader(
		gl,
		gl.VERTEX_SHADER,
		VERTEX_SHADER_SOURCE,
	);
	const fragmentShader = compileShader(
		gl,
		gl.FRAGMENT_SHADER,
		`${GLSL_PRELUDE}\n${FLUX_LATTICE_FRAGMENT}`,
	);
	if (!vertexShader || !fragmentShader) {
		if (vertexShader) gl.deleteShader(vertexShader);
		if (fragmentShader) gl.deleteShader(fragmentShader);
		return null;
	}

	const program = gl.createProgram();
	if (!program) return null;

	gl.attachShader(program, vertexShader);
	gl.attachShader(program, fragmentShader);
	gl.linkProgram(program);
	gl.deleteShader(vertexShader);
	gl.deleteShader(fragmentShader);

	if (!gl.getProgramParameter(program, gl.LINK_STATUS)) {
		console.warn(
			"Page loading shader link failed:",
			gl.getProgramInfoLog(program),
		);
		gl.deleteProgram(program);
		return null;
	}
	return program;
}

function LatticeCanvas() {
	const canvasRef = useRef<HTMLCanvasElement>(null);
	const themeRef = useRef<ShaderTheme>(DEFAULT_SHADER_THEME);

	useEffect(() => {
		themeRef.current = readShaderTheme();

		let scheduledFrame: number | null = null;
		const updateTheme = () => {
			scheduledFrame = null;
			themeRef.current = readShaderTheme();
		};
		const scheduleThemeUpdate = () => {
			if (scheduledFrame !== null) return;
			scheduledFrame = window.requestAnimationFrame(updateTheme);
		};

		const observer = new MutationObserver(scheduleThemeUpdate);
		observer.observe(document.documentElement, {
			attributes: true,
			attributeFilter: ["class", "data-theme", "style"],
		});
		observer.observe(document.head, {
			childList: true,
			characterData: true,
			subtree: true,
		});

		const systemTheme = window.matchMedia("(prefers-color-scheme: dark)");
		systemTheme.addEventListener("change", scheduleThemeUpdate);

		return () => {
			observer.disconnect();
			systemTheme.removeEventListener("change", scheduleThemeUpdate);
			if (scheduledFrame !== null) {
				window.cancelAnimationFrame(scheduledFrame);
			}
		};
	}, []);

	useEffect(() => {
		const canvas = canvasRef.current;
		if (!canvas) return;

		const gl = canvas.getContext("webgl", {
			alpha: true,
			antialias: false,
			depth: false,
			powerPreference: "low-power",
			preserveDrawingBuffer: false,
			stencil: false,
		});
		if (!gl) return;

		const program = createShaderProgram(gl);
		if (!program) return;

		const positionBuffer = gl.createBuffer();
		const positionLocation = gl.getAttribLocation(program, "aPosition");
		if (!positionBuffer || positionLocation < 0) {
			gl.deleteProgram(program);
			return;
		}

		const uniforms = {
			resolution: gl.getUniformLocation(program, "uResolution"),
			time: gl.getUniformLocation(program, "uTime"),
			background: gl.getUniformLocation(program, "uBackground"),
			foreground: gl.getUniformLocation(program, "uForeground"),
			primary: gl.getUniformLocation(program, "uPrimary"),
			accent: gl.getUniformLocation(program, "uAccent"),
			tertiary: gl.getUniformLocation(program, "uTertiary"),
		};

		gl.bindBuffer(gl.ARRAY_BUFFER, positionBuffer);
		gl.bufferData(
			gl.ARRAY_BUFFER,
			new Float32Array([-1, -1, 3, -1, -1, 3]),
			gl.STATIC_DRAW,
		);
		gl.useProgram(program);
		gl.enableVertexAttribArray(positionLocation);
		gl.vertexAttribPointer(positionLocation, 2, gl.FLOAT, false, 0, 0);

		const resize = () => {
			const pixelRatio = Math.min(window.devicePixelRatio || 1, 1.5);
			const width = Math.max(
				1,
				Math.floor((canvas.clientWidth || window.innerWidth) * pixelRatio),
			);
			const height = Math.max(
				1,
				Math.floor((canvas.clientHeight || window.innerHeight) * pixelRatio),
			);

			if (canvas.width !== width || canvas.height !== height) {
				canvas.width = width;
				canvas.height = height;
				gl.viewport(0, 0, width, height);
			}
		};

		const setColor = (location: WebGLUniformLocation | null, color: RGB) => {
			if (!location) return;
			gl.uniform3f(location, color[0], color[1], color[2]);
		};

		const reducedMotion = window.matchMedia("(prefers-reduced-motion: reduce)");
		let animationFrame: number | null = null;
		let lastFrame = 0;

		const render = (time: number) => {
			animationFrame = null;
			if (
				!document.hidden &&
				(reducedMotion.matches || time - lastFrame >= 1000 / 45)
			) {
				lastFrame = time;
				resize();

				const theme = themeRef.current;
				gl.useProgram(program);
				if (uniforms.resolution) {
					gl.uniform2f(uniforms.resolution, canvas.width, canvas.height);
				}
				if (uniforms.time) gl.uniform1f(uniforms.time, time * 0.001);
				setColor(uniforms.background, theme.background);
				setColor(uniforms.foreground, theme.foreground);
				setColor(uniforms.primary, theme.primary);
				setColor(uniforms.accent, theme.accent);
				setColor(uniforms.tertiary, theme.tertiary);
				gl.drawArrays(gl.TRIANGLES, 0, 3);
			}

			if (!reducedMotion.matches) {
				animationFrame = window.requestAnimationFrame(render);
			}
		};

		animationFrame = window.requestAnimationFrame(render);

		return () => {
			if (animationFrame !== null) {
				window.cancelAnimationFrame(animationFrame);
			}
			gl.deleteBuffer(positionBuffer);
			gl.deleteProgram(program);
		};
	}, []);

	return (
		<canvas
			ref={canvasRef}
			className="absolute inset-0 h-full w-full"
			aria-hidden
		/>
	);
}

/* ── Readout ──────────────────────────────────────────────────────── */

const PHASES = [
	"Initializing workflow",
	"Loading resources",
	"Processing data",
	"Preparing interface",
] as const;

const PHASE_MS = 2200;

/**
 * The phase is the only thing on screen that changes, so it is the only thing
 * set large. The run title drops to a tracked eyebrow carrying four ticks.
 */
function PhaseReadout({ title }: Readonly<{ title: string }>) {
	const [index, setIndex] = useState(0);
	const [outgoing, setOutgoing] = useState<number | null>(null);
	const indexRef = useRef(0);

	useEffect(() => {
		if (
			typeof window !== "undefined" &&
			window.matchMedia("(prefers-reduced-motion: reduce)").matches
		) {
			return;
		}

		const id = setInterval(() => {
			const previous = indexRef.current;
			const next = (previous + 1) % PHASES.length;
			indexRef.current = next;
			setOutgoing(previous);
			setIndex(next);
		}, PHASE_MS);

		return () => clearInterval(id);
	}, []);

	useEffect(() => {
		if (outgoing === null) return;
		const id = setTimeout(() => setOutgoing(null), 1000);
		return () => clearTimeout(id);
	}, [outgoing]);

	return (
		<div className="fxl-readout">
			<div className="fxl-eyebrow">
				<span className="fxl-kicker">{title}</span>
				<span
					className="fxl-ticks"
					role="img"
					aria-label={`Step ${index + 1} of ${PHASES.length}`}
				>
					{PHASES.map((phase, position) => (
						<i
							key={phase}
							className={cn(
								"fxl-tick",
								position < index && "is-done",
								position === index && "is-cur",
							)}
						>
							<i className="fxl-tick-fill" />
						</i>
					))}
				</span>
			</div>

			<output className="fxl-stack" aria-live="polite">
				{outgoing !== null && (
					<p key={`out-${outgoing}`} className="fxl-phase is-out" aria-hidden>
						{PHASES[outgoing]}
					</p>
				)}
				<p
					key={`in-${index}`}
					className={cn("fxl-phase", outgoing !== null && "is-in")}
				>
					{PHASES[index]}
				</p>
			</output>
		</div>
	);
}

/* ── Main component ───────────────────────────────────────────────── */

export function PageLoadingSkeleton({
	className,
	title = "Running workflow",
}: Readonly<{ className?: string; title?: string }>) {
	return (
		<div
			className={cn(
				"fxl-root relative flex h-full w-full items-center justify-center overflow-hidden bg-background",
				className,
			)}
		>
			<LatticeCanvas />
			<PhaseReadout title={title} />

			<style>{`
				.fxl-root {
					container-type: inline-size;
					container-name: loading;
					padding: 1.5rem;
				}

				/* Two bands, nothing else: a quiet tracked eyebrow carrying the run
				   title and its four progress ticks, and the live phase set at display
				   scale beneath it. */
				.fxl-root .fxl-readout {
					position: relative;
					z-index: 10;
					max-width: 100%;
					display: flex;
					flex-direction: column;
					align-items: center;
					gap: clamp(0.5rem, 1.1cqi, 0.95rem);
					text-align: center;
					animation: fxl-rise 0.9s cubic-bezier(0.22, 1, 0.36, 1) both;

					/* Knockout halo painted in the page's own ground colour. It follows
					   the glyph outlines exactly, so an ignited filament crossing behind
					   the type is pushed off the letterforms without a card or a pane. */
					--fxl-halo:
						0 0 0.09em color-mix(in oklab, var(--background) 96%, transparent),
						0 0 0.2em color-mix(in oklab, var(--background) 92%, transparent),
						0 0 0.42em color-mix(in oklab, var(--background) 82%, transparent),
						0 0 0.85em color-mix(in oklab, var(--background) 64%, transparent),
						0 0 1.7em color-mix(in oklab, var(--background) 42%, transparent);
				}

				/* Broad feathered wash that deepens the shader's own centre calm rather
				   than covering it — it never reaches an edge, so it reads as sky. */
				.fxl-root .fxl-readout::before {
					content: "";
					position: absolute;
					left: 50%;
					top: 50%;
					width: calc(100% + clamp(4rem, 14cqi, 15rem));
					height: calc(100% + clamp(3rem, 9cqi, 9rem));
					transform: translate(-50%, -50%);
					background: radial-gradient(
						52% 56% at 50% 50%,
						color-mix(in oklab, var(--background) 80%, transparent) 0%,
						color-mix(in oklab, var(--background) 60%, transparent) 44%,
						color-mix(in oklab, var(--background) 24%, transparent) 70%,
						transparent 89%
					);
					pointer-events: none;
				}

				.fxl-root .fxl-eyebrow {
					position: relative;
					z-index: 1;
					display: flex;
					align-items: center;
					gap: clamp(0.65rem, 1.5cqi, 1.1rem);
				}

				.fxl-root .fxl-kicker {
					margin-right: -0.34em;
					font-size: clamp(0.5625rem, 0.82cqi, 0.75rem);
					font-weight: 500;
					line-height: 1;
					letter-spacing: 0.34em;
					text-transform: uppercase;
					white-space: nowrap;
					color: color-mix(in oklab, var(--foreground) 58%, var(--muted-foreground));
					text-shadow:
						0 0 0.45em color-mix(in oklab, var(--background) 97%, transparent),
						0 0 0.9em color-mix(in oklab, var(--background) 90%, transparent),
						0 0 2em color-mix(in oklab, var(--background) 66%, transparent);
				}

				/* Four ticks, not four junctions: a flat measure that cannot be mistaken
				   for the lattice behind it. The live one charges across its own dwell. */
				.fxl-root .fxl-ticks {
					display: flex;
					align-items: center;
					gap: clamp(4px, 0.55cqi, 7px);
				}

				.fxl-root .fxl-tick {
					position: relative;
					flex: 0 0 auto;
					width: clamp(13px, 1.5cqi, 20px);
					height: 3px;
					border-radius: 999px;
					overflow: hidden;
					background: color-mix(in oklab, var(--foreground) 24%, transparent);
					box-shadow: 0 0 0 1.5px color-mix(in oklab, var(--background) 74%, transparent);
					transition: background 320ms ease;
				}

				.fxl-root .fxl-tick.is-done {
					background: color-mix(in oklab, var(--foreground) 46%, transparent);
				}

				/* Tinted before it charges, so the live tick is never mistaken for a
				   pending one in the first moments after an advance. */
				.fxl-root .fxl-tick.is-cur {
					background: color-mix(in oklab, var(--primary) 30%, transparent);
					box-shadow:
						0 0 0 1.5px color-mix(in oklab, var(--background) 74%, transparent),
						0 0 9px color-mix(in oklab, var(--primary) 45%, transparent);
				}

				.fxl-root .fxl-tick-fill {
					position: absolute;
					inset: 0;
					transform: scaleX(0);
					transform-origin: left center;
					background: var(--primary);
				}

				.fxl-root .fxl-tick.is-cur .fxl-tick-fill {
					animation: fxl-charge 2.2s linear both;
				}

				/* One cell, two layers: the outgoing phase and the incoming one overlap
				   here so the change is a dissolve in place rather than a reflow. */
				.fxl-root .fxl-stack {
					position: relative;
					z-index: 1;
					display: grid;
					max-width: 100%;
					font-size: clamp(1.5rem, 5.5cqi, 3.75rem);
				}

				.fxl-root .fxl-stack::before {
					content: "";
					position: absolute;
					inset: -0.46em -1.7em;
					background: radial-gradient(
						50% 50% at 50% 50%,
						color-mix(in oklab, var(--background) 76%, transparent) 0%,
						color-mix(in oklab, var(--background) 54%, transparent) 50%,
						transparent 100%
					);
					pointer-events: none;
				}

				.fxl-root .fxl-phase {
					grid-area: 1 / 1;
					position: relative;
					z-index: 1;
					justify-self: center;
					margin: 0 -0.06em 0 0;
					font-size: 1em;
					font-weight: 300;
					line-height: 1.12;
					letter-spacing: 0.06em;
					white-space: nowrap;
					color: var(--foreground);
					text-shadow: var(--fxl-halo);
				}

				/* Staggered so the two phrases never share the line at full strength:
				   the old one is gone by the time the new one comes into focus. */
				.fxl-root .fxl-phase.is-in {
					animation: fxl-phase-in 0.72s cubic-bezier(0.16, 1, 0.3, 1) 0.15s both;
				}

				.fxl-root .fxl-phase.is-out {
					animation: fxl-phase-out 0.34s cubic-bezier(0.4, 0, 0.25, 1) both;
				}

				/* Above the fold the pair can breathe and the eyebrow opens up; below it
				   the two bands close ranks so the phase keeps the space. */
				@container loading (min-width: 30rem) {
					.fxl-root .fxl-readout {
						gap: clamp(0.85rem, 1.5cqi, 1.4rem);
					}

					.fxl-root .fxl-eyebrow {
						gap: clamp(0.85rem, 1.6cqi, 1.25rem);
					}

					.fxl-root .fxl-kicker {
						margin-right: -0.38em;
						letter-spacing: 0.38em;
					}
				}

				@keyframes fxl-rise {
					from { opacity: 0; transform: translateY(12px) scale(0.985); }
					to   { opacity: 1; transform: none; }
				}

				@keyframes fxl-charge {
					from { transform: scaleX(0); }
					to   { transform: scaleX(1); }
				}

				/* Tracking-in: the phrase resolves out of the field instead of appearing. */
				@keyframes fxl-phase-in {
					from { opacity: 0; transform: translateY(0.22em); filter: blur(10px); letter-spacing: 0.2em; }
					55%  { opacity: 1; }
					to   { opacity: 1; transform: none; filter: blur(0); letter-spacing: 0.06em; }
				}

				@keyframes fxl-phase-out {
					from { opacity: 1; transform: none; filter: blur(0); letter-spacing: 0.06em; }
					to   { opacity: 0; transform: translateY(-0.16em); filter: blur(8px); letter-spacing: 0.11em; }
				}

				@media (prefers-reduced-motion: reduce) {
					.fxl-root .fxl-readout { animation: none; }

					.fxl-root .fxl-phase.is-in,
					.fxl-root .fxl-phase.is-out {
						animation: none;
					}

					.fxl-root .fxl-tick {
						transition: none;
					}

					.fxl-root .fxl-tick.is-cur .fxl-tick-fill {
						animation: none;
						transform: none;
					}
				}
			`}</style>
		</div>
	);
}
