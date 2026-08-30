import { afterEach, beforeEach, describe, expect, test } from "bun:test";
import {
	type ITemporaryPresignedUpload,
	TEMPORARY_PRESIGN_BATCH_SIZE,
	uploadTemporaryFilesInBatches,
	uploadTemporaryFilesLocally,
} from "./temporary-upload-batch";
import { clearTemporaryUploadCache } from "./temporary-upload-cache";

type XhrOutcome = { status: number; statusText?: string };

/** `uploadToSignedUrl` is XHR-based, so the transfer layer is faked here. */
class FakeXMLHttpRequest {
	static outcomeFor: (url: string) => XhrOutcome = () => ({ status: 200 });
	static requested: string[] = [];

	readonly upload = {
		addEventListener: (_type: string, _listener: () => void) => {},
	};

	status = 0;
	statusText = "";

	private url = "";
	private readonly listeners = new Map<string, (() => void)[]>();

	open(_method: string, url: string) {
		this.url = url;
	}

	setRequestHeader(_name: string, _value: string) {}

	addEventListener(type: string, listener: () => void) {
		const existing = this.listeners.get(type) ?? [];
		existing.push(listener);
		this.listeners.set(type, existing);
	}

	send(_body: unknown) {
		queueMicrotask(() => {
			FakeXMLHttpRequest.requested.push(this.url);
			const outcome = FakeXMLHttpRequest.outcomeFor(this.url);
			this.status = outcome.status;
			this.statusText = outcome.statusText ?? "";
			for (const listener of this.listeners.get("load") ?? []) listener();
		});
	}

	abort() {
		for (const listener of this.listeners.get("abort") ?? []) listener();
	}
}

function fakeFile(name: string, relativePath?: string): File {
	const file = new File([new Uint8Array(8)], name);
	if (relativePath) {
		Object.defineProperty(file, "webkitRelativePath", {
			value: relativePath,
		});
	}
	return file;
}

function presignFor(files: File[], batchIndex: number) {
	return files.map(
		(file, position): ITemporaryPresignedUpload => ({
			uploadUrl: `https://storage.test/upload/${batchIndex}-${position}-${file.name}`,
			downloadUrl: `https://storage.test/download/${batchIndex}-${position}-${file.name}`,
			key: `tmp/${batchIndex}-${position}-${file.name}`,
		}),
	);
}

let originalXhr: unknown;

beforeEach(() => {
	originalXhr = (globalThis as { XMLHttpRequest?: unknown }).XMLHttpRequest;
	(globalThis as { XMLHttpRequest?: unknown }).XMLHttpRequest =
		FakeXMLHttpRequest;
	FakeXMLHttpRequest.outcomeFor = () => ({ status: 200 });
	FakeXMLHttpRequest.requested = [];
	clearTemporaryUploadCache();
});

afterEach(() => {
	(globalThis as { XMLHttpRequest?: unknown }).XMLHttpRequest = originalXhr;
	clearTemporaryUploadCache();
});

describe("uploadTemporaryFilesInBatches", () => {
	test("presigns in bounded batches and uploads every file", async () => {
		const files = Array.from({ length: 250 }, (_, index) =>
			fakeFile(`file-${index}.bin`, `folder/file-${index}.bin`),
		);
		const batchSizes: number[] = [];

		const results = await uploadTemporaryFilesInBatches(files, {
			scope: "test",
			presign: async (batch) => {
				batchSizes.push(batch.length);
				return presignFor(batch, batchSizes.length);
			},
		});

		expect(results).toHaveLength(250);
		expect(results.every((result) => result.uploaded?.url)).toBe(true);
		expect(FakeXMLHttpRequest.requested).toHaveLength(250);
		expect(batchSizes.reduce((sum, size) => sum + size, 0)).toBe(250);
		expect(Math.max(...batchSizes)).toBeLessThanOrEqual(
			TEMPORARY_PRESIGN_BATCH_SIZE,
		);
	});

	test("keeps results aligned with the input order", async () => {
		const files = [fakeFile("a.txt"), fakeFile("b.txt"), fakeFile("c.txt")];

		const results = await uploadTemporaryFilesInBatches(files, {
			scope: "test",
			presign: async (batch) => presignFor(batch, 0),
		});

		expect(results.map((result) => result.file.name)).toEqual([
			"a.txt",
			"b.txt",
			"c.txt",
		]);
		expect(results[1].uploaded?.url).toContain("b.txt");
	});

	test("uploads same-named files from different folders separately", async () => {
		const files = [
			fakeFile("notes.md", "docs/a/notes.md"),
			fakeFile("notes.md", "docs/b/notes.md"),
		];

		const results = await uploadTemporaryFilesInBatches(files, {
			scope: "test",
			presign: async (batch) => presignFor(batch, 0),
		});

		expect(FakeXMLHttpRequest.requested).toHaveLength(2);
		expect(results[0].uploaded?.url).not.toBe(results[1].uploaded?.url);
	});

	test("reports a failed transfer per file without sinking the batch", async () => {
		const files = [fakeFile("good.txt"), fakeFile("bad.txt")];
		FakeXMLHttpRequest.outcomeFor = (url) =>
			url.includes("bad.txt")
				? { status: 400, statusText: "Bad Request" }
				: { status: 200 };

		const results = await uploadTemporaryFilesInBatches(files, {
			scope: "test",
			presign: async (batch) => presignFor(batch, 0),
		});

		expect(results[0].uploaded?.url).toContain("good.txt");
		expect(results[0].error).toBeUndefined();
		expect(results[1].uploaded).toBeUndefined();
		expect(results[1].error).toContain("400");
	});

	test("a presign failure keeps earlier successes and fails the rest", async () => {
		const files = Array.from({ length: 150 }, (_, index) =>
			fakeFile(`file-${index}.bin`),
		);
		let calls = 0;

		const results = await uploadTemporaryFilesInBatches(files, {
			scope: "test",
			presign: async (batch) => {
				calls++;
				if (calls > 1) throw new Error("presign exploded");
				return presignFor(batch, calls);
			},
		});

		const uploaded = results.filter((result) => result.uploaded);
		const failed = results.filter((result) => result.error);
		expect(uploaded.length).toBeGreaterThan(0);
		expect(failed.length).toBeGreaterThan(0);
		expect(uploaded.length + failed.length).toBe(150);
		expect(failed[failed.length - 1].error).toContain("presign exploded");
	});

	test("files the backend declined to presign are reported, not dropped", async () => {
		const files = [fakeFile("a.txt"), fakeFile("b.txt")];

		const results = await uploadTemporaryFilesInBatches(files, {
			scope: "test",
			presign: async (batch) => [presignFor(batch, 0)[0]],
		});

		expect(results[0].uploaded?.url).toContain("a.txt");
		expect(results[1].uploaded).toBeUndefined();
		expect(results[1].error).toBeTruthy();
	});

	test("reuses cached uploads instead of presigning them again", async () => {
		const files = [fakeFile("cached.txt")];
		let calls = 0;
		const options = {
			scope: "test",
			presign: async (batch: File[]) => {
				calls++;
				return presignFor(batch, calls);
			},
		};

		const first = await uploadTemporaryFilesInBatches(files, options);
		const second = await uploadTemporaryFilesInBatches(files, options);

		expect(calls).toBe(1);
		expect(FakeXMLHttpRequest.requested).toHaveLength(1);
		expect(second[0].uploaded?.url).toBe(first[0].uploaded?.url as string);
	});
});

describe("uploadTemporaryFilesLocally", () => {
	test("uploads every file and isolates failures", async () => {
		const files = [fakeFile("a.txt"), fakeFile("boom.txt"), fakeFile("c.txt")];

		const results = await uploadTemporaryFilesLocally(files, async (file) => {
			if (file.name === "boom.txt") throw new Error("disk full");
			return { url: `local://${file.name}` };
		});

		expect(results[0].uploaded?.url).toBe("local://a.txt");
		expect(results[1].error).toContain("disk full");
		expect(results[2].uploaded?.url).toBe("local://c.txt");
	});

	test("keeps at most the configured number of writes in flight", async () => {
		const files = Array.from({ length: 20 }, (_, index) =>
			fakeFile(`file-${index}.bin`),
		);
		let inFlight = 0;
		let peak = 0;

		await uploadTemporaryFilesLocally(
			files,
			async (file) => {
				inFlight++;
				peak = Math.max(peak, inFlight);
				await new Promise((resolve) => setTimeout(resolve, 1));
				inFlight--;
				return { url: `local://${file.name}` };
			},
			{ concurrency: 3 },
		);

		expect(peak).toBeLessThanOrEqual(3);
	});
});
