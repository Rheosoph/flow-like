// Shared soap-film bubble shader + theme/token helpers.
//
// The iridescent WebGL bubble is the visual signature of the FlowPilot composer on the start page.
// It is reused verbatim by the small round launcher (flowpilot-bubble-button) shown on every other
// page, so both render the exact same film — tune the shader here and both stay 1:1.

export const VERT =
	"attribute vec2 p; void main(){ gl_Position = vec4(p, 0.0, 1.0); }";

export const FRAG = `
precision highp float;
uniform vec2 u_res;
uniform float u_time;
uniform float u_focus;
uniform vec2 u_box;
uniform vec2 u_mouse;
uniform float u_mstr;
uniform float u_morph;
uniform float u_light;
uniform vec3 u_primary;

float hash(vec2 p) {
	return fract(sin(dot(p, vec2(127.1, 311.7))) * 43758.5453123);
}
float noise(vec2 p) {
	vec2 i = floor(p), f = fract(p);
	vec2 u = f * f * (3.0 - 2.0 * f);
	return mix(mix(hash(i), hash(i + vec2(1.0, 0.0)), u.x),
	           mix(hash(i + vec2(0.0, 1.0)), hash(i + vec2(1.0, 1.0)), u.x), u.y);
}
float fbm(vec2 p) {
	float v = 0.0, a = 0.5;
	for (int i = 0; i < 4; i++) {
		v += a * noise(p);
		p = p * 2.03 + vec2(11.3, 7.7);
		a *= 0.5;
	}
	return v;
}
float sdRoundBox(vec2 p, vec2 b, float r) {
	vec2 q = abs(p) - b + r;
	return length(max(q, 0.0)) + min(max(q.x, q.y), 0.0) - r;
}
void main() {
	vec2 uv = (gl_FragCoord.xy * 2.0 - u_res) / u_res.y;
	float t = u_time * 0.3;
	float wob = 1.0 + u_focus * 0.4;

	// morph: 0 = free-floating bubble, 1 = straightened composer rectangle.
	// u_morph may overshoot past 1 (spring), shaping radius only; color mixes clamp.
	float m = clamp(u_morph, 0.0, 1.0);
	float calm = mix(1.0, 0.06, m);

	// long, slow swells — lower frequency + higher amplitude = bigger waves
	float n1 = fbm(uv * 0.28 + vec2(t * 0.12, -t * 0.09));
	float n2 = fbm(uv * 0.38 + vec2(-t * 0.10, t * 0.12) + 5.2);
	vec2 p = uv + (vec2(n1, n2) - 0.5) * 0.44 * wob * calm;
	float radius = u_box.y * mix(1.0, 0.3, u_morph);
	float d = sdRoundBox(p, u_box, radius);
	d -= (fbm(p * 0.55 + vec2(t * 0.16, 1.7)) - 0.5) * 0.11 * wob * calm;
	// traveling surface wave keeps the silhouette visibly alive
	d -= sin(p.x * 1.8 - t * 1.4) * 0.03 * mix(1.0, 0.2, m);

	// cursor physics: the film bulges toward the pointer and ripples around it
	// (only a whisper of it remains in composer mode)
	vec2 mv = p - u_mouse;
	float mdist = length(mv);
	float mnear = exp(-mdist * mdist * 2.0) * u_mstr;
	d -= 0.16 * mnear * mix(1.0, 0.12, m);
	d -= 0.025 * sin(mdist * 9.0 - u_time * 5.0) * exp(-mdist * 1.8) * u_mstr * mix(1.0, 0.15, m);

	float px = 2.5 / u_res.y;
	float fill = smoothstep(px, -px, d);

	// fresnel: a soap film is bright at the silhouette, near-transparent inside
	float depth = clamp(-d / 0.55, 0.0, 1.0);
	float fres = pow(1.0 - depth, 2.6);

	// thin-film interference: hue cycles with distance from the film edge,
	// and the interference pattern shifts under the cursor
	float phase = -d * 5.0 + fbm(p * 1.3 + vec2(t * 0.25, -t * 0.2)) * 1.6 + t * 0.5 + mnear * 0.9;
	vec3 film = 0.5 + 0.5 * cos(6.2831 * (phase * 0.22 + vec3(0.0, 0.21, 0.42)));
	film = mix(film, vec3(0.62, 0.66, 1.0), 0.3);

	// directional grade: cyan crown, violet left, and the app's --primary (warm
	// ember) sweeping the lower-right so the film harmonizes with the brand scheme
	float top = clamp(p.y / u_box.y, -1.0, 1.0) * 0.5 + 0.5;
	float left = clamp(-p.x / u_box.x, -1.0, 1.0) * 0.5 + 0.5;
	vec3 warm = u_primary * 1.15 + vec3(0.05);           // brighten so it reads in the film
	film = mix(film, vec3(0.55, 0.85, 1.0), top * 0.38);
	film = mix(film, vec3(0.62, 0.42, 1.0), left * 0.42);
	film = mix(film, warm, (1.0 - left) * (1.0 - top) * 0.5 + (1.0 - left) * top * 0.22);
	film = mix(film, vec3(1.0, 0.5, 0.85), (1.0 - top) * left * 0.22);
	float boost = 1.0 + 0.5 * u_focus;
	float swirl = fbm(p * 1.1 + vec2(t * 0.12, -t * 0.09) + n1);
	float swirl2 = fbm(p * 2.1 - vec2(t * 0.07, t * 0.1) + n2);
	// window-light streak, hugging the upper-left rim away from the text
	vec2 hlp = (p - vec2(-u_box.x * 0.55 + sin(t * 0.4) * 0.05, u_box.y * 0.8)) * vec2(1.4, 3.0);
	float streak = exp(-dot(hlp, hlp) * 4.5);

	// ————— DARK: iridescent film glowing over black —————
	float rimCoreD = exp(-abs(d) * 80.0) * 1.05 * boost * (1.0 + 1.2 * mnear) * mix(1.0, 0.55, m);
	float rimAuraD = exp(-abs(d) * 16.0) * 0.3 * boost * (1.0 + 0.8 * mnear) * mix(1.0, 0.4, m);
	float glowOutD = exp(-max(d, 0.0) * 8.0) * 0.16 * mix(1.0, 0.45, m);
	vec3 intD = film * (0.10 + 0.16 * swirl + 0.08 * swirl2) * (0.35 + 0.65 * fres);
	intD += vec3(0.05, 0.055, 0.09);
	intD = mix(intD, vec3(0.07, 0.075, 0.115), m * 0.85);
	vec3 colD = intD * fill;
	colD += (film * 0.85 + vec3(0.22)) * rimCoreD;
	colD += film * rimAuraD;
	colD += film * glowOutD;
	colD += vec3(0.9, 0.95, 1.0) * streak * 0.3 * mix(1.0, 0.25, m) * fill;
	colD += film * exp(-mdist * 2.5) * 0.3 * u_mstr * fill * mix(1.0, 0.25, m);
	float alphaD = fill * mix(0.42 + 0.5 * fres + streak * 0.3 + 0.15 * mnear, 0.93, m)
		+ rimCoreD + rimAuraD * 0.8 + glowOutD * 0.7;

	// ————— LIGHT: translucent iridescent soap film over the light page —————
	// punch saturation so thin-film hues read as colour when composited on white
	vec3 grayF = vec3(dot(film, vec3(0.3333)));
	vec3 filmL = clamp(mix(grayF, film, 2.45), 0.0, 1.0);   // punchier iridescence
	// tint darkens the white page in interference bands (rainbow), denser at rim
	vec3 tintL = filmL * (0.54 + 0.28 * swirl + 0.22 * swirl2);
	tintL = mix(tintL, filmL * 0.78, fres * 0.62);
	// specular + inner sheen are gated by fill so nothing paints onto the page
	float spec = pow(streak, 1.4) * 1.1 * fill;     // tight window highlight, inside only
	float rimCoreL = exp(-abs(d) * 70.0) * (1.0 + 1.0 * mnear) * mix(1.0, 0.62, m);
	float sheenL = exp(-abs(d) * 15.0) * 0.5 * boost * (1.0 + 0.8 * mnear) * mix(1.0, 0.6, m) * fill;
	vec3 colL = tintL;
	colL += filmL * sheenL;
	colL += clamp(filmL * 1.3, 0.0, 1.0) * rimCoreL * 0.55;
	colL = mix(colL, vec3(1.0), spec * 0.8);        // white specular hotspot
	colL += filmL * exp(-mdist * 2.5) * 0.34 * u_mstr * fill;
	// open composer keeps a soft tint (was near-white) so it still reads coloured,
	// while staying light enough for legible input text
	colL = mix(colL, mix(vec3(0.985, 0.982, 0.997), filmL * 0.5 + vec3(0.6), 0.5), m * 0.9);
	// alpha = translucent film body + a crisp thin rim line ONLY — no broad outer
	// halo, so the canvas stays transparent over the heading text behind it
	float rimLineL = exp(-abs(d) * 60.0) * (1.0 + 0.6 * mnear) * mix(1.0, 0.7, m);
	float alphaL = fill * mix(0.1 + 0.5 * fres + 0.1 * swirl + spec + 0.12 * mnear, 0.95, m)
		+ rimLineL * 0.7;

	vec3 col = mix(colD, colL, u_light);
	float alpha = mix(alphaD, alphaL, u_light);
	// premultiplied output: color is multiplied by alpha so nothing can leak where
	// alpha is 0 (fixes a colored/white halo on hardware WebGL that mishandles
	// straight-alpha compositing, e.g. macOS WKWebView / Tauri)
	float a = clamp(alpha, 0.0, 1.0);
	gl_FragColor = vec4(clamp(col, 0.0, 1.0) * a, a);
}`;

// Theme detection reads the actual --background token luminance, so it stays
// correct no matter how the app signals light/dark (.dark class, data-theme,
// color-scheme…) and is immune to hydration timing. A single reused hidden probe.
let themeProbe: HTMLSpanElement | null = null;
export function themeIsLight() {
	if (typeof document === "undefined") return true;
	if (!themeProbe) {
		themeProbe = document.createElement("span");
		themeProbe.setAttribute("aria-hidden", "true");
		themeProbe.style.cssText =
			"position:fixed;width:0;height:0;pointer-events:none;background-color:var(--background)";
	}
	if (!themeProbe.isConnected) document.body.appendChild(themeProbe);
	const s = getComputedStyle(themeProbe).backgroundColor.trim();
	let m = s.match(/^okl(?:ch|ab)\(\s*([\d.]+)/i);
	if (m) return +m[1] > 0.5;
	m = s.match(/^rgba?\(\s*([\d.]+)[,\s]+([\d.]+)[,\s]+([\d.]+)/i);
	if (m) return (0.299 * +m[1] + 0.587 * +m[2] + 0.114 * +m[3]) / 255 > 0.5;
	return true;
}

// resolve any CSS color token (oklch/rgb/…) to sRGB [0..255] via a 1px canvas,
// so the bubble + verb gradient can pull the app's --primary from the theme
let tokenCanvas: CanvasRenderingContext2D | null = null;
export function readTokenRGB(
	cssVar: string,
	fallback: readonly [number, number, number],
): [number, number, number] {
	if (typeof document === "undefined") return [...fallback];
	if (!tokenCanvas) {
		const c = document.createElement("canvas");
		c.width = c.height = 1;
		tokenCanvas = c.getContext("2d");
	}
	if (!tokenCanvas) return [...fallback];
	const el = document.createElement("span");
	el.style.cssText = `position:fixed;width:0;height:0;color:var(${cssVar})`;
	document.body.appendChild(el);
	const color = getComputedStyle(el).color.trim();
	el.remove();
	try {
		tokenCanvas.fillStyle = "#000";
		tokenCanvas.fillStyle = color;
		tokenCanvas.fillRect(0, 0, 1, 1);
		const d = tokenCanvas.getImageData(0, 0, 1, 1).data;
		if (d[0] === 0 && d[1] === 0 && d[2] === 0) return [...fallback];
		return [d[0], d[1], d[2]];
	} catch {
		return [...fallback];
	}
}
