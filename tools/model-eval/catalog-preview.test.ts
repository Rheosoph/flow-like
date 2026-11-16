import { describe, expect, test } from "bun:test";
import {
	type PreviewBit,
	planPreviewUpdates,
	previewDescription,
	verifyPreviewAsset,
} from "./catalog-preview";
import type {
	CatalogPreviewAsset,
	CatalogPreviewEntry,
	PreviewKind,
} from "./catalog-preview-types";

const asset: CatalogPreviewAsset = {
	url: "https://publisher.example/samples/voice.wav",
	kind: "audio",
	source_url: "https://publisher.example/model",
	description: "Publisher speech sample.",
	scope: "output",
};

function bit(overrides: Partial<PreviewBit> = {}): PreviewBit {
	return {
		id: "voice-bit",
		type: "Tts",
		repository: "https://publisher.example/model",
		parameters: { provider: { model_id: "publisher/voice" } },
		meta: {
			en: {
				name: "Voice",
				description: "Read text aloud.",
				long_description: "Owner description and runtime limitations.",
				preview_media: ["https://publisher.example/existing.wav"],
				tags: ["speech"],
				created_at: { secs_since_epoch: 1, nanos_since_epoch: 0 },
			},
		},
		...overrides,
	};
}

function entry(
	overrides: Partial<CatalogPreviewEntry> = {},
): CatalogPreviewEntry {
	return {
		bit_ids: ["voice-bit"],
		model_id: "publisher/voice",
		repository: "https://publisher.example/model",
		media: [asset],
		...overrides,
	};
}

function fakeFetch(response: Response): typeof fetch {
	return (async () => response) as unknown as typeof fetch;
}

function mediaResponse(
	bytes: Uint8Array | string,
	type = "application/octet-stream",
) {
	const body = typeof bytes === "string" ? bytes : new Uint8Array(bytes).buffer;
	return new Response(body, { headers: { "content-type": type } });
}

describe("catalog preview planning", () => {
	test("preserves existing media, owner descriptions and unrelated metadata", () => {
		const original = bit();
		const before = JSON.stringify(original);
		const [plan] = planPreviewUpdates([original], [entry()]);
		expect(plan.changed).toBe(true);
		expect(plan.meta.preview_media).toEqual([
			"https://publisher.example/existing.wav",
			asset.url,
		]);
		expect(plan.meta).toMatchObject({
			name: original.meta.en.name,
			description: original.meta.en.description,
			tags: original.meta.en.tags,
			created_at: original.meta.en.created_at,
		});
		expect(plan.meta.long_description).toStartWith(
			`${original.meta.en.long_description}\n\nPreview samples and sources:`,
		);
		expect(plan.meta.long_description).toContain(asset.source_url);
		expect(JSON.stringify(original)).toBe(before);
	});

	test("rerunning the same update makes no changes or duplicate media", () => {
		const original = bit();
		const [first] = planPreviewUpdates([original], [entry()]);
		const [second] = planPreviewUpdates(
			[{ ...original, meta: { ...original.meta, en: first.meta } }],
			[entry()],
		);
		expect(second.changed).toBe(false);
		expect(second.meta).toEqual(first.meta);
	});

	test("replaces only managed source lines and retains copy appended by an owner", () => {
		const first = previewDescription("Owner introduction.", [asset]);
		for (const separator of ["\n\n", "\n"]) {
			const ownerCopy = `${separator}Owner added deployment instructions.\n\nAnother owner paragraph.`;
			const updated = previewDescription(first + ownerCopy, [
				{ ...asset, description: "New publisher sample." },
			]);
			expect(updated).toStartWith("Owner introduction.\n\n");
			expect(updated).toEndWith(ownerCopy);
			expect(updated).toContain("New publisher sample.");
			expect(updated).not.toContain(asset.description);
			expect(updated.match(/Preview samples and sources:/g)).toHaveLength(1);
		}
		expect(previewDescription(null, [asset])).toStartWith("Preview samples");
		expect(() => previewDescription({ owner: "copy" }, [asset])).toThrow(
			"long description",
		);
	});

	test("requires exact identity and unique existing target IDs", () => {
		for (const changed of [
			bit({ parameters: { provider: { model_id: "publisher/voice-v2" } } }),
			bit({ repository: "https://publisher.example/model-finetune" }),
			bit({ parameters: null }),
		]) {
			expect(() => planPreviewUpdates([changed], [entry()])).toThrow(
				"identity changed",
			);
		}
		expect(() => planPreviewUpdates([bit(), bit()], [entry()])).toThrow(
			"Duplicate Bit IDs",
		);
		expect(() => planPreviewUpdates([bit()], [entry(), entry()])).toThrow(
			"Multiple preview entries",
		);
		expect(() => planPreviewUpdates([], [entry()])).toThrow("no longer exists");
		expect(() =>
			planPreviewUpdates(
				[bit()],
				[entry({ model_id: undefined, repository: undefined })],
			),
		).toThrow("exact model identity");
	});

	test("keeps samples within model categories and preserves declared sample scope", () => {
		for (const type of ["File", "Config", "Llm", "Embedding", "Forecasting"]) {
			expect(() => planPreviewUpdates([bit({ type })], [entry()])).toThrow(
				"media kind does not match",
			);
		}
		expect(
			previewDescription("", [{ ...asset, scope: "base_model_output" }]),
		).toContain("this catalog variant was not separately sampled");
		expect(
			previewDescription("", [{ ...asset, scope: "input_demo" }]),
		).toContain("example input used in the publisher's demonstration");
		expect(() =>
			previewDescription("", [
				{ ...asset, scope: "unverified" as CatalogPreviewAsset["scope"] },
			]),
		).toThrow("sample scope");
	});

	test("accepts supported media extensions with stable query strings", () => {
		const formats: [PreviewKind, string, string[]][] = [
			[
				"audio",
				"Tts",
				["mp3", "wav", "ogg", "oga", "m4a", "aac", "flac", "opus", "weba"],
			],
			[
				"image",
				"ImageGeneration",
				["png", "jpg", "jpeg", "webp", "gif", "avif", "svg"],
			],
			["video", "VideoGeneration", ["mp4", "webm", "mov", "m4v"]],
		];
		for (const [kind, type, extensions] of formats) {
			for (const extension of extensions) {
				const url = `https://publisher.example/sample.${extension.toUpperCase()}?download=true`;
				expect(
					planPreviewUpdates(
						[bit({ type })],
						[entry({ media: [{ ...asset, kind, url }] })],
					)[0].meta.preview_media,
				).toContain(url);
			}
		}
		for (const url of [
			"https://publisher.example/demo",
			"https://publisher.example/demo.html",
			"https://publisher.example/demo.wav.exe",
		]) {
			expect(() =>
				planPreviewUpdates([bit()], [entry({ media: [{ ...asset, url }] })]),
			).toThrow("supported audio extension");
		}
	});

	test("rejects unsafe or ephemeral asset and source URLs before fetching", async () => {
		for (const url of [
			"http://publisher.example/sample.wav",
			"https://user:secret@publisher.example/sample.wav",
			" https://publisher.example/sample.wav",
			...[
				"X-Amz-Signature",
				"X-Goog-Credential",
				"signature",
				"sig",
				"token",
				"access_token",
				"api_key",
				"authorization",
				"Expires",
				"Key-Pair-Id",
			].map((key) => `https://publisher.example/sample.wav?${key}=secret`),
		]) {
			const fetcher = (async () => {
				throw new Error("Unexpected fetch");
			}) as unknown as typeof fetch;
			expect(() =>
				planPreviewUpdates([bit()], [entry({ media: [{ ...asset, url }] })]),
			).toThrow();
			await expect(
				verifyPreviewAsset({ ...asset, url }, fetcher),
			).rejects.not.toThrow("Unexpected fetch");
		}
		expect(() =>
			planPreviewUpdates(
				[bit()],
				[
					entry({
						media: [{ ...asset, source_url: "http://publisher.example/model" }],
					}),
				],
			),
		).toThrow("HTTPS");
		expect(() =>
			planPreviewUpdates(
				[bit()],
				[
					entry({
						media: [{ ...asset, description: "First line\nOwner text" }],
					}),
				],
			),
		).toThrow("single-line");
	});
});

describe("preview byte verification", () => {
	test("reads binary container tags at byte offsets when size fields resemble UTF-8", async () => {
		const encoder = new TextEncoder();
		const binarySize = [0xc2, 0xa9, 0x00, 0x00];
		const samples: [PreviewKind, string, Uint8Array][] = [
			[
				"image",
				"webp",
				new Uint8Array([
					...encoder.encode("RIFF"),
					...binarySize,
					...encoder.encode("WEBPsample"),
				]),
			],
			[
				"audio",
				"wav",
				new Uint8Array([
					...encoder.encode("RIFF"),
					...binarySize,
					...encoder.encode("WAVEsample"),
				]),
			],
			[
				"image",
				"avif",
				new Uint8Array([...binarySize, ...encoder.encode("ftypavif0000")]),
			],
			[
				"audio",
				"m4a",
				new Uint8Array([...binarySize, ...encoder.encode("ftypM4A 0000")]),
			],
			[
				"video",
				"mp4",
				new Uint8Array([...binarySize, ...encoder.encode("ftypisom0000")]),
			],
		];
		for (const [kind, extension, bytes] of samples) {
			const sample = {
				...asset,
				kind,
				url: `https://publisher.example/sample.${extension}`,
			};
			expect(
				await verifyPreviewAsset(sample, fakeFetch(mediaResponse(bytes))),
			).toMatchObject({ url: sample.url, kind });
		}
	});

	test("recognizes actual image, audio and video bytes", async () => {
		const samples: [PreviewKind, string, Uint8Array | string][] = [
			[
				"image",
				"png",
				new Uint8Array([0x89, 0x50, 0x4e, 0x47, 0x0d, 0x0a, 0x1a, 0x0a]),
			],
			["image", "jpg", new Uint8Array([0xff, 0xd8, 0xff, 0xe0])],
			["image", "webp", "RIFF0000WEBPsample"],
			["image", "gif", "GIF89asample"],
			["image", "avif", "0000ftypavif0000"],
			[
				"image",
				"svg",
				'<?xml version="1.0"?>\n<svg xmlns="http://www.w3.org/2000/svg"></svg>',
			],
			["audio", "wav", "RIFF0000WAVEsample"],
			["audio", "mp3", "ID3sample"],
			["audio", "ogg", "OggSsample"],
			["audio", "flac", "fLaCsample"],
			["audio", "m4a", "0000ftypM4A 0000"],
			["video", "mp4", "0000ftypisom0000"],
			["video", "webm", new Uint8Array([0x1a, 0x45, 0xdf, 0xa3])],
		];
		for (const [kind, extension, bytes] of samples) {
			const sample = {
				...asset,
				kind,
				url: `https://publisher.example/sample.${extension}`,
			};
			expect(
				await verifyPreviewAsset(sample, fakeFetch(mediaResponse(bytes))),
			).toMatchObject({ url: sample.url, kind });
		}
	});

	test("rejects HTTP-200 HTML pages, misleading MIME types and truncated signatures", async () => {
		const invalid: [PreviewKind, string, string | Uint8Array, string][] = [
			["audio", "wav", "<!doctype html><html>Rate limited</html>", "audio/wav"],
			[
				"image",
				"svg",
				"<html><body><svg></svg></body></html>",
				"image/svg+xml",
			],
			["image", "svg", "<svg></svg>", "text/html; charset=utf-8"],
			["image", "png", new Uint8Array([0x89, 0x50, 0x4e, 0x47]), "image/png"],
			["audio", "weba", new Uint8Array([0x1a, 0x45]), "audio/webm"],
			["image", "png", "RIFF0000WAVEsample", "image/png"],
		];
		for (const [kind, extension, bytes, mime] of invalid) {
			await expect(
				verifyPreviewAsset(
					{
						...asset,
						kind,
						url: `https://publisher.example/sample.${extension}`,
					},
					fakeFetch(mediaResponse(bytes, mime)),
				),
			).rejects.toThrow(`not valid ${kind}`);
		}
	});

	test("uses a bounded ranged read and cancels the body after verification", async () => {
		let cancelled = false;
		let request: RequestInit | undefined;
		const response = new Response(
			new ReadableStream<Uint8Array>({
				start(controller) {
					controller.enqueue(new TextEncoder().encode("RIFF0000"));
					controller.enqueue(
						new TextEncoder().encode(`WAVE${"x".repeat(512)}`),
					);
				},
				cancel() {
					cancelled = true;
				},
			}),
		);
		const fetcher = (async (_url: unknown, options?: RequestInit) => {
			request = options;
			return response;
		}) as unknown as typeof fetch;
		await verifyPreviewAsset(asset, fetcher);
		expect(new Headers(request?.headers).get("range")).toBe("bytes=0-1023");
		expect(cancelled).toBe(true);
	});

	test("rejects failed responses and insecure redirects, but permits stable URLs with HTTPS storage redirects", async () => {
		for (const response of [
			new Response("unavailable", { status: 503 }),
			new Response(null),
		]) {
			await expect(
				verifyPreviewAsset(asset, fakeFetch(response)),
			).rejects.toThrow("HTTP");
		}
		for (const url of [
			"http://cdn.example/sample.wav",
			"https://user:secret@cdn.example/sample.wav",
		]) {
			const response = mediaResponse("RIFF0000WAVEsample");
			Object.defineProperty(response, "url", { value: url });
			await expect(
				verifyPreviewAsset(asset, fakeFetch(response)),
			).rejects.toThrow("credential-free HTTPS");
		}
		const redirected = mediaResponse("RIFF0000WAVEsample");
		Object.defineProperty(redirected, "url", {
			value: "https://cdn.example/sample.wav?X-Amz-Signature=temporary",
		});
		expect((await verifyPreviewAsset(asset, fakeFetch(redirected))).url).toBe(
			asset.url,
		);
	});
});
