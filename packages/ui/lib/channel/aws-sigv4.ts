// AWS Signature Version 4 request signing on WebCrypto (no SDK dependency).
// Spec: https://docs.aws.amazon.com/IAM/latest/UserGuide/reference_sigv-create-signed-request.html

const ALGORITHM = "AWS4-HMAC-SHA256";
const encoder = new TextEncoder();

export interface SigV4Credentials {
	accessKeyId: string;
	secretAccessKey: string;
	sessionToken?: string;
}

export interface SigV4Request {
	method: string;
	url: string | URL;
	/** Headers to sign; `host`, `x-amz-date` and `x-amz-security-token` are added automatically. */
	headers?: Record<string, string>;
	body?: string | Uint8Array;
	service: string;
	region: string;
	/** Signing time; defaults to now. */
	date?: Date;
}

export interface SigV4Signature {
	/** Every header that must travel with the request, `host` excluded (the browser sets it). */
	headers: Record<string, string>;
	canonicalRequest: string;
	stringToSign: string;
	signature: string;
	amzDate: string;
	credentialScope: string;
	signedHeaders: string;
}

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

/** Path with every segment RFC 3986-encoded exactly once (an already-encoded path is not re-encoded). */
export function canonicalUri(pathname: string): string {
	if (!pathname) return "/";
	return pathname
		.split("/")
		.map((segment) => rfc3986Encode(safeDecode(segment)))
		.join("/");
}

function safeDecode(segment: string): string {
	try {
		return decodeURIComponent(segment);
	} catch {
		return segment;
	}
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

function canonicalHeaderValue(value: string): string {
	return value.trim().replace(/\s+/g, " ");
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

export async function signAwsRequest(
	request: SigV4Request,
	credentials: SigV4Credentials,
): Promise<SigV4Signature> {
	const url = new URL(request.url.toString());
	const date = request.date ?? new Date();
	const amz = amzDate(date);
	const dateStamp = amz.slice(0, 8);
	const payloadHash = await sha256Hex(request.body ?? "");

	const headers = new Map<string, string>();
	for (const [name, value] of Object.entries(request.headers ?? {})) {
		headers.set(name.toLowerCase(), canonicalHeaderValue(value));
	}
	headers.set("host", url.host);
	headers.set("x-amz-date", amz);
	if (credentials.sessionToken) {
		headers.set("x-amz-security-token", credentials.sessionToken);
	}

	const headerNames = [...headers.keys()].sort();
	const canonicalHeaders = headerNames
		.map((name) => `${name}:${headers.get(name)}\n`)
		.join("");
	const signedHeaders = headerNames.join(";");
	const canonicalRequest = [
		request.method.toUpperCase(),
		canonicalUri(url.pathname),
		canonicalQuery(url.searchParams),
		canonicalHeaders,
		signedHeaders,
		payloadHash,
	].join("\n");

	const credentialScope = `${dateStamp}/${request.region}/${request.service}/aws4_request`;
	const stringToSign = [
		ALGORITHM,
		amz,
		credentialScope,
		await sha256Hex(canonicalRequest),
	].join("\n");
	const key = await signingKey(
		credentials.secretAccessKey,
		dateStamp,
		request.region,
		request.service,
	);
	const signature = hex(await hmacSha256(key, stringToSign));

	const outgoing: Record<string, string> = {};
	for (const name of headerNames) {
		if (name !== "host") outgoing[name] = headers.get(name) as string;
	}
	outgoing.authorization = `${ALGORITHM} Credential=${credentials.accessKeyId}/${credentialScope}, SignedHeaders=${signedHeaders}, Signature=${signature}`;

	return {
		headers: outgoing,
		canonicalRequest,
		stringToSign,
		signature,
		amzDate: amz,
		credentialScope,
		signedHeaders,
	};
}
