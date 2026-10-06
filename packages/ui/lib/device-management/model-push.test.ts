import { afterEach, beforeEach, describe, expect, test } from "bun:test";
import { readFileSync } from "node:fs";
import type { PreparedModelAsset } from "./artifacts";
import {
	DownloadedFile,
	bitStoreFile,
	modelPushSeams,
	pushFile,
	pushModelAsset,
	pushableAsset,
	rememberPushableAssets,
} from "./model-push";
import {
	type ModelAssetDescriptor,
	type ModelAssetStatus,
	modelJobSchema,
} from "./models";
import type { TunnelModelAssetPush } from "./tunnel-data";

const JOB = "12345678-1234-4234-8234-123456789abc";
const DIGEST = { algorithm: "blake3" as const, hex: "a".repeat(64) };
/** The agent's wire shapes, shared with the protocol crate's tests. */
const fixture = JSON.parse(
	readFileSync(
		new URL(
			"../../../device-protocol/fixtures/models-v1.json",
			import.meta.url,
		),
		"utf8",
	),
);
const retries = modelPushSeams.retryMs;
beforeEach(() => {
	modelPushSeams.retryMs = [0, 0, 0];
});
afterEach(() => {
	modelPushSeams.retryMs = retries;
});

const bytesOf = (size: number) =>
	Uint8Array.from({ length: size }, (_, index) => index % 251);

function memoryFile(bytes: Uint8Array) {
	return {
		size: bytes.length,
		slice: (start: number, end: number) => ({
			arrayBuffer: async () => bytes.slice(start, end).buffer,
		}),
	};
}

/**
 * A device that keeps what it was sent and answers like the agent's push
 * session. `unverified`: it holds `held` bytes it never verified, and, like
 * the agent, verifies only once a chunk reaching the end arrives, a repeated
 * one too.
 */
function fakeDevice(
	size: number,
	options: { held?: number; failOnce?: boolean; unverified?: boolean } = {},
) {
	let held = options.held ?? 0;
	let failOnce = options.failOnce ?? false;
	let verified = !options.unverified;
	const opens: number[] = [];
	const push = async (
		input: TunnelModelAssetPush,
	): Promise<ModelAssetStatus> => {
		opens.push(input.offset);
		if (input.offset > held) throw new Error("offset beyond the device's copy");
		if (input.file) {
			for (let offset = input.offset; offset < size; offset += 100) {
				const end = Math.min(size, offset + 100);
				await input.file.slice(offset, end).arrayBuffer();
				if (failOnce && end > size / 2) {
					failOnce = false;
					throw new Error("tunnel closed");
				}
				held = Math.max(held, end);
				verified = held === size;
				input.onProgress?.(end);
			}
		}
		return held === size && verified
			? { digest: DIGEST, state: "present" }
			: { digest: DIGEST, job_id: JOB, state: "awaiting_push", bytes: held };
	};
	return { push, opens, held: () => held };
}

describe("pushing a model asset", () => {
	test("asks where the device's copy ends and sends only the rest", async () => {
		const bytes = bytesOf(1000);
		const device = fakeDevice(1000, { held: 300 });
		const progress: number[] = [];
		const status = await pushModelAsset({
			jobId: JOB,
			file: memoryFile(bytes),
			push: device.push,
			onProgress: ({ bytes: held, total }) => {
				expect(total).toBe(1000);
				progress.push(held);
			},
		});
		expect(status.state).toBe("present");
		expect(device.opens).toEqual([0, 300]);
		expect(progress[0]).toBe(300);
		expect(progress.at(-1)).toBe(1000);
	});

	test("an interrupted push resumes from where the device's copy ends", async () => {
		const device = fakeDevice(1000, { failOnce: true });
		const status = await pushModelAsset({
			jobId: JOB,
			file: memoryFile(bytesOf(1000)),
			push: device.push,
		});
		expect(status.state).toBe("present");
		expect(device.opens).toEqual([0, 0, 0, 500]);
	});

	test("a device that holds the file already gets no bytes", async () => {
		const device = fakeDevice(10, { held: 10 });
		await pushModelAsset({
			jobId: JOB,
			file: memoryFile(bytesOf(10)),
			push: device.push,
		});
		expect(device.opens).toEqual([0]);
	});

	test("a device that holds every byte unverified gets the last byte again, which makes it verify", async () => {
		const device = fakeDevice(1000, { held: 1000, unverified: true });
		const progress: number[] = [];
		const status = await pushModelAsset({
			jobId: JOB,
			file: memoryFile(bytesOf(1000)),
			push: device.push,
			onProgress: ({ bytes }) => progress.push(bytes),
		});
		expect(status.state).toBe("present");
		expect(device.opens).toEqual([0, 999]);
		expect(progress).toEqual([1000, 1000]);
	});

	test("a device that keeps fetching refuses the push with its state", async () => {
		const push = async (): Promise<ModelAssetStatus> => ({
			digest: DIGEST,
			job_id: JOB,
			state: "fetching",
			source_index: 0,
			bytes: 5,
		});
		await expect(
			pushModelAsset({ jobId: JOB, file: memoryFile(bytesOf(10)), push }),
		).rejects.toThrow("fetching");
	});

	test("cancelling stops the push", async () => {
		const abort = new AbortController();
		abort.abort(new Error("Cancelled by you."));
		const device = fakeDevice(10);
		await expect(
			pushModelAsset({
				jobId: JOB,
				file: memoryFile(bytesOf(10)),
				push: device.push,
				signal: abort.signal,
			}),
		).rejects.toThrow("Cancelled by you.");
		expect(device.opens).toEqual([]);
	});
});

type Answer = (offset: number) => Response;

/** A fetch whose answers are given per call; requests and their Range headers are recorded. */
function fakeFetch(...answers: Answer[]) {
	const asked: { url: string; range: string | null; redirect?: string }[] = [];
	const fetcher = (async (input: RequestInfo | URL, init?: RequestInit) => {
		const range = new Headers(init?.headers).get("range");
		asked.push({ url: String(input), range, redirect: init?.redirect });
		const offset = range ? Number(range.slice(6, -1)) : 0;
		const answer = answers.shift();
		if (!answer) throw new TypeError("no answer left");
		return answer(offset);
	}) as typeof fetch;
	return { fetcher, asked };
}

const whole = (bytes: Uint8Array) => () => new Response(bytes.slice());
const ranged = (bytes: Uint8Array) => (offset: number) =>
	new Response(bytes.slice(offset), {
		status: 206,
		headers: {
			"content-range": `bytes ${offset}-${bytes.length - 1}/${bytes.length}`,
		},
	});
/** Delivers `count` bytes from `offset`, then the connection breaks. */
const broken = (bytes: Uint8Array, count: number) => (offset: number) => {
	let sent = false;
	return new Response(
		new ReadableStream({
			pull(controller) {
				if (sent) return controller.error(new TypeError("connection reset"));
				sent = true;
				controller.enqueue(bytes.slice(offset, offset + count));
			},
		}),
		{
			status: offset ? 206 : 200,
			headers: offset
				? {
						"content-range": `bytes ${offset}-${bytes.length - 1}/${bytes.length}`,
					}
				: {},
		},
	);
};

async function readAll(file: DownloadedFile, step = 64) {
	const out = new Uint8Array(file.size);
	for (let offset = 0; offset < file.size; offset += step) {
		const end = Math.min(file.size, offset + step);
		out.set(
			new Uint8Array(await file.slice(offset, end).arrayBuffer()),
			offset,
		);
	}
	return out;
}

describe("a tunnelled download", () => {
	const bytes = bytesOf(1000);

	test("reads the file front to back from one response and follows redirects", async () => {
		const { fetcher, asked } = fakeFetch(whole(bytes));
		const file = new DownloadedFile(1000, ["https://cdn.test/m"], "m.gguf", {
			fetcher,
		});
		expect(await readAll(file)).toEqual(bytes);
		expect(asked).toEqual([
			{ url: "https://cdn.test/m", range: null, redirect: "follow" },
		]);
	});

	test("a broken connection resumes with a Range request", async () => {
		const { fetcher, asked } = fakeFetch(broken(bytes, 300), ranged(bytes));
		const file = new DownloadedFile(1000, ["https://cdn.test/m"], "m.gguf", {
			fetcher,
		});
		expect(await readAll(file)).toEqual(bytes);
		expect(asked.map((entry) => entry.range)).toEqual([null, "bytes=300-"]);
	});

	test("a source that ignores Range is read past the bytes already sent", async () => {
		const { fetcher } = fakeFetch(broken(bytes, 300), whole(bytes));
		const file = new DownloadedFile(1000, ["https://cdn.test/m"], "m.gguf", {
			fetcher,
		});
		expect(await readAll(file)).toEqual(bytes);
	});

	test("a refusing source hands over to the next one", async () => {
		const { fetcher, asked } = fakeFetch(
			() => new Response("gone", { status: 404 }),
			whole(bytes),
		);
		const file = new DownloadedFile(
			1000,
			["https://cdn.test/m", "https://huggingface.co/o/r/resolve/x/m"],
			"m.gguf",
			{ fetcher },
		);
		expect(await readAll(file)).toEqual(bytes);
		expect(asked.map((entry) => entry.url)).toEqual([
			"https://cdn.test/m",
			"https://huggingface.co/o/r/resolve/x/m",
		]);
	});

	test("when every source fails the error names each one", async () => {
		const { fetcher } = fakeFetch(
			() => new Response("", { status: 403 }),
			() => new Response("", { status: 404 }),
		);
		const file = new DownloadedFile(
			1000,
			["https://cdn.test/m", "https://huggingface.co/m"],
			"m.gguf",
			{ fetcher },
		);
		await expect(file.slice(0, 10).arrayBuffer()).rejects.toThrow(
			"None of the sources of m.gguf delivered it (cdn.test: HTTP 403; huggingface.co: HTTP 404).",
		);
	});

	test("once every source failed, a push that tries again fails at once with the same reason", async () => {
		const { fetcher, asked } = fakeFetch(
			() => new Response("", { status: 404 }),
		);
		const file = new DownloadedFile(1000, ["https://cdn.test/m"], "m.gguf", {
			fetcher,
		});
		const device = fakeDevice(1000);
		await expect(
			pushModelAsset({ jobId: JOB, file, push: device.push }),
		).rejects.toThrow(
			"None of the sources of m.gguf delivered it (cdn.test: HTTP 404).",
		);
		expect(asked).toHaveLength(1);
		await expect(file.slice(0, 10).arrayBuffer()).rejects.toThrow(
			"None of the sources of m.gguf delivered it (cdn.test: HTTP 404).",
		);
		expect(asked).toHaveLength(1);
	});

	test("a push asks a source for the first byte before the device hands its job over", async () => {
		const { fetcher, asked } = fakeFetch(
			() => new Response("", { status: 403 }),
			() => new Response(bytes.slice(0, 1), { status: 206 }),
			ranged(bytes),
		);
		const file = new DownloadedFile(
			1000,
			["https://cdn.test/m", "https://huggingface.co/m"],
			"m.gguf",
			{ fetcher },
		);
		const device = fakeDevice(1000, { held: 400 });
		const status = await pushModelAsset({
			jobId: JOB,
			file,
			push: device.push,
		});
		expect(status.state).toBe("present");
		expect(asked).toEqual([
			{ url: "https://cdn.test/m", range: "bytes=0-0", redirect: "follow" },
			{
				url: "https://huggingface.co/m",
				range: "bytes=0-0",
				redirect: "follow",
			},
			{
				url: "https://huggingface.co/m",
				range: "bytes=400-",
				redirect: "follow",
			},
		]);
		expect(device.opens).toEqual([0, 400]);
	});

	test("a file no source delivers never takes the device's job over", async () => {
		const { fetcher } = fakeFetch(() => new Response("", { status: 404 }));
		const file = new DownloadedFile(1000, ["https://cdn.test/m"], "m.gguf", {
			fetcher,
		});
		const device = fakeDevice(1000);
		await expect(
			pushModelAsset({ jobId: JOB, file, push: device.push }),
		).rejects.toThrow("cdn.test: HTTP 404");
		expect(device.opens).toEqual([]);
		const refusing = Object.assign(device.push, {
			refuses: () => "This app can't send that file.",
		});
		await expect(
			pushModelAsset({
				jobId: JOB,
				file: memoryFile(bytesOf(10)),
				push: refusing,
			}),
		).rejects.toThrow("This app can't send that file.");
		expect(device.opens).toEqual([]);
	});

	test("reading again from an earlier byte drops what the last response still held", async () => {
		const chunked = (offset: number) =>
			new Response(
				new ReadableStream({
					start(controller) {
						for (let at = offset; at < bytes.length; at += 100)
							controller.enqueue(bytes.slice(at, Math.min(at + 100, 1000)));
						controller.close();
					},
				}),
				offset
					? {
							status: 206,
							headers: { "content-range": `bytes ${offset}-999/1000` },
						}
					: {},
			);
		const { fetcher, asked } = fakeFetch(chunked, chunked);
		const file = new DownloadedFile(1000, ["https://cdn.test/m"], "m.gguf", {
			fetcher,
		});
		await file.slice(0, 250).arrayBuffer();
		const again = new Uint8Array(await file.slice(120, 370).arrayBuffer());
		expect(again).toEqual(bytes.slice(120, 370));
		expect(asked.map((entry) => entry.range)).toEqual([null, "bytes=120-"]);
	});

	test("a 206 whose Content-Range CORS hides is checked by its Content-Length", async () => {
		const hidden = (length: (offset: number) => number) => (offset: number) =>
			new Response(bytes.slice(offset), {
				status: 206,
				headers: { "content-length": String(length(offset)) },
			});
		const { fetcher, asked } = fakeFetch(
			broken(bytes, 300),
			hidden((offset) => 1000 - offset),
		);
		const file = new DownloadedFile(1000, ["https://cdn.test/m"], "m.gguf", {
			fetcher,
		});
		expect(await readAll(file)).toEqual(bytes);
		expect(asked.map((entry) => entry.range)).toEqual([null, "bytes=300-"]);

		const other = fakeFetch(
			broken(bytes, 300),
			hidden(() => 1000),
			whole(bytes),
		);
		const mismatched = new DownloadedFile(
			1000,
			["https://cdn.test/m", "https://huggingface.co/m"],
			"m.gguf",
			{ fetcher: other.fetcher },
		);
		expect(await readAll(mismatched)).toEqual(bytes);
		expect(other.asked.map((entry) => entry.url)).toEqual([
			"https://cdn.test/m",
			"https://cdn.test/m",
			"https://huggingface.co/m",
		]);
	});
});

describe("where the bytes come from", () => {
	const descriptor: ModelAssetDescriptor = {
		digest: DIGEST,
		size: 4,
		file_name: "m.gguf",
		sources: ["https://cdn.test/m"],
	};
	const store = (files: Record<string, Uint8Array>) => {
		const reads: [string, number, number][] = [];
		const read = async (
			hash: string,
			name: string,
			offset: number,
			length: number,
		) => {
			reads.push([`${hash}/${name}`, offset, length]);
			const file = files[`${hash}/${name}`];
			if (!file || offset + length > file.length) throw new Error("missing");
			return file.slice(offset, offset + length).buffer;
		};
		return { read, reads };
	};

	test("desktop sends its own Bit store copy in bounded chunks", async () => {
		const { read, reads } = store({ "h/m.gguf": bytesOf(4) });
		const file = await pushFile(
			{ descriptor, bitHash: "h" },
			{ readBitStore: read },
		);
		expect(file.from).toBe("bit_store");
		expect(new Uint8Array(await file.slice(0, 4).arrayBuffer())).toEqual(
			bytesOf(4),
		);
		expect(reads.at(-1)).toEqual(["h/m.gguf", 0, 4]);
		const big = bitStoreFile(read, "h", "m.gguf", 3 * 1024 * 1024);
		await expect(big.slice(0, 3 * 1024 * 1024).arrayBuffer()).rejects.toThrow();
		expect(reads.at(-1)?.[2]).toBe(1024 * 1024);
	});

	test("desktop downloads a missing model first, then sends its copy", async () => {
		const files: Record<string, Uint8Array> = {};
		const { read } = store(files);
		const downloaded: string[] = [];
		const progress: number[] = [];
		const file = await pushFile(
			{ descriptor, bitHash: "h", pin: { id: "model", hub: "hub.test" } },
			{
				readBitStore: read,
				downloadLocally: async (asset, report) => {
					downloaded.push(asset.pin?.id ?? "");
					report(2);
					report(9);
					files["h/m.gguf"] = bytesOf(4);
				},
				onLocalDownload: (bytes) => progress.push(bytes),
			},
		);
		expect(downloaded).toEqual(["model"]);
		expect(progress).toEqual([2, 4]);
		expect(file.from).toBe("bit_store");
	});

	test("cancelling stops waiting for the download into this computer", async () => {
		const { read } = store({});
		const abort = new AbortController();
		const pending = pushFile(
			{ descriptor, bitHash: "h", pin: { id: "model" } },
			{
				readBitStore: read,
				downloadLocally: () => new Promise(() => {}),
				signal: abort.signal,
			},
		);
		abort.abort(new Error("Sending was stopped."));
		await expect(pending).rejects.toThrow("Sending was stopped.");
	});

	test("a failed download into this computer says why", async () => {
		const { read } = store({});
		const environment = {
			readBitStore: read,
			downloadLocally: async () => {
				throw new Error("the hub is unreachable");
			},
		};
		const pinned = { descriptor, bitHash: "h", pin: { id: "model" } };
		await expect(
			pushFile(
				{ ...pinned, descriptor: { ...descriptor, sources: [] } },
				environment,
			),
		).rejects.toThrow(
			"Downloading m.gguf to this computer failed (the hub is unreachable), and no other source of it is known.",
		);
		const { fetcher } = fakeFetch(() => new Response("", { status: 404 }));
		const file = await pushFile(pinned, { ...environment, fetcher });
		expect(file.from).toBe("download");
		await expect(file.ready?.()).rejects.toThrow(
			"None of the sources of m.gguf delivered it (the download to this computer: the hub is unreachable; cdn.test: HTTP 404).",
		);
	});

	test("a device's own sources make a file sendable that this window never prepared", () => {
		const job = modelJobSchema.parse(fixture.jobs.jobs[1]);
		const asset = pushableAsset(job);
		expect(asset.descriptor.sources).toEqual(fixture.jobs.jobs[1].sources);
		expect(asset.bitHash).toBeUndefined();
		const plain = pushableAsset({
			digest: { algorithm: "sha256", hex: "e".repeat(64) },
			size: 4,
			file_name: "m.gguf",
			sources: ["http://plain.test/m", "https://cdn.test/m"],
		});
		expect(plain.descriptor.sources).toEqual(["https://cdn.test/m"]);
		const known = { algorithm: "sha256" as const, hex: "f".repeat(64) };
		rememberPushableAssets([
			{
				pin: "model",
				bitId: "model",
				bitHash: "weights",
				descriptor: { digest: known, size: 4, file_name: "m.gguf" },
			},
		]);
		expect(
			pushableAsset({
				digest: known,
				size: 4,
				file_name: "m.gguf",
				sources: ["https://cdn.test/m"],
			}),
		).toEqual({
			descriptor: {
				digest: known,
				size: 4,
				file_name: "m.gguf",
				sources: ["https://cdn.test/m"],
			},
			bitHash: "weights",
			pin: { id: "model" },
		});
	});

	test("without a local copy the download streams through", async () => {
		const { read } = store({});
		const file = await pushFile(
			{ descriptor, bitHash: "h" },
			{ readBitStore: read },
		);
		expect(file.from).toBe("download");
		expect((await pushFile({ descriptor }, {})).from).toBe("download");
		await expect(
			pushFile({ descriptor: { ...descriptor, sources: [] } }, {}),
		).rejects.toThrow("no copy of m.gguf");
	});

	test("an asset this window prepared keeps its sources; a hub file is found by its blake3", () => {
		const prepared: PreparedModelAsset = {
			pin: "model",
			pinHub: "hub.test",
			bitId: "model",
			bitHash: "weights",
			descriptor: {
				...descriptor,
				digest: { algorithm: "sha256", hex: "c".repeat(64) },
			},
		};
		rememberPushableAssets([prepared]);
		expect(pushableAsset({ ...prepared.descriptor })).toEqual({
			descriptor: prepared.descriptor,
			bitHash: "weights",
			pin: { id: "model", hub: "hub.test" },
		});
		expect(
			pushableAsset({ digest: DIGEST, size: 4, file_name: "m.gguf" }),
		).toEqual({
			descriptor: { digest: DIGEST, size: 4, file_name: "m.gguf" },
			bitHash: DIGEST.hex,
		});
		const unknown = { algorithm: "sha256" as const, hex: "d".repeat(64) };
		expect(
			pushableAsset({ digest: unknown, size: 4, file_name: "x" }).bitHash,
		).toBeUndefined();
	});
});
