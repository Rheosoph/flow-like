"use client";

import { useEffect, useMemo, useRef, useState } from "react";
import { cn } from "../../lib";

/* ── Animated node graph ──────────────────────────────────────────── */

interface GraphNode {
	id: number;
	cx: number;
	cy: number;
	r: number;
	delay: number;
}

interface GraphEdge {
	from: number;
	to: number;
	delay: number;
}

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

const VERTEX_SHADER_SOURCE = `
attribute vec2 aPosition;

void main() {
	gl_Position = vec4(aPosition, 0.0, 1.0);
}
`;

const FRAGMENT_SHADER_SOURCE = `
precision mediump float;

uniform vec2 uResolution;
uniform float uTime;
uniform vec3 uBackground;
uniform vec3 uForeground;
uniform vec3 uPrimary;
uniform vec3 uAccent;
uniform vec3 uTertiary;

float hash(vec2 p) {
	return fract(sin(dot(p, vec2(127.1, 311.7))) * 43758.5453123);
}

float noise(vec2 p) {
	vec2 i = floor(p);
	vec2 f = fract(p);
	vec2 u = f * f * (3.0 - 2.0 * f);
	return mix(
		mix(hash(i), hash(i + vec2(1.0, 0.0)), u.x),
		mix(hash(i + vec2(0.0, 1.0)), hash(i + vec2(1.0, 1.0)), u.x),
		u.y
	);
}

float fbm(vec2 p) {
	float value = 0.0;
	float amplitude = 0.5;
	for (int i = 0; i < 3; i++) {
		value += noise(p) * amplitude;
		p = p * 2.04 + 9.17;
		amplitude *= 0.5;
	}
	return value;
}

float lane(vec2 uv, float offset, float phase) {
	float wave = sin(uv.x * 3.1 + phase + uTime * 0.28) * 0.12;
	wave += sin(uv.x * 7.0 - phase + uTime * 0.11) * 0.035;
	return smoothstep(0.055, 0.0, abs(uv.y - offset - wave));
}

void main() {
	vec2 uv = (gl_FragCoord.xy - 0.5 * uResolution.xy) / max(uResolution.y, 1.0);
	float drift = fbm(uv * 1.8 + vec2(uTime * 0.035, -uTime * 0.025));
	vec2 flow = uv + vec2(drift * 0.12, fbm(uv * 1.25 - uTime * 0.03) * 0.08);

	float laneA = lane(flow, -0.16, 0.0);
	float laneB = lane(flow, 0.08, 2.0);
	float laneC = lane(flow, 0.28, 4.0);

	vec2 grid = fract((uv + vec2(uTime * 0.018, -uTime * 0.012)) * vec2(6.5, 4.0)) - 0.5;
	float nodes = 1.0 - smoothstep(0.01, 0.035, dot(grid, grid));
	float vignette = smoothstep(1.35, 0.15, length(uv * vec2(0.65, 0.95)));

	vec3 color = uBackground;
	color += uPrimary * laneA * 0.28 * vignette;
	color += uAccent * laneB * 0.22 * vignette;
	color += uTertiary * laneC * 0.16 * vignette;
	color += mix(uPrimary, uForeground, 0.18) * nodes * 0.045 * vignette;
	color = mix(color, uBackground, smoothstep(0.2, 1.28, length(uv)));

	gl_FragColor = vec4(color, 1.0);
}
`;

function buildGraph(): { nodes: GraphNode[]; edges: GraphEdge[] } {
	const nodes: GraphNode[] = [
		{ id: 0, cx: 50, cy: 60, r: 6, delay: 0 },
		{ id: 1, cx: 130, cy: 35, r: 5, delay: 0.3 },
		{ id: 2, cx: 130, cy: 85, r: 5, delay: 0.5 },
		{ id: 3, cx: 210, cy: 60, r: 7, delay: 0.8 },
		{ id: 4, cx: 290, cy: 35, r: 5, delay: 1.1 },
		{ id: 5, cx: 290, cy: 85, r: 5, delay: 1.3 },
		{ id: 6, cx: 370, cy: 60, r: 6, delay: 1.6 },
	];
	const edges: GraphEdge[] = [
		{ from: 0, to: 1, delay: 0.15 },
		{ from: 0, to: 2, delay: 0.25 },
		{ from: 1, to: 3, delay: 0.55 },
		{ from: 2, to: 3, delay: 0.65 },
		{ from: 3, to: 4, delay: 0.95 },
		{ from: 3, to: 5, delay: 1.05 },
		{ from: 4, to: 6, delay: 1.35 },
		{ from: 5, to: 6, delay: 1.45 },
	];
	return { nodes, edges };
}

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
		FRAGMENT_SHADER_SOURCE,
	);
	if (!vertexShader || !fragmentShader) return null;

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

function NodeGraph() {
	const { nodes, edges } = useMemo(() => buildGraph(), []);

	return (
		<svg
			viewBox="0 0 420 120"
			className="w-full max-w-xs h-auto"
			fill="none"
			role="img"
			aria-label="Workflow progress"
		>
			{/* edges with traveling-dot animation */}
			{edges.map((e) => {
				const a = nodes[e.from];
				const b = nodes[e.to];
				const pathId = `e-${e.from}-${e.to}`;
				return (
					<g key={pathId}>
						<path
							id={pathId}
							d={`M${a.cx},${a.cy} C${(a.cx + b.cx) / 2},${a.cy} ${(a.cx + b.cx) / 2},${b.cy} ${b.cx},${b.cy}`}
							className="pls-edge"
							style={{ animationDelay: `${e.delay}s` }}
						/>
						<circle r="2.5" className="pls-particle">
							<animateMotion
								dur="2s"
								repeatCount="indefinite"
								begin={`${e.delay}s`}
							>
								<mpath href={`#${pathId}`} />
							</animateMotion>
						</circle>
					</g>
				);
			})}

			{/* nodes */}
			{nodes.map((n) => (
				<g key={n.id}>
					<circle
						cx={n.cx}
						cy={n.cy}
						r={n.r + 4}
						className="pls-ring"
						style={{ animationDelay: `${n.delay}s` }}
					/>
					<circle
						cx={n.cx}
						cy={n.cy}
						r={n.r}
						className="pls-node"
						style={{ animationDelay: `${n.delay}s` }}
					/>
				</g>
			))}
		</svg>
	);
}

/* ── Step labels ──────────────────────────────────────────────────── */

const STEPS = [
	"Initializing workflow",
	"Loading resources",
	"Processing data",
	"Preparing interface",
];

function StepIndicator() {
	const [step, setStep] = useState(0);

	useEffect(() => {
		const id = setInterval(() => {
			setStep((s) => (s + 1) % STEPS.length);
		}, 2200);
		return () => clearInterval(id);
	}, []);

	return (
		<div className="flex items-center gap-2">
			<div className="flex gap-1">
				{STEPS.map((stepLabel, i) => (
					<div
						key={stepLabel}
						className={cn(
							"h-1 rounded-full transition-all duration-500",
							i <= step ? "w-5 bg-primary/60" : "w-1.5 bg-muted-foreground/15",
						)}
					/>
				))}
			</div>
			<span className="text-xs text-muted-foreground/50 min-w-32.5 transition-all duration-300">
				{STEPS[step]}…
			</span>
		</div>
	);
}

function WorkflowLoadingShader() {
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
		<div
			className="pointer-events-none absolute inset-0 overflow-hidden"
			aria-hidden
		>
			<div
				className="absolute inset-0 opacity-70"
				style={{
					background:
						"linear-gradient(135deg, color-mix(in oklch, var(--primary) 14%, var(--background)) 0%, var(--background) 54%, color-mix(in oklch, var(--tertiary) 12%, var(--background)) 100%)",
				}}
			/>
			<canvas ref={canvasRef} className="absolute inset-0 h-full w-full" />
			<div
				className="absolute inset-0"
				style={{
					background:
						"radial-gradient(ellipse at center, transparent 0%, color-mix(in oklch, var(--background) 30%, transparent) 58%, var(--background) 100%)",
				}}
			/>
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
				"relative flex flex-col items-center justify-center h-full w-full gap-8 overflow-hidden bg-background p-8",
				className,
			)}
		>
			<WorkflowLoadingShader />

			{/* animated node graph */}
			<div className="relative z-10 pls-enter">
				<NodeGraph />
			</div>

			{/* status area */}
			<div
				className="relative z-10 flex flex-col items-center gap-3 rounded-lg border border-border/45 bg-background/60 px-5 py-4 shadow-sm backdrop-blur-md pls-enter"
				style={{ animationDelay: "0.2s" }}
			>
				<p className="text-sm font-medium text-foreground/70">{title}</p>
				<StepIndicator />
			</div>

			<style>{`
				/* entry */
				.pls-enter {
					animation: pls-enter 0.7s ease-out both;
				}
				@keyframes pls-enter {
					from { opacity: 0; transform: translateY(12px) scale(0.97); }
					to   { opacity: 1; transform: translateY(0) scale(1); }
				}

				/* graph edges */
				.pls-edge {
					stroke: color-mix(in oklch, var(--primary) 18%, transparent);
					stroke-width: 1.5;
					stroke-dasharray: 200;
					stroke-dashoffset: 200;
					animation: pls-draw 1.2s ease-out forwards;
				}
				@keyframes pls-draw {
					to { stroke-dashoffset: 0; }
				}

				/* traveling particle */
				.pls-particle {
					fill: color-mix(in oklch, var(--primary) 62%, var(--foreground));
				}

				/* graph nodes */
				.pls-node {
					fill: color-mix(in oklch, var(--primary) 16%, var(--background));
					stroke: color-mix(in oklch, var(--primary) 42%, var(--foreground));
					stroke-width: 1.5;
					animation: pls-pop 0.5s ease-out both;
				}
				@keyframes pls-pop {
					from { r: 0; opacity: 0; }
					to   { opacity: 1; }
				}

				/* outer ring pulse on nodes */
				.pls-ring {
					fill: none;
					stroke: color-mix(in oklch, var(--primary) 12%, transparent);
					stroke-width: 1;
					animation: pls-ring-pulse 2.5s ease-in-out infinite;
				}
				@keyframes pls-ring-pulse {
					0%, 100% { opacity: 0.4; r: inherit; }
					50%      { opacity: 0;   }
				}

				@media (prefers-reduced-motion: reduce) {
					.pls-enter,
					.pls-edge,
					.pls-node,
					.pls-ring {
						animation: none !important;
					}
				}
			`}</style>
		</div>
	);
}
