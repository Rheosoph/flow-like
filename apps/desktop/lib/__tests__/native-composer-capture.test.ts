import { describe, expect, test, vi } from "vitest";
const native = vi.hoisted(() => ({ invoke: vi.fn() }));
vi.mock("@tauri-apps/api/core", () => ({
	invoke: native.invoke,
	isTauri: () => true,
}));
import { captureFileName } from "@flow-like/flow-like-ui/lib/capture-file-name";
import {
	locationText,
	nativeComposerSources,
	splitNativeError,
} from "../../hooks/use-native-composer-capture";

const kinds = (sources: { kind: string }[]) =>
	sources.map((source) => source.kind);

describe("native composer sources", () => {
	test("an iPhone with scanner, camera and live text offers every source", () => {
		expect(
			kinds(
				nativeComposerSources(
					"scope",
					{ document: true, photo: true, liveText: true },
					true,
				),
			),
		).toEqual([
			"scan_document",
			"scan_text",
			"live_text",
			"take_photo",
			"location",
		]);
	});

	test("Android's file-picker fallback is not offered as a scanner", () => {
		expect(
			kinds(
				nativeComposerSources(
					"scope",
					{ document: true, documentMode: "picker", photo: true },
					true,
				),
			),
		).toEqual(["take_photo", "location"]);
	});

	test("a desktop without capture offers nothing", () => {
		expect(nativeComposerSources("scope", {}, false)).toEqual([]);
	});

	test("a cancelled scan resolves empty and a failure keeps the native message", async () => {
		const [scan] = nativeComposerSources("scope", { document: true }, false);
		native.invoke.mockRejectedValueOnce(
			"cancelled: Document scanning was cancelled.",
		);
		await expect(scan.attach()).resolves.toEqual({});
		native.invoke.mockRejectedValueOnce("unavailable: No camera.");
		await expect(scan.attach()).rejects.toThrow("No camera.");
		expect(native.invoke).toHaveBeenCalledWith("native_bot_request", {
			requestId: expect.any(String),
			options: { scope: "scope", operation: "document" },
		});
	});

	test("scanned text without content is reported instead of inserted", async () => {
		const [, scanText] = nativeComposerSources(
			"scope",
			{ document: true },
			false,
		);
		native.invoke.mockResolvedValueOnce({ text: "  Invoice 42 " });
		await expect(scanText.attach()).resolves.toEqual({ text: "Invoice 42" });
		native.invoke.mockResolvedValueOnce({ text: "  " });
		await expect(scanText.attach()).rejects.toThrow();
	});
});

describe("capture helpers", () => {
	test("native errors split into code and message", () => {
		expect(splitNativeError("busy: Finish the current scan first.")).toEqual({
			code: "busy",
			message: "Finish the current scan first.",
		});
		expect(splitNativeError(new Error("plain"))).toEqual({
			code: "",
			message: "plain",
		});
	});

	test("capture files get readable, sortable names", () => {
		const at = new Date(2026, 10, 10, 9, 5, 3);
		expect(captureFileName("scan", at, ".pdf")).toBe(
			"scan-2026-11-10-090503.pdf",
		);
		expect(captureFileName("photo", at, ".jpg", 2)).toBe(
			"photo-2026-11-10-090503-2.jpg",
		);
	});

	test("a location becomes coordinates with a map link", () => {
		const apple = locationText(52.520008, 13.404954, 14.6, true);
		expect(apple).toContain("52.52001, 13.40495");
		expect(apple).toContain("±15 m");
		expect(apple).toContain("https://maps.apple.com/?ll=52.52001,13.40495");
		expect(locationText(1, 2, 3, false)).toContain(
			"https://www.openstreetmap.org/?mlat=1.00000&mlon=2.00000",
		);
	});
});
