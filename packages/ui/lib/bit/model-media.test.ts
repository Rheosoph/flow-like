import { describe, expect, test } from "bun:test";
import type { IBit } from "../schema/bit/bit";
import { bitPreviewMedia } from "./model-media";

function bit(previewMedia: unknown): IBit {
	return { meta: { en: { preview_media: previewMedia } } } as unknown as IBit;
}

describe("preview media", () => {
	test("sorts samples by file type and ignores query strings", () => {
		expect(
			bitPreviewMedia(
				bit([
					"https://cdn.example.com/a.png?v=2",
					"/media/voice.mp3",
					"https://cdn.example.com/clip.webm",
					"https://cdn.example.com/notes.pdf",
				]),
			),
		).toEqual({
			images: ["https://cdn.example.com/a.png?v=2"],
			audio: ["/media/voice.mp3"],
			video: ["https://cdn.example.com/clip.webm"],
		});
	});

	test("drops unsafe, duplicate and malformed entries", () => {
		expect(
			bitPreviewMedia(
				bit([
					"javascript:alert(1)//.png",
					"data:image/png;base64,AAAA",
					"https://user:pass@example.com/a.png",
					"https://example.com/b.png",
					"https://example.com/b.png",
					42,
					"",
				]),
			).images,
		).toEqual(["https://example.com/b.png"]);
		expect(bitPreviewMedia(bit(undefined))).toEqual({
			images: [],
			audio: [],
			video: [],
		});
	});
});
