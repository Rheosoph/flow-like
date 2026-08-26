import { afterEach, beforeEach, describe, expect, mock, test } from "bun:test";
import { Window } from "happy-dom";
import { registerLivePage } from "../components/a2ui/live-page-registry";
import {
	INLINE_PAGE_REVEAL_EVENT,
	captureInlineAppPageSnapshots,
	isAppPageSnapshotSourceCurrent,
	normalizePageCaptureClone,
	uploadPageSnapshots,
	waitForCapturePaint,
} from "./app-page-snapshot";

let browserWindow: Window;
const rasterizedElements: HTMLElement[] = [];

mock.module("html2canvas-pro", () => ({
	default: async (element: HTMLElement) => {
		rasterizedElements.push(element);
		return {
			toBlob: (callback: (blob: Blob) => void, mediaType: string) =>
				callback(new Blob(["raster"], { type: mediaType })),
		};
	},
}));

beforeEach(() => {
	browserWindow = new Window({ url: "https://flow-like.test" });
	Object.assign(browserWindow, { SyntaxError, TypeError });
	Object.assign(globalThis, {
		document: browserWindow.document,
		window: browserWindow,
	});
	rasterizedElements.length = 0;
});

afterEach(() => {
	browserWindow.close();
});

describe("app page capture hardening", () => {
	test("resolves the exact registered app and event instance", async () => {
		const target = document.createElement("main");
		const distractor = document.createElement("main");
		for (const element of [target, distractor]) {
			Object.defineProperties(element, {
				clientHeight: { configurable: true, value: 600 },
				clientWidth: { configurable: true, value: 720 },
				scrollHeight: { configurable: true, value: 600 },
				scrollWidth: { configurable: true, value: 720 },
			});
			element.getBoundingClientRect = () => ({ width: 720 }) as DOMRect;
			document.body.append(element);
		}
		const handle = (element: HTMLElement, appId: string, eventId: string) => ({
			appId,
			eventId,
			pageId: `${appId}-page`,
			getContainer: () => element,
			getElementValues: () => ({}),
			getSurface: () => null,
			isLoading: () => false,
			setElementValue: () => undefined,
			triggerComponentEvent: async () => ({
				actionCount: 0,
				runs: [],
				source: "none" as const,
				triggered: false,
			}),
		});
		const unregisterTarget = registerLivePage(
			handle(target, "target-app", "target-event"),
		);
		const unregisterDistractor = registerLivePage(
			handle(distractor, "target-app", "other-event"),
		);

		try {
			const result = await captureInlineAppPageSnapshots(
				"target-app",
				"target-event",
				500,
			);
			expect(result.complete).toBe(true);
			expect(rasterizedElements).toEqual([target]);
		} finally {
			unregisterDistractor();
			unregisterTarget();
		}
	});

	test("ties visual evidence to the exact live instance that was captured", async () => {
		const target = document.createElement("main");
		Object.defineProperties(target, {
			clientHeight: { configurable: true, value: 300 },
			clientWidth: { configurable: true, value: 500 },
			scrollHeight: { configurable: true, value: 300 },
			scrollWidth: { configurable: true, value: 500 },
		});
		target.getBoundingClientRect = () => ({ width: 500 }) as DOMRect;
		document.body.append(target);
		const handle = {
			appId: "source-app",
			eventId: "source-event",
			pageId: "source-page",
			getContainer: () => target,
			getElementValues: () => ({}),
			getSurface: () => null,
			isLoading: () => false,
			setElementValue: () => undefined,
			triggerComponentEvent: async () => ({
				actionCount: 0,
				runs: [],
				source: "none" as const,
				triggered: false,
			}),
		};
		const unregister = registerLivePage(handle);
		try {
			const result = await captureInlineAppPageSnapshots(
				"source-app",
				"source-event",
				500,
			);
			expect(
				isAppPageSnapshotSourceCurrent(
					result.source,
					"source-app",
					"source-event",
				),
			).toBe(true);

			unregister();
			expect(
				isAppPageSnapshotSourceCurrent(
					result.source,
					"source-app",
					"source-event",
				),
			).toBe(false);
		} finally {
			unregister();
		}
	});

	test("normalizes a parked capture clone", () => {
		const parked = document.createElement("div");
		parked.setAttribute("data-flowpilot-page-parking", "");
		parked.hidden = true;
		parked.style.position = "fixed";
		parked.style.left = "-100000px";
		parked.style.overflow = "hidden";
		const page = document.createElement("main");
		page.setAttribute("data-flowpilot-capture-target", "capture-1");
		parked.append(page);
		document.body.append(parked);

		normalizePageCaptureClone(document, "capture-1", 720);

		expect(parked.hidden).toBe(false);
		expect(parked.style.getPropertyValue("position")).toBe("absolute");
		expect(parked.style.getPropertyValue("left")).toBe("0px");
		expect(parked.style.getPropertyValue("overflow")).toBe("visible");
		expect(parked.style.getPropertyValue("width")).toBe("720px");
		expect(page.style.getPropertyValue("content-visibility")).toBe("visible");
	});

	test("paint settling completes when animation frames are paused", async () => {
		let cancelled = 0;
		const pausedAnimationFrame = () => 7;
		browserWindow.requestAnimationFrame =
			pausedAnimationFrame as unknown as Window["requestAnimationFrame"];
		browserWindow.cancelAnimationFrame = (() => {
			cancelled += 1;
		}) as unknown as Window["cancelAnimationFrame"];

		await waitForCapturePaint(Date.now() + 20);

		expect(cancelled).toBe(1);
	});

	test("a missing page times out without dispatching a reveal", async () => {
		let reveals = 0;
		window.addEventListener(INLINE_PAGE_REVEAL_EVENT, () => {
			reveals += 1;
		});

		const result = await captureInlineAppPageSnapshots(
			"missing-app",
			"missing-event",
			5,
		);

		expect(result.complete).toBe(false);
		expect(result.images).toHaveLength(0);
		expect(reveals).toBe(0);
	});

	test("desktop attachments use bounded inline base64 without remote storage", async () => {
		Object.assign(browserWindow, { __TAURI_INTERNALS__: {} });
		let remoteCalls = 0;
		const backend = {
			helperState: {
				fileToTemporaryFile: async () => {
					remoteCalls += 1;
					return { url: "https://should-not-run.test/capture.png" };
				},
			},
		} as never;

		const result = await uploadPageSnapshots(backend, [
			{
				blob: new Blob(["image bytes"], { type: "image/png" }),
				mediaType: "image/png",
			},
		]);

		expect(remoteCalls).toBe(0);
		expect(result.uploadErrors).toEqual([]);
		expect(result.uploaded[0]?.url).toBeUndefined();
		expect(atob(result.uploaded[0]?.data ?? "")).toBe("image bytes");
	});

	test("desktop attachments stop at the first segment that exceeds a limit", async () => {
		Object.assign(browserWindow, { __TAURI_INTERNALS__: {} });
		const oversized = new Blob(["oversized"], { type: "image/png" });
		Object.defineProperty(oversized, "size", {
			value: 8 * 1024 * 1024 + 1,
		});
		const backend = { helperState: {} } as never;

		const result = await uploadPageSnapshots(backend, [
			{
				blob: new Blob(["first"], { type: "image/png" }),
				mediaType: "image/png",
			},
			{ blob: oversized, mediaType: "image/png" },
			{
				blob: new Blob(["third"], { type: "image/png" }),
				mediaType: "image/png",
			},
		]);

		expect(result.uploaded).toHaveLength(1);
		expect(atob(result.uploaded[0]?.data ?? "")).toBe("first");
		expect(result.uploadErrors[0]).toContain("Capture 2");
		expect(result.uploadErrors[1]).toContain(
			"1 later capture attachment(s) were skipped",
		);
	});

	test("web attachments retain remotely readable URLs", async () => {
		const backend = {
			helperState: {
				fileToTemporaryFile: async () => ({
					url: "https://temporary.test/capture.png",
				}),
			},
		} as never;

		const result = await uploadPageSnapshots(backend, [
			{
				blob: new Blob(["image bytes"], { type: "image/png" }),
				mediaType: "image/png",
			},
		]);

		expect(result.uploadErrors).toEqual([]);
		expect(result.uploaded[0]?.url).toBe("https://temporary.test/capture.png");
		expect(result.uploaded[0]?.data).toBeUndefined();
	});

	test("web attachments retain only the contiguous prefix after an upload failure", async () => {
		let uploadCalls = 0;
		const backend = {
			helperState: {
				fileToTemporaryFile: async () => {
					uploadCalls += 1;
					if (uploadCalls === 2) throw new Error("temporary storage failed");
					return {
						url: `https://temporary.test/capture-${uploadCalls}.png`,
					};
				},
			},
		} as never;
		const images = ["first", "second", "third"].map((contents) => ({
			blob: new Blob([contents], { type: "image/png" }),
			mediaType: "image/png",
		}));

		const result = await uploadPageSnapshots(backend, images);

		expect(uploadCalls).toBe(2);
		expect(result.uploaded).toHaveLength(1);
		expect(result.uploaded[0]?.url).toBe(
			"https://temporary.test/capture-1.png",
		);
		expect(result.uploadErrors[0]).toContain(
			"Capture 2: temporary storage failed",
		);
		expect(result.uploadErrors[1]).toContain(
			"1 later capture attachment(s) were skipped",
		);
	});
});
