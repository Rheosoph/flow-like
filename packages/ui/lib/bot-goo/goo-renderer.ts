/**
 * One body of goo in CSS pixels of its canvas: a rounded box, or with `to` a stroke that tapers
 * from `radius` at (x, y) to `to.radius` at its end (hw and hh are unused then).
 */
export interface GooBlob {
	x: number;
	y: number;
	/** Half width and half height. */
	hw: number;
	hh: number;
	radius: number;
	/** Melts into the den's edge with concave shoulders, like a notch. */
	anchored?: boolean;
	to?: { x: number; y: number; radius: number };
}

/** The screen edge a den grows out of. */
export type DenEdge = "top" | "right";

/** Where the den's edge runs: `at` is its y for the top edge, its x for the right one. */
export interface GooAnchor {
	edge: DenEdge;
	at: number;
}

/** How the goo is lit: its body's grey level and the light along its rim. */
export interface GooLook {
	body: number;
	rim: number;
}

/** Near-black glass: the notch, and the den on a light theme. */
export const INK_GOO: GooLook = { body: 0.028, rim: 0 };
/** A raised dark glass that still reads against a dark theme's background. */
export const DUSK_GOO: GooLook = { body: 0.12, rim: 0.16 };

export interface GooSurface {
	resize(width: number, height: number, dpr: number): void;
	draw(blobs: readonly GooBlob[], anchor: GooAnchor, look?: GooLook): void;
	dispose(): void;
}

/** A busy crew outgrows 32 bodies (a thrown, working bot alone draws a dozen); 64 stays well within WebGL 2's guaranteed 224 fragment uniform vectors. */
export const MAX_BLOBS = 64;
/** How far apart two bodies start to flow into each other. */
const MELT = 16;
/** Reach of the melt, the shoulders and the drop shadow around the bodies, in CSS pixels. */
const BLEED = 44;

const VERTEX = `#version 300 es
in vec2 aPosition;
void main() { gl_Position = vec4(aPosition, 0.0, 1.0); }
`;

const FRAGMENT = `#version 300 es
precision highp float;
uniform vec2 uSize;
uniform float uDpr;
uniform vec4 uBox[${MAX_BLOBS}];
uniform vec4 uShape[${MAX_BLOBS}];
uniform int uCount;
uniform vec3 uAnchor;
uniform vec2 uLook;
out vec4 color;

float roundBox(vec2 p, vec2 half_size, float radius) {
	vec2 q = abs(p) - half_size + radius;
	return length(max(q, 0.0)) + min(max(q.x, q.y), 0.0) - radius;
}

float cross2(vec2 a, vec2 b) {
	return a.x * b.y - a.y * b.x;
}

float taperedStroke(vec2 p, vec2 a, vec2 b, float ra, float rb) {
	p -= a;
	b -= a;
	float h = dot(b, b);
	if (h < 1e-3) return length(p) - max(ra, rb);
	vec2 q = vec2(dot(p, vec2(b.y, -b.x)), dot(p, b)) / h;
	q.x = abs(q.x);
	float lean = ra - rb;
	vec2 c = vec2(sqrt(max(h - lean * lean, 1e-3)), lean);
	float k = cross2(c, q);
	float n = dot(q, q);
	if (k < 0.0) return sqrt(h * n) - ra;
	if (k > c.x) return sqrt(h * (n + 1.0 - 2.0 * q.y)) - rb;
	return dot(c, q) - ra;
}

float smoothUnion(float a, float b, float k) {
	float h = clamp(0.5 + 0.5 * (b - a) / k, 0.0, 1.0);
	return mix(b, a, h) - k * h * (1.0 - h);
}

// Distance from the den's edge, negative beyond it.
float inside(vec2 p) {
	return dot(p, uAnchor.xy) + uAnchor.z;
}

// shoulders: the den melts into its edge. A shadow pass leaves that out, or the whole edge would
// darken. Beyond the edge the den draws nothing: another panel may sit there.
float field(vec2 p, bool shoulders) {
	bool beyond = inside(p) < 0.0;
	float d = 1e5;
	for (int i = 0; i < ${MAX_BLOBS}; i++) {
		if (i >= uCount) break;
		vec4 box = uBox[i];
		vec4 shape = uShape[i];
		bool anchored = shape.y > 0.5;
		if (anchored && beyond) continue;
		float s;
		if (shape.z > 0.5) {
			s = taperedStroke(p, box.xy, box.zw, shape.x, shape.w);
		} else {
			s = roundBox(p - box.xy, box.zw, min(shape.x, min(box.z, box.w)));
			if (shoulders && anchored) s = smoothUnion(s, inside(p) + 3.0, 11.0);
		}
		d = smoothUnion(d, s, ${MELT.toFixed(1)});
	}
	return d;
}

void main() {
	vec2 p = vec2(gl_FragCoord.x, uSize.y * uDpr - gl_FragCoord.y) / uDpr;
	float d = field(p, true);
	if (d > 24.0) discard;
	float alpha = clamp(0.5 - d * uDpr, 0.0, 1.0);
	float shadow = 0.0;
	if (alpha < 1.0) {
		float below = field(p - vec2(0.0, 6.0), false);
		shadow = (1.0 - smoothstep(-8.0, 18.0, below)) * 0.34;
	}
	vec3 body = vec3(0.0);
	if (alpha > 0.0) {
		float shade = uLook.x;
		shade += smoothstep(-1.8, 0.0, d) * uLook.y;
		if (d > -7.0) {
			vec2 e = vec2(0.75, 0.0);
			vec2 normal = normalize(vec2(
				field(p + e.xy, true) - field(p - e.xy, true),
				field(p + e.yx, true) - field(p - e.yx, true)
			) + 1e-5);
			float light = clamp(dot(normal, normalize(vec2(-0.5, -1.0))), 0.0, 1.0);
			shade += smoothstep(-7.0, 0.0, d) * (0.045 + 0.13 * light);
			shade += pow(light, 12.0) * smoothstep(-3.5, -0.6, d) * 0.42;
		}
		body = vec3(shade, shade, shade * 1.1);
	}
	float coverage = alpha + shadow * (1.0 - alpha);
	if (coverage <= 0.002) discard;
	color = vec4(body * alpha, coverage);
}
`;

/** The edge as a plane `dot(p, n) + c`: positive on the screen side. */
function anchorPlane(anchor: GooAnchor): [number, number, number] {
	return anchor.edge === "top" ? [0, 1, -anchor.at] : [-1, 0, anchor.at];
}

interface Box {
	left: number;
	top: number;
	right: number;
	bottom: number;
}

function blobBox(blob: GooBlob): Box {
	if (!blob.to)
		return {
			left: blob.x - blob.hw - BLEED,
			top: blob.y - blob.hh - BLEED,
			right: blob.x + blob.hw + BLEED,
			bottom: blob.y + blob.hh + BLEED,
		};
	const reach = Math.max(blob.radius, blob.to.radius) + BLEED;
	return {
		left: Math.min(blob.x, blob.to.x) - reach,
		top: Math.min(blob.y, blob.to.y) - reach,
		right: Math.max(blob.x, blob.to.x) + reach,
		bottom: Math.max(blob.y, blob.to.y) + reach,
	};
}

function toRect(box: Box): {
	x: number;
	y: number;
	width: number;
	height: number;
} {
	return {
		x: box.left,
		y: box.top,
		width: box.right - box.left,
		height: box.bottom - box.top,
	};
}

/**
 * The regions the bodies can touch, so the shader only runs there: one per group of bodies close
 * enough to melt together, so a den and a bot perched far away don't light up the screen between.
 */
export function gooClusters(
	blobs: readonly GooBlob[],
): { x: number; y: number; width: number; height: number }[] {
	const boxes = blobs.map(blobBox);
	let merged = true;
	while (merged) {
		merged = false;
		for (let i = 0; i < boxes.length && !merged; i++) {
			for (let j = i + 1; j < boxes.length; j++) {
				const a = boxes[i];
				const b = boxes[j];
				if (
					a.left <= b.right &&
					b.left <= a.right &&
					a.top <= b.bottom &&
					b.top <= a.bottom
				) {
					boxes[i] = {
						left: Math.min(a.left, b.left),
						top: Math.min(a.top, b.top),
						right: Math.max(a.right, b.right),
						bottom: Math.max(a.bottom, b.bottom),
					};
					boxes.splice(j, 1);
					merged = true;
					break;
				}
			}
		}
	}
	return boxes.map(toRect);
}

/** Everything the bodies can touch. */
export function gooBounds(
	blobs: readonly GooBlob[],
): { x: number; y: number; width: number; height: number } | null {
	const clusters = gooClusters(blobs);
	if (clusters.length === 0) return null;
	const left = Math.min(...clusters.map((rect) => rect.x));
	const top = Math.min(...clusters.map((rect) => rect.y));
	const right = Math.max(...clusters.map((rect) => rect.x + rect.width));
	const bottom = Math.max(...clusters.map((rect) => rect.y + rect.height));
	return { x: left, y: top, width: right - left, height: bottom - top };
}

function compile(
	gl: WebGL2RenderingContext,
	type: number,
	source: string,
): WebGLShader {
	const shader = gl.createShader(type);
	if (!shader) throw new Error("WebGL could not create a goo shader");
	gl.shaderSource(shader, source);
	gl.compileShader(shader);
	if (!gl.getShaderParameter(shader, gl.COMPILE_STATUS)) {
		const log = gl.getShaderInfoLog(shader);
		gl.deleteShader(shader);
		throw new Error(`The goo shader did not compile: ${log}`);
	}
	return shader;
}

class WebGlGoo implements GooSurface {
	private readonly boxes = new Float32Array(MAX_BLOBS * 4);
	private readonly shapes = new Float32Array(MAX_BLOBS * 4);
	private width = 0;
	private height = 0;
	private dpr = 1;

	constructor(
		private readonly canvas: HTMLCanvasElement,
		private readonly gl: WebGL2RenderingContext,
		private readonly program: WebGLProgram,
		private readonly buffer: WebGLBuffer,
		private readonly uniforms: {
			size: WebGLUniformLocation | null;
			dpr: WebGLUniformLocation | null;
			box: WebGLUniformLocation | null;
			shape: WebGLUniformLocation | null;
			count: WebGLUniformLocation | null;
			anchor: WebGLUniformLocation | null;
			look: WebGLUniformLocation | null;
		},
	) {}

	resize(width: number, height: number, dpr: number): void {
		this.width = width;
		this.height = height;
		this.dpr = dpr;
		this.canvas.width = Math.max(1, Math.round(width * dpr));
		this.canvas.height = Math.max(1, Math.round(height * dpr));
		this.canvas.style.width = `${width}px`;
		this.canvas.style.height = `${height}px`;
		this.gl.viewport(0, 0, this.canvas.width, this.canvas.height);
	}

	draw(
		blobs: readonly GooBlob[],
		anchor: GooAnchor,
		look: GooLook = INK_GOO,
	): void {
		const { gl } = this;
		const count = Math.min(blobs.length, MAX_BLOBS);
		for (let index = 0; index < count; index++) {
			const blob = blobs[index];
			const to = blob.to;
			this.boxes.set(
				to ? [blob.x, blob.y, to.x, to.y] : [blob.x, blob.y, blob.hw, blob.hh],
				index * 4,
			);
			this.shapes.set(
				[blob.radius, blob.anchored ? 1 : 0, to ? 1 : 0, to?.radius ?? 0],
				index * 4,
			);
		}
		gl.disable(gl.SCISSOR_TEST);
		gl.clearColor(0, 0, 0, 0);
		gl.clear(gl.COLOR_BUFFER_BIT);
		if (count === 0) return;
		gl.useProgram(this.program);
		gl.uniform2f(this.uniforms.size, this.width, this.height);
		gl.uniform1f(this.uniforms.dpr, this.dpr);
		gl.uniform4fv(this.uniforms.box, this.boxes);
		gl.uniform4fv(this.uniforms.shape, this.shapes);
		gl.uniform1i(this.uniforms.count, count);
		gl.uniform3fv(this.uniforms.anchor, anchorPlane(anchor));
		gl.uniform2f(this.uniforms.look, look.body, look.rim);
		gl.bindBuffer(gl.ARRAY_BUFFER, this.buffer);
		gl.enable(gl.SCISSOR_TEST);
		for (const rect of gooClusters(blobs.slice(0, count))) {
			const x = Math.max(0, Math.floor(rect.x * this.dpr));
			const top = Math.max(0, Math.floor(rect.y * this.dpr));
			const right = Math.min(
				this.canvas.width,
				Math.ceil((rect.x + rect.width) * this.dpr),
			);
			const bottom = Math.min(
				this.canvas.height,
				Math.ceil((rect.y + rect.height) * this.dpr),
			);
			if (right <= x || bottom <= top) continue;
			gl.scissor(x, this.canvas.height - bottom, right - x, bottom - top);
			gl.drawArrays(gl.TRIANGLES, 0, 3);
		}
	}

	/** Keeps the context: the canvas hands the same one to the next renderer. */
	dispose(): void {
		this.gl.deleteProgram(this.program);
		this.gl.deleteBuffer(this.buffer);
	}
}

/** Draws the goo on the canvas with WebGL 2; null where WebGL 2 is unavailable. */
export function createWebGlGoo(canvas: HTMLCanvasElement): GooSurface | null {
	const gl = canvas.getContext("webgl2", {
		alpha: true,
		premultipliedAlpha: true,
		antialias: false,
		depth: false,
		stencil: false,
		powerPreference: "low-power",
	});
	if (!gl) return null;
	try {
		const program = gl.createProgram();
		const buffer = gl.createBuffer();
		if (!program || !buffer) return null;
		gl.attachShader(program, compile(gl, gl.VERTEX_SHADER, VERTEX));
		gl.attachShader(program, compile(gl, gl.FRAGMENT_SHADER, FRAGMENT));
		gl.linkProgram(program);
		if (!gl.getProgramParameter(program, gl.LINK_STATUS)) {
			console.warn(
				"[BotGoo] The goo program did not link:",
				gl.getProgramInfoLog(program),
			);
			return null;
		}
		gl.bindBuffer(gl.ARRAY_BUFFER, buffer);
		gl.bufferData(
			gl.ARRAY_BUFFER,
			new Float32Array([-1, -1, 3, -1, -1, 3]),
			gl.STATIC_DRAW,
		);
		const position = gl.getAttribLocation(program, "aPosition");
		gl.enableVertexAttribArray(position);
		gl.vertexAttribPointer(position, 2, gl.FLOAT, false, 0, 0);
		gl.enable(gl.BLEND);
		gl.blendFunc(gl.ONE, gl.ONE_MINUS_SRC_ALPHA);
		return new WebGlGoo(canvas, gl, program, buffer, {
			size: gl.getUniformLocation(program, "uSize"),
			dpr: gl.getUniformLocation(program, "uDpr"),
			box: gl.getUniformLocation(program, "uBox"),
			shape: gl.getUniformLocation(program, "uShape"),
			count: gl.getUniformLocation(program, "uCount"),
			anchor: gl.getUniformLocation(program, "uAnchor"),
			look: gl.getUniformLocation(program, "uLook"),
		});
	} catch (error) {
		console.warn("[BotGoo] Goo falls back to plain shapes:", error);
		return null;
	}
}

/** Plain rounded shapes without melting, where WebGL 2 is missing. */
export function createCssGoo(host: HTMLElement): GooSurface {
	const nodes: HTMLDivElement[] = [];
	return {
		resize: () => undefined,
		draw: (blobs, anchor, look = INK_GOO) => {
			const grey = Math.round(look.body * 255);
			while (nodes.length < blobs.length) {
				const node = document.createElement("div");
				node.style.position = "absolute";
				node.style.background = "#09090b";
				node.style.boxShadow = look.rim
					? `inset 0 0 0 1px rgb(255 255 255 / ${look.rim})`
					: "none";
				node.style.pointerEvents = "none";
				host.appendChild(node);
				nodes.push(node);
			}
			nodes.forEach((node, index) => {
				const blob = blobs[index];
				node.style.display = blob && !blob.to ? "block" : "none";
				node.style.background = `rgb(${grey} ${grey} ${Math.min(255, grey + 3)})`;
				if (!blob || blob.to) return;
				const right =
					blob.anchored && anchor.edge === "right"
						? Math.min(blob.x + blob.hw, anchor.at)
						: blob.x + blob.hw;
				node.style.left = `${blob.x - blob.hw}px`;
				node.style.top = `${blob.y - blob.hh}px`;
				node.style.width = `${right - (blob.x - blob.hw)}px`;
				node.style.height = `${blob.hh * 2}px`;
				node.style.borderRadius = `${Math.min(blob.radius, blob.hw, blob.hh)}px`;
			});
		},
		dispose: () => {
			for (const node of nodes) node.remove();
			nodes.length = 0;
		},
	};
}
