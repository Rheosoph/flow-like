import {
	type BinaryDecodeOptions,
	type BinaryReading,
	binaryBytes,
	decodeBinary,
	oversizedBinaryReading,
} from "./binary-value";

interface PendingRequest {
	resolve: (reading: BinaryReading) => void;
	reject: (error: Error) => void;
}

interface BinaryDecodeResponse {
	id: number;
	ok: boolean;
	reading?: BinaryReading;
	error?: string;
}

let worker: Worker | null | undefined;
let requestSequence = 0;
const pendingRequests = new Map<number, PendingRequest>();

function failPendingRequests(reason: string): void {
	for (const request of pendingRequests.values()) {
		request.reject(new Error(reason));
	}
	pendingRequests.clear();
}

function getBinaryWorker(): Worker | null {
	if (worker !== undefined) return worker;
	if (typeof Worker === "undefined") {
		worker = null;
		return worker;
	}

	try {
		worker = new Worker(new URL("./binary-value.worker.ts", import.meta.url), {
			type: "module",
		});
		worker.onmessage = (event: MessageEvent<BinaryDecodeResponse>) => {
			const { id, ok, reading, error } = event.data;
			const request = pendingRequests.get(id);
			if (!request) return;
			pendingRequests.delete(id);
			if (ok && reading) request.resolve(reading);
			else request.reject(new Error(error ?? "Binary decode worker failed"));
		};
		worker.onerror = (event) => {
			console.warn(
				"[binary-value] Worker crashed; decoding falls back to the main thread.",
				event,
			);
			failPendingRequests("Binary decode worker crashed");
			worker?.terminate();
			worker = null;
		};
	} catch (error) {
		console.warn(
			"[binary-value] Failed to start worker; decoding on the main thread.",
			error,
		);
		worker = null;
	}

	return worker;
}

async function decodeOffThread(
	bytes: Uint8Array,
	options: BinaryDecodeOptions,
): Promise<BinaryReading> {
	const activeWorker = getBinaryWorker();
	if (!activeWorker) return decodeBinary(bytes, options);

	try {
		return await new Promise<BinaryReading>((resolve, reject) => {
			const id = ++requestSequence;
			pendingRequests.set(id, { resolve, reject });
			try {
				activeWorker.postMessage({ id, bytes, options }, [bytes.buffer]);
			} catch (error) {
				pendingRequests.delete(id);
				reject(error instanceof Error ? error : new Error(String(error)));
			}
		});
	} catch (error) {
		console.warn(
			"[binary-value] Worker request failed; decoding on the main thread.",
			error,
		);
		return decodeBinary(bytes, options);
	}
}

type ReadingCache<T> = WeakMap<object, Map<BinaryDecodeOptions, T>>;

const pendingReadings: ReadingCache<Promise<BinaryReading>> = new WeakMap();
const settledReadings: ReadingCache<BinaryReading> = new WeakMap();

const cached = <T>(cache: ReadingCache<T>, value: object) => {
	let entries = cache.get(value);
	if (!entries) {
		entries = new Map();
		cache.set(value, entries);
	}
	return entries;
};

/** A reading already decoded for this exact cell value, so a remount paints it at once. */
export function peekBinaryReading(
	value: unknown,
	options: BinaryDecodeOptions,
): BinaryReading | undefined {
	if (typeof value !== "object" || value === null) return undefined;
	return (
		settledReadings.get(value)?.get(options) ??
		oversizedBinaryReading(value, options) ??
		undefined
	);
}

/**
 * Decodes a cell's bytes in the background, once per value and options, so a
 * page of rows and the dialog opened from one of them share the work. Resolves
 * null when the value is not bytes.
 */
export function readBinaryValue(
	value: unknown,
	options: BinaryDecodeOptions,
): Promise<BinaryReading | null> {
	if (typeof value !== "object" || value === null) {
		return Promise.resolve(null);
	}
	const known = peekBinaryReading(value, options);
	if (known) return Promise.resolve(known);

	const pending = cached(pendingReadings, value);
	const inFlight = pending.get(options);
	if (inFlight) return inFlight;

	const bytes = binaryBytes(value);
	if (!bytes) return Promise.resolve(null);
	// Transferring detaches the buffer, so only a copy made here may go as is.
	const reading = decodeOffThread(
		Array.isArray(value) ? bytes : bytes.slice(),
		options,
	)
		.then((result) => {
			cached(settledReadings, value).set(options, result);
			return result;
		})
		.finally(() => pending.delete(options));
	pending.set(options, reading);
	return reading;
}
