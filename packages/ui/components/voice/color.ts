// Convert a CSS color string (hex / rgb / rgba) to an rgba() string with the
// given alpha. Named/hsl colors are returned unchanged (alpha not applied).
export function withAlpha(color: string, alpha: number): string {
	const a = Math.max(0, Math.min(1, alpha));
	if (color.startsWith("#")) {
		let hex = color.slice(1);
		if (hex.length === 3) {
			hex = hex
				.split("")
				.map((c) => c + c)
				.join("");
		}
		const num = Number.parseInt(hex, 16);
		if (Number.isNaN(num)) return color;
		const r = (num >> 16) & 255;
		const g = (num >> 8) & 255;
		const b = num & 255;
		return `rgba(${r}, ${g}, ${b}, ${a})`;
	}
	if (color.startsWith("rgba(")) return color;
	if (color.startsWith("rgb(")) {
		return color.replace("rgb(", "rgba(").replace(")", `, ${a})`);
	}
	return color;
}

// [r, g, b] 0..255 from a hex color, violet fallback for non-hex input.
function hexToRgb255(color: string): [number, number, number] {
	if (color.startsWith("#")) {
		let hex = color.slice(1);
		if (hex.length === 3) {
			hex = hex
				.split("")
				.map((c) => c + c)
				.join("");
		}
		const num = Number.parseInt(hex, 16);
		if (!Number.isNaN(num)) {
			return [(num >> 16) & 255, (num >> 8) & 255, num & 255];
		}
	}
	return [139, 92, 246];
}

// Blend a color toward white by `amount` (0..1) and return an rgba() string.
export function lighten(color: string, amount: number, alpha = 1): string {
	const [r, g, b] = hexToRgb255(color);
	const t = Math.max(0, Math.min(1, amount));
	const mix = (c: number) => Math.round(c + (255 - c) * t);
	return `rgba(${mix(r)}, ${mix(g)}, ${mix(b)}, ${Math.max(0, Math.min(1, alpha))})`;
}

// Normalized [r, g, b] in 0..1 from a hex color, for WebGL uniforms.
// Falls back to violet for non-hex inputs.
export function hexToRgbNorm(color: string): [number, number, number] {
	if (color.startsWith("#")) {
		let hex = color.slice(1);
		if (hex.length === 3) {
			hex = hex
				.split("")
				.map((c) => c + c)
				.join("");
		}
		const num = Number.parseInt(hex, 16);
		if (!Number.isNaN(num)) {
			return [
				((num >> 16) & 255) / 255,
				((num >> 8) & 255) / 255,
				(num & 255) / 255,
			];
		}
	}
	return [0.545, 0.361, 0.965];
}
