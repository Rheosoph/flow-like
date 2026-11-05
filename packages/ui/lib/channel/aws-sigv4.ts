// AWS Signature Version 4 primitives on WebCrypto (no SDK dependency), used to presign
// the realtime IoT WebSocket URL.
// Spec: https://docs.aws.amazon.com/IAM/latest/UserGuide/reference_sigv-create-signed-request.html

const encoder = new TextEncoder();

function toBytes(value: string | Uint8Array): Uint8Array {
	return typeof value === "string" ? encoder.encode(value) : value;
}

function hex(buffer: ArrayBuffer | Uint8Array): string {
	const bytes = buffer instanceof Uint8Array ? buffer : new Uint8Array(buffer);
	let out = "";
	for (const byte of bytes) out += byte.toString(16).padStart(2, "0");
	return out;
}

function subtle(): SubtleCrypto {
	const impl = globalThis.crypto?.subtle;
	if (!impl) throw new Error("WebCrypto is unavailable; cannot sign requests.");
	return impl;
}

export async function sha256Hex(data: string | Uint8Array): Promise<string> {
	return hex(await subtle().digest("SHA-256", toBytes(data) as BufferSource));
}

export async function hmacSha256(
	key: string | Uint8Array,
	data: string | Uint8Array,
): Promise<Uint8Array> {
	const cryptoKey = await subtle().importKey(
		"raw",
		toBytes(key) as BufferSource,
		{ name: "HMAC", hash: "SHA-256" },
		false,
		["sign"],
	);
	return new Uint8Array(
		await subtle().sign("HMAC", cryptoKey, toBytes(data) as BufferSource),
	);
}

/** RFC 3986 encoding: only `A-Z a-z 0-9 - _ . ~` stay unescaped. */
export function rfc3986Encode(value: string): string {
	return encodeURIComponent(value).replace(
		/[!'()*]/g,
		(char) => `%${char.charCodeAt(0).toString(16).toUpperCase()}`,
	);
}

/** Query parameters sorted by encoded key, then encoded value. */
export function canonicalQuery(params: URLSearchParams): string {
	const pairs: Array<[string, string]> = [];
	params.forEach((value, key) => {
		pairs.push([rfc3986Encode(key), rfc3986Encode(value)]);
	});
	pairs.sort(([leftKey, leftValue], [rightKey, rightValue]) =>
		leftKey < rightKey
			? -1
			: leftKey > rightKey
				? 1
				: leftValue < rightValue
					? -1
					: leftValue > rightValue
						? 1
						: 0,
	);
	return pairs.map(([key, value]) => `${key}=${value}`).join("&");
}

export function amzDate(date: Date): string {
	return date.toISOString().replace(/[:-]|\.\d{3}/g, "");
}

export async function signingKey(
	secretAccessKey: string,
	dateStamp: string,
	region: string,
	service: string,
): Promise<Uint8Array> {
	const kDate = await hmacSha256(`AWS4${secretAccessKey}`, dateStamp);
	const kRegion = await hmacSha256(kDate, region);
	const kService = await hmacSha256(kRegion, service);
	return hmacSha256(kService, "aws4_request");
}
