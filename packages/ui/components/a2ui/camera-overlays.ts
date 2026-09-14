/** Coordinates always refer to the upright, unmirrored captured image, in [0, 1]. */
export interface CameraOverlay {
	id: string;
	type: "box" | "text" | "point" | "polygon" | "blur" | "dim";
	x?: number;
	y?: number;
	width?: number;
	height?: number;
	points?: [number, number][];
	text?: string;
	color?: string;
	background?: string;
	confidence?: number;
	opacity?: number;
	fontSize?: number;
	zIndex?: number;
}

export interface CameraEffects {
	grayscale?: number;
	sepia?: number;
	blur?: number;
	brightness?: number;
	contrast?: number;
}

export interface CameraOverlayUpdate {
	sessionId: string;
	frameId: string;
	overlays: CameraOverlay[];
	effects?: CameraEffects;
	/** Time before these annotations disappear. Defaults to 5 seconds. */
	ttlMs?: number;
}

export class CameraError extends Error {
	constructor(
		public readonly code: string,
		message: string,
	) {
		super(message);
		this.name = "CameraError";
	}
}

const record = (value: unknown): value is Record<string, unknown> =>
	Boolean(value && typeof value === "object" && !Array.isArray(value));

function number(
	value: unknown,
	min: number,
	max: number,
	field: string,
): number {
	if (
		typeof value !== "number" ||
		!Number.isFinite(value) ||
		value < min ||
		value > max
	)
		throw new CameraError(
			"invalid_arguments",
			`${field} must be between ${min} and ${max}.`,
		);
	return value;
}

function color(value: unknown): string | undefined {
	if (value === undefined) return undefined;
	// Exclude URLs and arbitrary CSS. Colors are data, never an asset loading mechanism.
	if (
		typeof value !== "string" ||
		!/^(#[0-9a-f]{3,8}|[a-z]{1,24}|rgba?\([\d.,%\s]+\)|hsla?\([\d.,%\s]+\))$/i.test(
			value,
		)
	)
		throw new CameraError(
			"invalid_arguments",
			"Overlay colors must be CSS colors without URLs.",
		);
	return value;
}

export function parseCameraEffects(value: unknown): CameraEffects {
	if (!record(value))
		throw new CameraError("invalid_arguments", "Effects must be an object.");
	const effects: CameraEffects = {};
	for (const key of [
		"grayscale",
		"sepia",
		"blur",
		"brightness",
		"contrast",
	] as const) {
		if (value[key] !== undefined)
			effects[key] = number(
				value[key],
				0,
				key === "blur"
					? 24
					: key === "brightness" || key === "contrast"
						? 3
						: 1,
				key,
			);
	}
	return effects;
}

export function cameraFilter(effects: CameraEffects): string {
	return (
		Object.entries(effects)
			.map(([key, value]) => `${key}(${value}${key === "blur" ? "px" : ""})`)
			.join(" ") || "none"
	);
}

export function parseCameraOverlayUpdate(value: unknown): CameraOverlayUpdate {
	if (
		!record(value) ||
		typeof value.sessionId !== "string" ||
		!value.sessionId ||
		typeof value.frameId !== "string" ||
		!value.frameId
	)
		throw new CameraError(
			"invalid_arguments",
			"Overlay updates require sessionId and frameId.",
		);
	if (!Array.isArray(value.overlays) || value.overlays.length > 200)
		throw new CameraError(
			"invalid_arguments",
			"Overlays must be an array of at most 200 items.",
		);
	const ids = new Set<string>();
	const overlays = value.overlays
		.map((item): CameraOverlay => {
			if (
				!record(item) ||
				typeof item.id !== "string" ||
				!item.id ||
				item.id.length > 128 ||
				ids.has(item.id)
			)
				throw new CameraError(
					"invalid_arguments",
					"Every overlay needs a unique id of at most 128 characters.",
				);
			ids.add(item.id);
			if (
				!["box", "text", "point", "polygon", "blur", "dim"].includes(
					String(item.type),
				)
			)
				throw new CameraError(
					"invalid_arguments",
					"Unknown camera overlay type.",
				);
			const overlay: CameraOverlay = {
				id: item.id,
				type: item.type as CameraOverlay["type"],
			};
			if (overlay.type === "polygon") {
				if (
					!Array.isArray(item.points) ||
					item.points.length < 3 ||
					item.points.length > 128
				)
					throw new CameraError(
						"invalid_arguments",
						"Polygons need 3 to 128 points.",
					);
				overlay.points = item.points.map((point) => {
					if (!Array.isArray(point) || point.length !== 2)
						throw new CameraError(
							"invalid_arguments",
							"Each point needs x and y.",
						);
					return [number(point[0], 0, 1, "x"), number(point[1], 0, 1, "y")];
				});
			} else {
				overlay.x = number(item.x, 0, 1, "x");
				overlay.y = number(item.y, 0, 1, "y");
				if (["box", "blur", "dim"].includes(overlay.type)) {
					overlay.width = number(item.width, 0, 1 - overlay.x, "width");
					overlay.height = number(item.height, 0, 1 - overlay.y, "height");
				}
			}
			if (item.text !== undefined) {
				if (typeof item.text !== "string" || item.text.length > 2000)
					throw new CameraError(
						"invalid_arguments",
						"Overlay text is limited to 2000 characters.",
					);
				overlay.text = item.text;
			}
			overlay.color = color(item.color);
			overlay.background = color(item.background);
			for (const key of ["confidence", "opacity"] as const)
				if (item[key] !== undefined)
					overlay[key] = number(item[key], 0, 1, key);
			if (item.fontSize !== undefined)
				overlay.fontSize = number(item.fontSize, 8, 72, "fontSize");
			if (item.zIndex !== undefined)
				overlay.zIndex = number(item.zIndex, -100, 100, "zIndex");
			return overlay;
		})
		.sort((a, b) => (a.zIndex ?? 0) - (b.zIndex ?? 0));
	return {
		sessionId: value.sessionId,
		frameId: value.frameId,
		overlays,
		effects:
			value.effects === undefined
				? undefined
				: parseCameraEffects(value.effects),
		ttlMs:
			value.ttlMs === undefined
				? 5000
				: number(value.ttlMs, 100, 60000, "ttlMs"),
	};
}

/** The displayed image rectangle, including cover cropping and contain letterboxing. */
export function cameraImageRect(
	container: { width: number; height: number },
	image: { width: number; height: number },
	fit: "contain" | "cover" = "contain",
) {
	if (image.width <= 0 || image.height <= 0)
		return { x: 0, y: 0, ...container };
	const scale = (fit === "cover" ? Math.max : Math.min)(
		container.width / image.width,
		container.height / image.height,
	);
	const width = image.width * scale;
	const height = image.height * scale;
	return {
		x: (container.width - width) / 2,
		y: (container.height - height) / 2,
		width,
		height,
	};
}

export function cameraOverlayX(x: number, width: number, mirrored: boolean) {
	return mirrored ? 1 - x - width : x;
}
