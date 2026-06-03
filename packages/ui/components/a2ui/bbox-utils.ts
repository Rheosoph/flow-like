// Smart bounding box parser shared by BoundingBoxOverlay and ImageLabeler.
//
// Accepts the many shapes boxes arrive in from workflows and normalizes them to
// a single canonical representation: { id, x, y, width, height, label?, confidence?, color? }
// where x/y is the top-left corner and width/height the extent.
//
// Supported per-box shapes:
//   - Top-left + size:   { x, y, width, height }      (aliases: w, h, left, top)
//   - Corner coords:     { x1, y1, x2, y2 }           (aliases: xmin/ymin/xmax/ymax, left/top/right/bottom)
//   - Center + size:     { cx, cy, width, height }     (aliases: centerX, centerY)
//   - Object detection:  { x1, y1, x2, y2, score, class_idx, class_name }
//   - Arrays:            [x1, y1, x2, y2]              (treated as corner coords)
// Label aliases:      label, class_name, className, class, name, class_idx
// Confidence aliases: confidence, score, conf
//
// The whole payload may also be a JSON string or an object that wraps the array
// under a `boxes` key.

export interface NormalizedBox {
	id: string;
	x: number;
	y: number;
	width: number;
	height: number;
	label?: string;
	confidence?: number;
	color?: string;
}

function toNumber(value: unknown): number | undefined {
	if (typeof value === "number")
		return Number.isFinite(value) ? value : undefined;
	if (typeof value === "string") {
		const parsed = Number.parseFloat(value);
		return Number.isFinite(parsed) ? parsed : undefined;
	}
	return undefined;
}

function pickNumber(
	record: Record<string, unknown>,
	...keys: string[]
): number | undefined {
	for (const key of keys) {
		const value = toNumber(record[key]);
		if (value !== undefined) return value;
	}
	return undefined;
}

function pickString(
	record: Record<string, unknown>,
	...keys: string[]
): string | undefined {
	for (const key of keys) {
		const value = record[key];
		if (typeof value === "string" && value.length > 0) return value;
	}
	return undefined;
}

function normalizeBox(raw: unknown, index: number): NormalizedBox | null {
	// Array form: [x1, y1, x2, y2] treated as corner coordinates.
	if (Array.isArray(raw)) {
		const [a, b, c, d] = raw.map(toNumber);
		if (
			a === undefined ||
			b === undefined ||
			c === undefined ||
			d === undefined
		) {
			return null;
		}
		return {
			id: `box_${index}`,
			x: Math.min(a, c),
			y: Math.min(b, d),
			width: Math.abs(c - a),
			height: Math.abs(d - b),
		};
	}

	if (typeof raw !== "object" || raw === null) return null;
	const record = raw as Record<string, unknown>;

	let x = pickNumber(record, "x", "x1", "xmin", "left");
	let y = pickNumber(record, "y", "y1", "ymin", "top");
	let width = pickNumber(record, "width", "w");
	let height = pickNumber(record, "height", "h");

	const x2 = pickNumber(record, "x2", "xmax", "right");
	const y2 = pickNumber(record, "y2", "ymax", "bottom");
	const cx = pickNumber(record, "cx", "centerX");
	const cy = pickNumber(record, "cy", "centerY");

	// Corner format: derive size from the opposite corner.
	if (
		(width === undefined || height === undefined) &&
		x2 !== undefined &&
		y2 !== undefined
	) {
		const left = x ?? Math.min(x ?? x2, x2);
		const top = y ?? Math.min(y ?? y2, y2);
		if (x !== undefined) {
			width = Math.abs(x2 - x);
			x = Math.min(x, x2);
		} else {
			x = left;
		}
		if (y !== undefined) {
			height = Math.abs(y2 - y);
			y = Math.min(y, y2);
		} else {
			y = top;
		}
	}

	// Center format: shift center to top-left once size is known.
	if (x === undefined && cx !== undefined && width !== undefined)
		x = cx - width / 2;
	if (y === undefined && cy !== undefined && height !== undefined)
		y = cy - height / 2;

	if (
		x === undefined ||
		y === undefined ||
		width === undefined ||
		height === undefined
	) {
		return null;
	}

	const classIdx = pickNumber(record, "class_idx", "classIdx");
	const label =
		pickString(record, "label", "class_name", "className", "class", "name") ??
		(classIdx !== undefined ? `class ${classIdx}` : undefined);

	const confidence = pickNumber(record, "confidence", "score", "conf");
	const color = pickString(record, "color");
	const id = pickString(record, "id") ?? `box_${index}`;

	return {
		id,
		x,
		y,
		width,
		height,
		...(label !== undefined ? { label } : {}),
		...(confidence !== undefined ? { confidence } : {}),
		...(color !== undefined ? { color } : {}),
	};
}

/**
 * Normalize an arbitrary boxes payload into canonical {@link NormalizedBox} objects.
 * Invalid entries are dropped instead of throwing so partial data still renders.
 */
export function normalizeBoxes(input: unknown): NormalizedBox[] {
	let source: unknown = input;

	if (typeof source === "string") {
		try {
			source = JSON.parse(source);
		} catch {
			return [];
		}
	}

	if (!Array.isArray(source)) {
		if (
			source &&
			typeof source === "object" &&
			Array.isArray((source as { boxes?: unknown }).boxes)
		) {
			source = (source as { boxes: unknown[] }).boxes;
		} else {
			return [];
		}
	}

	return (source as unknown[])
		.map((box, index) => normalizeBox(box, index))
		.filter((box): box is NormalizedBox => box !== null);
}

/**
 * Reads a component's `boxes` field which may be stored as a raw array or wrapped in a
 * BoundValue (`literalOptions` / `literalJson`), returning canonical boxes.
 */
export function resolveBoxesField(bound: unknown): NormalizedBox[] {
	let raw: unknown = bound;
	if (bound && typeof bound === "object" && !Array.isArray(bound)) {
		const value = bound as Record<string, unknown>;
		if (Array.isArray(value.literalOptions)) raw = value.literalOptions;
		else if (typeof value.literalJson === "string") raw = value.literalJson;
	}
	return normalizeBoxes(raw);
}
