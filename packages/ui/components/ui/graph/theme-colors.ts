interface GraphThemeColors {
	bgRgb: [number, number, number];
	fgRgb: [number, number, number];
	isDark: boolean;
}

let cached: GraphThemeColors | null = null;
let lastThemeKey = "";

const normalizeCtx: CanvasRenderingContext2D | null =
	typeof document !== "undefined"
		? document.createElement("canvas").getContext("2d")
		: null;

function clampByte(value: number): number {
	return Math.max(0, Math.min(255, Math.round(value)));
}

function parseAngle(value: string): number {
	if (value.endsWith("deg")) return Number(value.slice(0, -3));
	if (value.endsWith("grad")) return Number(value.slice(0, -4)) * 0.9;
	if (value.endsWith("rad"))
		return (Number(value.slice(0, -3)) * 180) / Math.PI;
	if (value.endsWith("turn")) return Number(value.slice(0, -4)) * 360;
	return Number(value);
}

function parseUnitInterval(value: string): number {
	if (value.endsWith("%")) return Number(value.slice(0, -1)) / 100;
	return Number(value);
}

function linearToSrgb(value: number): number {
	const clamped = Math.max(0, Math.min(1, value));
	if (clamped <= 0.0031308) return clamped * 12.92;
	return 1.055 * Math.pow(clamped, 1 / 2.4) - 0.055;
}

function parseOklchRgb(value: string): [number, number, number] | null {
	const match = value
		.trim()
		.toLowerCase()
		.match(
			/^oklch\(\s*([^\s/]+)\s+([^\s/]+)\s+([^\s/)]+)(?:\s*\/\s*[^)]+)?\s*\)$/,
		);
	if (!match) return null;

	const lightness = parseUnitInterval(match[1]);
	const chroma = Number(match[2]);
	const hue = parseAngle(match[3]);
	if (![lightness, chroma, hue].every(Number.isFinite)) return null;

	const hueRadians = (hue * Math.PI) / 180;
	const a = chroma * Math.cos(hueRadians);
	const b = chroma * Math.sin(hueRadians);

	const l = Math.pow(lightness + 0.3963377774 * a + 0.2158037573 * b, 3);
	const m = Math.pow(lightness - 0.1055613458 * a - 0.0638541728 * b, 3);
	const s = Math.pow(lightness - 0.0894841775 * a - 1.291485548 * b, 3);

	const r = linearToSrgb(
		4.0767416621 * l - 3.3077115913 * m + 0.2309699292 * s,
	);
	const g = linearToSrgb(
		-1.2684380046 * l + 2.6097574011 * m - 0.3413193965 * s,
	);
	const blue = linearToSrgb(
		-0.0041960863 * l - 0.7034186147 * m + 1.707614701 * s,
	);

	return [clampByte(r * 255), clampByte(g * 255), clampByte(blue * 255)];
}

function parseRgbString(value: string): [number, number, number] | null {
	const match = value.match(/[\d.]+/g);
	if (!match || match.length < 3) return null;
	return [
		clampByte(Number(match[0])),
		clampByte(Number(match[1])),
		clampByte(Number(match[2])),
	];
}

function resolveThemeMode(): "dark" | "light" {
	const root = document.documentElement;
	const dataTheme = root.getAttribute("data-theme");
	if (dataTheme === "dark" || dataTheme === "light") return dataTheme;
	return root.classList.contains("dark") ? "dark" : "light";
}

function getThemeCacheKey(): string {
	const root = document.documentElement;
	const styles = getComputedStyle(root);
	return [
		resolveThemeMode(),
		root.className,
		root.getAttribute("data-theme") ?? "",
		styles.getPropertyValue("--background").trim(),
		styles.getPropertyValue("--foreground").trim(),
	].join("|");
}

function resolveColorRgb(varName: string): [number, number, number] {
	const computed = getComputedStyle(document.documentElement)
		.getPropertyValue(varName)
		.trim();
	if (!computed) return [128, 128, 128];

	const oklch = parseOklchRgb(computed);
	if (oklch) return oklch;

	if (normalizeCtx) {
		normalizeCtx.fillStyle = "#000000";
		normalizeCtx.fillStyle = computed;
		const hex = normalizeCtx.fillStyle;
		if (typeof hex === "string" && hex.startsWith("#") && hex.length >= 7) {
			return [
				Number.parseInt(hex.slice(1, 3), 16),
				Number.parseInt(hex.slice(3, 5), 16),
				Number.parseInt(hex.slice(5, 7), 16),
			];
		}
	}

	const rgb = parseRgbString(computed);
	if (rgb) return rgb;

	return [128, 128, 128];
}

export function getGraphTheme(): GraphThemeColors {
	const themeKey = getThemeCacheKey();
	if (cached && lastThemeKey === themeKey) return cached;

	const bgRgb = resolveColorRgb("--background");
	const fgRgb = resolveColorRgb("--foreground");
	const isDark =
		(0.299 * bgRgb[0] + 0.587 * bgRgb[1] + 0.114 * bgRgb[2]) / 255 < 0.5;

	cached = { bgRgb, fgRgb, isDark };
	lastThemeKey = themeKey;
	return cached;
}

export function invalidateGraphTheme(): void {
	cached = null;
}
