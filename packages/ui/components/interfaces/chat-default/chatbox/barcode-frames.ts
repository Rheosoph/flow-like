export interface BarcodePoint {
	readonly x: number;
	readonly y: number;
}

export interface DetectedBarcode {
	readonly text: string;
	readonly format: string;
	/** Corners or end points of the code in the pixels of the frame it was found in. */
	readonly points: readonly BarcodePoint[];
}

/** An 8-bit grayscale image, row by row. */
export interface LumaFrame {
	readonly luma: Uint8Array;
	readonly width: number;
	readonly height: number;
}

/** Reads the codes in a frame: fast for live frames, `thorough` for the captured photo. */
export type BarcodeDetect = (
	frame: LumaFrame,
	thorough: boolean,
) => Promise<DetectedBarcode[]>;

/** The size of a `width`×`height` image scaled down to fit `maxEdge`. */
export function fitFrame(
	width: number,
	height: number,
	maxEdge: number,
): { width: number; height: number } {
	const scale = Math.min(1, maxEdge / Math.max(width, height));
	return {
		width: Math.max(1, Math.round(width * scale)),
		height: Math.max(1, Math.round(height * scale)),
	};
}

/** BT.601 luma of RGBA pixels, written into `reuse` when it has the right size. */
export function lumaFromRgba(
	rgba: Uint8ClampedArray | Uint8Array,
	pixels: number,
	reuse?: Uint8Array,
): Uint8Array {
	const luma = reuse?.length === pixels ? reuse : new Uint8Array(pixels);
	for (let pixel = 0, offset = 0; pixel < pixels; pixel++, offset += 4) {
		luma[pixel] =
			(rgba[offset] * 77 + rgba[offset + 1] * 150 + rgba[offset + 2] * 29) >> 8;
	}
	return luma;
}

function barcodeKey(code: DetectedBarcode): string {
	return `${code.format}\u0000${code.text}`;
}

/** Every distinct code across the lists, the first list's reading winning. */
export function mergeBarcodes(
	...lists: readonly (readonly DetectedBarcode[])[]
): DetectedBarcode[] {
	const seen = new Map<string, DetectedBarcode>();
	for (const code of lists.flat()) {
		const key = barcodeKey(code);
		if (!seen.has(key)) seen.set(key, code);
	}
	return [...seen.values()];
}

interface BarcodeSighting {
	readonly seenAt: number;
	/** Where the frame that last read this content showed it; twice for two equal labels. */
	readonly codes: readonly DetectedBarcode[];
}

/** The codes in view, by content, with the time each was last read. */
export type BarcodeTrack = ReadonlyMap<string, BarcodeSighting>;

/**
 * The track after a frame read `codes` at `now`. A code stays for `holdMs` after the last frame
 * that read it, so one blurred frame does not make its outline flicker.
 */
export function trackBarcodes(
	track: BarcodeTrack,
	codes: readonly DetectedBarcode[],
	now: number,
	holdMs: number,
): BarcodeTrack {
	const next = new Map<string, BarcodeSighting>();
	for (const [key, sighting] of track) {
		if (now - sighting.seenAt <= holdMs) next.set(key, sighting);
	}
	const read = new Map<string, DetectedBarcode[]>();
	for (const code of codes) {
		const key = barcodeKey(code);
		read.set(key, [...(read.get(key) ?? []), code]);
	}
	for (const [key, sightings] of read) {
		next.set(key, { seenAt: now, codes: sightings });
	}
	return next;
}

export function trackedBarcodes(track: BarcodeTrack): DetectedBarcode[] {
	return [...track.values()].flatMap((sighting) => sighting.codes);
}

/** The codes as a message block under `heading`; nothing when there are none. */
export function barcodeMessage(
	codes: readonly DetectedBarcode[],
	heading: string,
): string | undefined {
	if (codes.length === 0) return undefined;
	return [
		heading,
		...codes.map((code) => `- ${code.format}: ${code.text}`),
	].join("\n");
}
