import { oklchToHex } from "../appearance/appearance-theme";
import type { Palette, RGBA } from "./scene";

const byte = (value: string, alpha = false) => {
	const number = Number.parseFloat(value);
	const scaled = value.endsWith("%")
		? (number / 100) * 255
		: number * (alpha ? 255 : 1);
	return Math.round(Math.max(0, Math.min(255, scaled)));
};

export function canvasColor(value: string): RGBA {
	const oklchAlpha = /^oklch\(([^/]+)\/\s*([\d.]+%?)\s*\)$/i.exec(value.trim());
	if (oklchAlpha) {
		const rgb = canvasColor(`oklch(${oklchAlpha[1].trim()})`);
		const alpha = byte(oklchAlpha[2], true);
		if (!Number.isFinite(alpha))
			throw new Error(`Invalid alpha in canvas color: ${value}`);
		return [rgb[0], rgb[1], rgb[2], alpha];
	}
	const normalized = oklchToHex(value) ?? value.trim();
	const hex = /^#([\da-f]{3,8})$/i.exec(normalized)?.[1];
	if (hex && [3, 4, 6, 8].includes(hex.length)) {
		const full = hex.length < 5 ? [...hex].map((c) => c + c).join("") : hex;
		return [
			Number.parseInt(full.slice(0, 2), 16),
			Number.parseInt(full.slice(2, 4), 16),
			Number.parseInt(full.slice(4, 6), 16),
			full.length === 8 ? Number.parseInt(full.slice(6, 8), 16) : 255,
		];
	}
	const rgb = /^rgba?\((.+)\)$/i.exec(normalized)?.[1];
	if (rgb) {
		const parts = rgb.split(/[,\s/]+/).filter(Boolean);
		if (parts.length === 3 || parts.length === 4) {
			const channels = parts.slice(0, 3).map((p) => byte(p));
			const alpha = parts[3] === undefined ? 255 : byte(parts[3], true);
			if (channels.every(Number.isFinite) && Number.isFinite(alpha))
				return [channels[0], channels[1], channels[2], alpha];
		}
	}
	if (normalized === "transparent") return [0, 0, 0, 0];
	throw new Error(`The egui preview cannot resolve theme color: ${value}`);
}

/** The run, log and lint tints the production components take from these classes. */
function statusPalette(
	textColor: (className: string, fallback: RGBA) => RGBA,
): Pick<
	Palette,
	"success" | "caution" | "warning" | "info" | "remote" | "fatal"
> {
	return {
		success: textColor(
			"text-green-600 dark:text-green-400",
			[34, 197, 94, 255],
		),
		caution: textColor(
			"text-yellow-600 dark:text-yellow-400",
			[234, 179, 8, 255],
		),
		warning: textColor("text-amber-500", [245, 158, 11, 255]),
		info: textColor("text-sky-500", [14, 165, 233, 255]),
		remote: textColor("text-blue-500", [59, 130, 246, 255]),
		fatal: textColor("text-red-800", [153, 27, 27, 255]),
	};
}

export interface CanvasTheme {
	palette: Palette;
	pinColors: Record<string, RGBA>;
	referenceColor: RGBA;
	headerColors: { function: RGBA; testEvent: RGBA };
	rootFontSize: number;
	nodeRadius: number;
	rerouteRadius: number;
	nativeFontCompatible: boolean;
}

/** Resolve theme colors in the host scope, including browser-computed CSS Color 4 values. */
export function readCanvasTheme(host: HTMLElement): CanvasTheme {
	const computed = getComputedStyle(host);
	const rootFontSize = Number.parseFloat(
		getComputedStyle(host.ownerDocument.documentElement).fontSize,
	);
	const firstFont = (
		computed.fontFamily || computed.getPropertyValue("--font-sans")
	)
		.split(",")[0]
		.trim()
		.replace(/^['"]|['"]$/g, "");
	const probe = document.createElement("span");
	probe.style.cssText =
		"position:absolute;visibility:hidden;pointer-events:none";
	host.append(probe);
	let colorContext: CanvasRenderingContext2D | null | undefined;
	const browserColor = (value: string): RGBA => {
		if (colorContext === undefined) {
			const canvas = host.ownerDocument.createElement("canvas");
			canvas.width = 1;
			canvas.height = 1;
			colorContext = canvas.getContext("2d", {
				colorSpace: "srgb",
				willReadFrequently: true,
			});
		}
		if (!colorContext)
			throw new Error(`Cannot convert browser theme color: ${value}`);
		// Unsupported fill styles leave the previous value intact. Two sentinels distinguish that from black.
		colorContext.fillStyle = "#010203";
		colorContext.fillStyle = value;
		const accepted = colorContext.fillStyle;
		colorContext.fillStyle = "#040506";
		colorContext.fillStyle = value;
		if (colorContext.fillStyle !== accepted)
			throw new Error(`Unsupported browser theme color: ${value}`);
		colorContext.clearRect(0, 0, 1, 1);
		colorContext.fillRect(0, 0, 1, 1);
		const channels = colorContext.getImageData(0, 0, 1, 1).data;
		return [channels[0], channels[1], channels[2], channels[3]];
	};
	const read = (token: string, fallback?: RGBA): RGBA => {
		const raw = computed.getPropertyValue(token).trim();
		if (!raw && fallback) return fallback;
		if (!raw) throw new Error(`Missing canvas theme token ${token}`);
		try {
			return canvasColor(raw);
		} catch {
			probe.style.color = "";
			probe.style.color = raw;
			if (!probe.style.color)
				throw new Error(`Invalid canvas theme token ${token}`);
			const resolved = getComputedStyle(probe).color;
			try {
				return canvasColor(resolved);
			} catch {
				return browserColor(resolved);
			}
		}
	};
	const radius = (className: string, fallback: number) => {
		probe.className = className;
		const value = getComputedStyle(probe).borderTopLeftRadius;
		const pixels = value?.endsWith("px")
			? Number.parseFloat(value)
			: Number.NaN;
		return Number.isFinite(pixels) ? Math.max(0, pixels) : fallback;
	};
	// Chrome colors the painter can derive itself; a theme without them still renders.
	const optional = (token: string) =>
		computed.getPropertyValue(token).trim() ? read(token) : undefined;
	const resolved = (value: string): RGBA | undefined => {
		if (!value) return undefined;
		try {
			return canvasColor(value);
		} catch {
			return browserColor(value);
		}
	};
	/** A text color class as the stylesheet resolves it; a class the build dropped falls back. */
	const textColor = (className: string, fallback: RGBA): RGBA => {
		probe.className = "";
		const inherited = getComputedStyle(probe).color;
		probe.className = className;
		const value = getComputedStyle(probe).color;
		return (value !== inherited && resolved(value)) || fallback;
	};
	/** Colors the production node takes from classes, read through the real stylesheet. */
	const styled = (className: string) => {
		probe.className = className;
		const style = getComputedStyle(probe);
		return {
			color: resolved(style.color),
			background: resolved(style.backgroundColor),
		};
	};
	try {
		const node = styled("react-flow__node-default");
		const palette: Palette = {
			background: read("--background"),
			card: read("--card"),
			foreground: read("--foreground"),
			muted: read("--muted-foreground"),
			border: read("--border"),
			primary: read("--primary"),
			tertiary: read("--tertiary"),
			nodeText: node.color,
			nodeBackdrop: node.background,
			input: optional("--input"),
			checkbox: styled("dark:bg-input/30").background,
			primaryForeground: optional("--primary-foreground"),
			mutedBackground: optional("--muted"),
			destructive: optional("--destructive"),
			...statusPalette(textColor),
		};
		const types = [
			"String",
			"Integer",
			"Float",
			"Boolean",
			"Struct",
			"Generic",
			"PathBuf",
			"Byte",
			"Date",
			"HashMap",
			"HashSet",
			"Execution",
			"Geometry",
		];
		const pinColors = Object.fromEntries(
			types.map((type) => [
				type,
				type === "Execution"
					? palette.foreground
					: read(`--pin-${type.toLowerCase()}`, palette.foreground),
			]),
		);
		return {
			palette,
			pinColors,
			referenceColor: read("--pin-fn-ref", palette.tertiary),
			headerColors: {
				function: read("--color-violet-500", [141, 84, 255, 255]),
				testEvent: read("--color-cyan-500", [0, 183, 215, 255]),
			},
			rootFontSize:
				Number.isFinite(rootFontSize) && rootFontSize > 0 ? rootFontSize : 16,
			nodeRadius: radius("rounded-md", 4),
			rerouteRadius: radius("rounded-lg", 6),
			nativeFontCompatible: /^(?:__)?Inter(?:$|[ _-]|Variable)/i.test(
				firstFont,
			),
		};
	} finally {
		probe.remove();
	}
}
