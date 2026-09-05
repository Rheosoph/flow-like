import { describe, expect, it } from "bun:test";
import {
	PROFILE_MEDIA_MAX_BYTES,
	fitProfileMedia,
	prepareProfileMedia,
	profileMediaDimensions,
	profileMediaUrl,
} from "./profile-media-image";

describe("profile artwork preparation", () => {
	it("fits icons and covers without changing aspect or enlarging small images", () => {
		expect(fitProfileMedia(2048, 1024, "icon")).toEqual({
			width: 512,
			height: 256,
		});
		expect(fitProfileMedia(2400, 3600, "cover")).toEqual({
			width: 1067,
			height: 1600,
		});
		expect(fitProfileMedia(96, 128, "icon")).toEqual({
			width: 96,
			height: 128,
		});
	});
	it("rejects dimensions that cannot be decoded within the source limit", () => {
		for (const dimensions of [
			[0, 50],
			[50, -1],
			[4097, 1],
			[1, 4097],
			[Number.NaN, 3],
		])
			expect(() =>
				fitProfileMedia(dimensions[0], dimensions[1], "cover"),
			).toThrow();
		expect(fitProfileMedia(4096, 4096, "cover")).toEqual({
			width: 1600,
			height: 1600,
		});
	});
	it("validates byte signatures instead of trusting an image filename or MIME type", async () => {
		const png = Uint8Array.from(
			atob(
				"iVBORw0KGgoAAAANSUhEUgAAAAEAAAABCAQAAAC1HAwCAAAAC0lEQVR42mP8/x8AAwMCAO+aD1sAAAAASUVORK5CYII=",
			),
			(char) => char.charCodeAt(0),
		);
		expect(profileMediaDimensions(png)).toEqual({ width: 1, height: 1 });
		expect(() => profileMediaDimensions(png.subarray(0, 16))).toThrow();
		await expect(
			prepareProfileMedia(
				new Blob(["<svg></svg>"], { type: "image/png" }),
				"icon",
			),
		).rejects.toThrow("could not be read");
		await expect(
			prepareProfileMedia(new Blob([png], { type: "image/svg+xml" }), "icon"),
		).rejects.toThrow("Choose a PNG");
	});
	it("rejects empty and oversized files before browser decoding", async () => {
		await expect(
			prepareProfileMedia(new Blob([], { type: "image/png" }), "icon"),
		).rejects.toThrow("empty");
		await expect(
			prepareProfileMedia(
				new Blob([new Uint8Array(PROFILE_MEDIA_MAX_BYTES + 1)], {
					type: "image/png",
				}),
				"cover",
			),
		).rejects.toThrow("10 MB");
	});
});

describe("profile artwork URLs", () => {
	it("accepts HTTP images while rejecting relative, executable, and ambiguous URLs", () => {
		expect(profileMediaUrl(" https://example.com/icon.webp?size=512 ")).toBe(
			"https://example.com/icon.webp?size=512",
		);
		expect(() => profileMediaUrl("/media/icon.webp")).toThrow();
		expect(profileMediaUrl(" ")).toBeNull();
		for (const url of [
			"javascript:alert(1)",
			"data:image/svg+xml,test",
			"//example.com/icon.webp",
			"/\\example.com/icon.webp",
			"https://name:secret@example.com/icon",
			"java\nscript:alert(1)",
		])
			expect(() => profileMediaUrl(url)).toThrow();
	});
});
