import { describe, expect, test } from "bun:test";
import {
	getExtension,
	matchesAccept,
	parentPrefix,
	resolveAssetPath,
} from "./asset-path";

describe("resolveAssetPath", () => {
	test("keeps app-relative listings relative to the browsed prefix", () => {
		expect(resolveAssetPath("", "logo.jpg")).toBe("logo.jpg");
		expect(resolveAssetPath("media", "media/logo.jpg")).toBe("media/logo.jpg");
		expect(resolveAssetPath("media/inner", "media/inner/logo.jpg")).toBe(
			"media/inner/logo.jpg",
		);
	});

	// The list endpoints used to hand back raw object-store keys, which the picker
	// fed back as a prefix — the base was then prepended a second time and every
	// folder listed as empty.
	test("never re-prefixes a raw object-store key", () => {
		expect(resolveAssetPath("", "apps/app-1/upload/media")).toBe("media");
		expect(resolveAssetPath("media", "apps/app-1/upload/media/logo.jpg")).toBe(
			"media/logo.jpg",
		);
		expect(
			resolveAssetPath("", "users/sub-1/apps/app-1/private/logo.jpg"),
		).toBe("logo.jpg");
	});

	test("tolerates stray slashes in the browsed prefix", () => {
		expect(resolveAssetPath("media/", "logo.jpg")).toBe("media/logo.jpg");
		expect(resolveAssetPath("/media//inner/", "logo.jpg")).toBe(
			"media/inner/logo.jpg",
		);
	});

	test("drops dot segments from both sides", () => {
		expect(resolveAssetPath("media/../..", "logo.jpg")).toBe("media/logo.jpg");
		expect(resolveAssetPath("media", "../../../etc/passwd")).toBe(
			"media/passwd",
		);
		expect(resolveAssetPath("media", "..")).toBe("media");
	});
});

describe("parentPrefix", () => {
	test("walks up one level and stops at the root", () => {
		expect(parentPrefix("media/inner")).toBe("media");
		expect(parentPrefix("media")).toBe("");
		expect(parentPrefix("")).toBe("");
		expect(parentPrefix("media/inner/")).toBe("media");
	});
});

describe("getExtension", () => {
	test("reads the extension from the file name only", () => {
		expect(getExtension("media/logo.jpg")).toBe("jpg");
		expect(getExtension("media.v2/logo")).toBe("");
		expect(getExtension("Report.FINAL.PDF")).toBe("pdf");
		expect(getExtension(".gitignore")).toBe("");
	});
});

describe("matchesAccept", () => {
	test("filters by asset kind and passes everything for all", () => {
		expect(matchesAccept("media/logo.jpg", "image")).toBe(true);
		expect(matchesAccept("media/logo.JPEG", "image")).toBe(true);
		expect(matchesAccept("media/clip.mp4", "image")).toBe(false);
		expect(matchesAccept("media/clip.mp4", "all")).toBe(true);
		expect(matchesAccept("media/notes", "image")).toBe(false);
	});
});
