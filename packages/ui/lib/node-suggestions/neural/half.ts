const scratch = new Float32Array(1);
const scratchBits = new Uint32Array(scratch.buffer);

/** IEEE 754 binary16 bits of `value`, rounded to nearest even. */
export function toHalfBits(value: number): number {
	scratch[0] = value;
	const bits = scratchBits[0];
	const sign = (bits >>> 16) & 0x8000;
	const exponent = (bits >>> 23) & 0xff;
	let mantissa = bits & 0x7fffff;
	if (exponent === 0xff) return sign | 0x7c00 | (mantissa ? 0x200 : 0);
	const halfExponent = exponent - 112;
	if (halfExponent >= 0x1f) return sign | 0x7c00;
	if (halfExponent <= 0) {
		if (halfExponent < -10) return sign;
		mantissa |= 0x800000;
		const shift = 14 - halfExponent;
		let half = mantissa >>> shift;
		const rest = mantissa & ((1 << shift) - 1);
		const midpoint = 1 << (shift - 1);
		if (rest > midpoint || (rest === midpoint && half & 1)) half++;
		return sign | half;
	}
	let half = (halfExponent << 10) | (mantissa >>> 13);
	const rest = mantissa & 0x1fff;
	if (rest > 0x1000 || (rest === 0x1000 && half & 1)) half++;
	return sign | half;
}

export function fromHalfBits(bits: number): number {
	const sign = bits & 0x8000 ? -1 : 1;
	const exponent = (bits >>> 10) & 0x1f;
	const mantissa = bits & 0x3ff;
	if (exponent === 0) return sign * mantissa * 2 ** -24;
	if (exponent === 0x1f) {
		return mantissa ? Number.NaN : sign * Number.POSITIVE_INFINITY;
	}
	return sign * (1 + mantissa / 1024) * 2 ** (exponent - 15);
}

/** Rounds every value to binary16 precision, so serializing the array loses nothing. */
export function roundToHalf(values: Float32Array): Float32Array {
	for (let index = 0; index < values.length; index++) {
		values[index] = fromHalfBits(toHalfBits(values[index]));
	}
	return values;
}

function bytesToBase64(bytes: Uint8Array): string {
	let binary = "";
	for (let offset = 0; offset < bytes.length; offset += 0x8000) {
		binary += String.fromCharCode(...bytes.subarray(offset, offset + 0x8000));
	}
	return btoa(binary);
}

function base64ToBytes(text: string): Uint8Array {
	const binary = atob(text);
	const bytes = new Uint8Array(binary.length);
	for (let index = 0; index < binary.length; index++) {
		bytes[index] = binary.charCodeAt(index);
	}
	return bytes;
}

/** Little-endian binary16, base64-encoded. */
export function encodeHalf(values: Float32Array): string {
	const bytes = new Uint8Array(values.length * 2);
	const view = new DataView(bytes.buffer);
	for (let index = 0; index < values.length; index++) {
		view.setUint16(index * 2, toHalfBits(values[index]), true);
	}
	return bytesToBase64(bytes);
}

export function decodeHalf(text: string): Float32Array {
	const bytes = base64ToBytes(text);
	if (bytes.length % 2 !== 0) {
		throw new Error(`decodeHalf: odd byte length ${bytes.length}`);
	}
	const view = new DataView(bytes.buffer);
	const values = new Float32Array(bytes.length / 2);
	for (let index = 0; index < values.length; index++) {
		values[index] = fromHalfBits(view.getUint16(index * 2, true));
	}
	return values;
}
