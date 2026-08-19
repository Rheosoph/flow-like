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

// ── FlowPilot activity state ──
// Every one of these is a no-op at 0, which is what unset uniforms are in WebGL. Callers that
// don't drive the orb's state (the hero composer) therefore render exactly the film they did
// before this block existed.
uniform float u_round;    // 0 organic wobble … 1 perfect circle
uniform float u_spin;     // accumulated rotation, radians
uniform float u_teeth;    // cog-rim amplitude
uniform float u_teethN;   // number of cog teeth
uniform float u_sat;      // orbiting satellite bubbles
uniform float u_pop;      // 1 → 0 acknowledge shockwave
uniform float u_spin_mix; // how much the iridescence rides along with u_spin

// ── blob field ──
// A constellation of droplets, positioned from JS every frame and merged into the body with a
// polynomial smooth minimum rather than a hard union — so a droplet necks out of the skin and the
// thread snaps on its own, and two droplets fuse when they meet, with no keyframe for either.
// Same no-op-at-0 contract: presence 0 is what an unset uniform is, and costs nothing.
uniform vec4 u_blob[8];      // xy centre (film space), z radius, w presence
uniform float u_blob_k[8];   // smooth-min radius; 0 is a hard union
uniform float u_blob_uv[8];  // 0 wobble space … 1 screen space
// A switch, not a fade: the body's whole field is pushed away, so it disappears as soon as this
// clears about u_box.y/8. Shrink u_box if you want the body to leave gradually.
uniform float u_body_off;    // 1 removes the body and its satellites, leaving only blobs

// The light branch's saturation punch (see filmL) was tuned against the 72px launcher, and reads
// as hot magenta on a mark several times that size. 0 keeps the launcher's value.
uniform float u_sat_l;

// The mark's nominal half-extent, for the directional grade and the window streak. u_box stops
// meaning "the mark" as soon as the body shrinks and blobs carry the silhouette: dividing by a
// collapsing box saturates the grade into a hard edge straight down the middle of the field.
// 0 uses u_box, which is exactly right for every caller whose body is the whole mark.
uniform vec2 u_grade;

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
mat2 rot(float a) { float c = cos(a), s = sin(a); return mat2(c, -s, s, c); }
float sdRoundBox(vec2 p, vec2 b, float r) {
	vec2 q = abs(p) - b + r;
	return length(max(q, 0.0)) + min(max(q.x, q.y), 0.0) - r;
}
// Polynomial smooth minimum. Exactly min() outside the k band, so a blob far from the body costs
// nothing and can never shrink it — which a naive blend would, everywhere, by k/4.
float smin(float a, float b, float k) {
	if (k <= 0.0) return min(a, b);
	float h = clamp(0.5 + 0.5 * (b - a) / k, 0.0, 1.0);
	return mix(b, a, h) - k * h * (1.0 - h);
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
	float organic = 1.0 - clamp(u_round, 0.0, 1.0);
	vec2 p = uv + (vec2(n1, n2) - 0.5) * 0.44 * wob * calm * organic;
	float radius = u_box.y * mix(1.0, 0.3, u_morph);
	float d = sdRoundBox(p, u_box, radius);
	d -= (fbm(p * 0.55 + vec2(t * 0.16, 1.7)) - 0.5) * 0.11 * wob * calm * organic;
	// traveling surface wave keeps the silhouette visibly alive
	d -= sin(p.x * 1.8 - t * 1.4) * 0.03 * mix(1.0, 0.2, m) * organic;

	// working: a cog rim turned by u_spin. Flat-topped but soft-shouldered so it still reads as
	// a soap film, and gated to a band around the silhouette so the interference pattern inside
	// stays concentric instead of turning into a sunburst.
	vec2 pr = rot(u_spin) * p;
	float cogAng = atan(pr.y, pr.x);
	float cog = clamp(cos(cogAng * max(u_teethN, 1.0)) * 2.6, -1.0, 1.0);
	d -= u_teeth * 0.042 * cog * exp(-d * d / 0.0144);

	// thinking: three satellites locked exactly 120 deg apart, so they can never bunch up —
	// even spacing is structural, not tuned. Depth comes from each shell breathing on its own
	// phase and each satellite swelling as it swings toward the viewer. Shell radii stay clear
	// of the rim at every point of the cycle, so a satellite never merges into the nucleus.
	float dsat = 8.0;
	for (int i = 0; i < 3; i++) {
		float fi = float(i);
		float oa = u_time * 1.15 + fi * 2.0944;
		float rad = 0.63 * (1.0 + 0.0794 * sin(u_time * 0.9 + fi * 2.1));
		vec2 o = vec2(cos(oa), sin(oa)) * rad;
		float depth = 1.0 + 0.32 * sin(u_time * 1.3 + fi * 2.4);
		dsat = min(dsat, length(p - o) - 0.046 * depth);
	}
	d = min(d, dsat + (1.0 - clamp(u_sat, 0.0, 1.0)) * 6.0);
	d += clamp(u_body_off, 0.0, 1.0) * 8.0;

	// blobs: everything the body itself cannot be. Merged with a smooth minimum, so a droplet
	// budding off the rim stretches a real thread that thins and snaps, and two droplets that
	// meet become one. A blob whose destination is a DOM rectangle crosses into uv space — the
	// wobble field baked into p would otherwise land it tens of pixels off its target.
	for (int i = 0; i < 8; i++) {
		vec4 b = u_blob[i];
		if (b.w <= 0.001) continue;
		vec2 bp = mix(p, uv, clamp(u_blob_uv[i], 0.0, 1.0));
		float db = length(bp - b.xy) - max(b.z, 0.0012) + (1.0 - b.w) * 8.0;
		d = smin(d, db, u_blob_k[i]);
	}

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
	// The cog can turn without dragging the iridescence around with it, and vice versa.
	vec2 pf = mix(p, pr, clamp(u_spin_mix, 0.0, 1.0));
	float base = fbm(pf * 1.3 + vec2(t * 0.25, -t * 0.2)) * 1.6 + t * 0.5 + mnear * 0.9;
	// Dispersion. The three fixed hue offsets below are a palette: they slide the rainbow without
	// ever separating it. Real thin film separates because n varies with wavelength, so blue counts
	// more fringes across the same thickness than red — which is a per-channel scale on the depth
	// term, not an offset. Strongest where the film is thinnest, exactly as it is in glass.
	vec3 phase = -d * 5.0 * mix(vec3(1.0), vec3(0.87, 1.0, 1.18), 0.35 + 0.65 * fres) + base;
	vec3 film = 0.5 + 0.5 * cos(6.2831 * (phase * 0.22 + vec3(0.0, 0.21, 0.42)));
	film = mix(film, vec3(0.62, 0.66, 1.0), 0.3);

	// directional grade: cyan crown, violet left, and the app's --primary (warm
	// ember) sweeping the lower-right so the film harmonizes with the brand scheme
	vec2 grade = u_grade.x > 0.0 ? u_grade : u_box;
	float top = clamp(p.y / grade.y, -1.0, 1.0) * 0.5 + 0.5;
	float left = clamp(-p.x / grade.x, -1.0, 1.0) * 0.5 + 0.5;
	vec3 warm = u_primary * 1.15 + vec3(0.05);           // brighten so it reads in the film
	film = mix(film, vec3(0.55, 0.85, 1.0), top * 0.38);
	film = mix(film, vec3(0.62, 0.42, 1.0), left * 0.42);
	film = mix(film, warm, (1.0 - left) * (1.0 - top) * 0.5 + (1.0 - left) * top * 0.22);
	film = mix(film, vec3(1.0, 0.5, 0.85), (1.0 - top) * left * 0.22);
	float boost = 1.0 + 0.5 * u_focus;
	float swirl = fbm(pf * 1.1 + vec2(t * 0.12, -t * 0.09) + n1);
	float swirl2 = fbm(pf * 2.1 - vec2(t * 0.07, t * 0.1) + n2);
	// window-light streak, hugging the upper-left rim away from the text
	vec2 hlp = (p - vec2(-grade.x * 0.55 + sin(t * 0.4) * 0.05, grade.y * 0.8)) * vec2(1.4, 3.0);
	float streak = exp(-dot(hlp, hlp) * 4.5);

	// absorption trough: every colour term below is additive, so brightness climbs monotonically
	// toward the silhouette and the film reads as glowing plastic. Real glass and real soap go
	// dark first — a narrow absorbing band just inside the rim, under the bright core, is what
	// makes the rim read as a surface turning away rather than as a stroke drawn around a shape.
	float band = exp(-abs(d + 0.026) * 46.0) * fill;

	// ————— DARK: iridescent film glowing over black —————
	float rimCoreD = exp(-abs(d) * 80.0) * 1.05 * boost * (1.0 + 1.2 * mnear) * mix(1.0, 0.55, m);
	float rimAuraD = exp(-abs(d) * 16.0) * 0.3 * boost * (1.0 + 0.8 * mnear) * mix(1.0, 0.4, m);
	float glowOutD = exp(-max(d, 0.0) * 8.0) * 0.16 * mix(1.0, 0.45, m);
	vec3 intD = film * (0.10 + 0.16 * swirl + 0.08 * swirl2) * (0.35 + 0.65 * fres);
	intD += vec3(0.05, 0.055, 0.09);
	intD = mix(intD, vec3(0.07, 0.075, 0.115), m * 0.85);
	vec3 colD = intD * fill;
	colD *= 1.0 - band * 0.62;
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
	// 2.45 was tuned against the launcher, where the film is ~72px across and the thin-film hues
	// would otherwise wash out to grey on white. On a mark two or three times that size the same
	// punch reads as hot magenta, so a caller rendering the film large passes its own value.
	float satL = u_sat_l > 0.0 ? u_sat_l : 2.45;
	vec3 filmL = clamp(mix(grayF, film, satL), 0.0, 1.0);   // punchier iridescence
	// tint darkens the white page in interference bands (rainbow), denser at rim
	vec3 tintL = filmL * (0.54 + 0.28 * swirl + 0.22 * swirl2);
	tintL = mix(tintL, filmL * 0.78, fres * 0.62);
	// specular + inner sheen are gated by fill so nothing paints onto the page
	float spec = pow(streak, 1.4) * 1.1 * fill;     // tight window highlight, inside only
	float rimCoreL = exp(-abs(d) * 70.0) * (1.0 + 1.0 * mnear) * mix(1.0, 0.62, m);
	float sheenL = exp(-abs(d) * 15.0) * 0.5 * boost * (1.0 + 0.8 * mnear) * mix(1.0, 0.6, m) * fill;
	vec3 colL = tintL;
	colL *= 1.0 - band * 0.34;
	colL += filmL * sheenL;
	colL += clamp(filmL * 1.3, 0.0, 1.0) * rimCoreL * 0.55;
	colL = mix(colL, vec3(1.0), spec * 0.8);        // white specular hotspot
	colL += filmL * exp(-mdist * 2.5) * 0.34 * u_mstr * fill;
	// open composer (light): wash the interior near-white so input text stays legible…
	vec3 openBody = mix(vec3(0.99, 0.988, 1.0), clamp(filmL * 0.38 + vec3(0.66), 0.0, 1.0), 0.28);
	colL = mix(colL, openBody, m * 0.92);
	// …then lay a THIN saturated iridescent line hugging the inner silhouette so the
	// cool bubble border survives the straighten-out (fill gates it from blooming out)
	float edgeLine = exp(-abs(d) * 80.0) * fill;
	colL = mix(colL, clamp(filmL * 0.95, 0.0, 1.0), edgeLine * m * 0.9);
	// A film rendered large on a white page wants a pale wash with a thin coloured edge, not a fat
	// coloured ring — and how far the caller has already backed satL off is exactly how large it
	// is. At the launcher's default this is zero, so the small film is untouched.
	float relax = clamp((2.45 - satL) / 1.55, 0.0, 1.0);
	colL = mix(colL, vec3(dot(colL, vec3(0.2126, 0.7152, 0.0722))), relax * 0.46);

	// alpha = translucent film body + a crisp thin rim line ONLY — no broad outer
	// halo, so the canvas stays transparent over the heading text behind it
	float rimLineL = exp(-abs(d) * 60.0) * (1.0 + 0.6 * mnear) * mix(1.0, 0.7, m);
	float alphaL = fill * mix(0.1 + 0.5 * fres + 0.1 * swirl + spec + 0.12 * mnear, 0.95, m)
		+ rimLineL * 0.7;

	// acknowledge: a ring launched off the rim, expanding and thinning as u_pop decays
	float popR = u_box.x + (1.0 - u_pop) * 0.50;
	float popRing = exp(-abs(length(p) - popR) * (14.0 + 26.0 * (1.0 - u_pop)))
		* u_pop * u_pop * 1.9;
	colD += film * popRing;
	colL += clamp(filmL * 1.2, 0.0, 1.0) * popRing * 0.75;
	alphaD += popRing * 0.85;
	alphaL += popRing * 0.7;

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
