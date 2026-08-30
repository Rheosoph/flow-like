/**
 * Captures an inline FlowPilot app page for vision-capable agents.
 *
 * The page itself scrolls inside the inline card, so capturing the visible card only
 * misses most long pages. We rasterize its registered page container as bounded vertical
 * slices, which avoids allocating one browser canvas at the full content height.
 */

import {
	type LivePageHandle,
	findLivePage,
} from "../components/a2ui/live-page-registry";
import type { IHelperState } from "../state/backend-state/helper-state";
import { isTauri } from "./platform";

const INLINE_PAGE_ATTRIBUTE = "data-flowpilot-inline-page";
const CAPTURE_TARGET_ATTRIBUTE = "data-flowpilot-capture-target";
export const INLINE_PAGE_REVEAL_EVENT = "flowpilot:inline-page-reveal";
const DEFAULT_RENDER_TIMEOUT_MS = 30_000;
const DIRECT_CAPTURE_SETTLE_TIMEOUT_MS = 2_000;
const PAINT_FALLBACK_TIMEOUT_MS = 120;
const MAX_CAPTURE_WIDTH_PX = 1_600;
const MAX_CAPTURE_PIXELS = 16_000_000;
const MAX_SEGMENT_HEIGHT_PX = 2_000;
const MAX_SEGMENTS = 12;
const MAX_INLINE_IMAGE_BYTES = 8 * 1024 * 1024;
const MAX_INLINE_TOTAL_BYTES = 24 * 1024 * 1024;

export interface AppPageSnapshot {
	blob: Blob;
	mediaType: string;
}

export interface AppPageSnapshotResult {
	images: AppPageSnapshot[];
	complete: boolean;
	totalHeight: number;
	/** Why a capture failed or contains only a top-to-bottom prefix of the page. */
	failureReason?: string;
	/** Exact live instance used for this capture. Kept private to the frontend tool executor. */
	source?: AppPageSnapshotSource;
}

export interface AppPageSnapshotSource {
	handle: LivePageHandle;
	element: HTMLElement;
}

export interface UploadedPageSnapshot {
	/** Remote web deployments pass a temporary HTTP URL. */
	url?: string;
	/** Desktop passes raw base64 so local captures do not depend on remote storage. */
	data?: string;
	media_type: string;
}

async function blobToBase64(blob: Blob): Promise<string> {
	const bytes = new Uint8Array(await blob.arrayBuffer());
	const chunks: string[] = [];
	const chunkSize = 0x8000;
	for (let offset = 0; offset < bytes.length; offset += chunkSize) {
		chunks.push(
			String.fromCharCode(...bytes.subarray(offset, offset + chunkSize)),
		);
	}
	return btoa(chunks.join(""));
}

/**
 * Turns capture segments into provider attachments. Desktop keeps the bytes local and sends
 * bounded base64 through Tauri. Web deployments retain remote temporary URLs so large binary
 * payloads do not pass through the tool result channel push.
 */
export async function uploadPageSnapshots(
	backend: { helperState: IHelperState },
	images: AppPageSnapshot[],
): Promise<{ uploaded: UploadedPageSnapshot[]; uploadErrors: string[] }> {
	const uploaded: UploadedPageSnapshot[] = [];
	const uploadErrors: string[] = [];
	const capturePrefix = images.slice(0, MAX_SEGMENTS);
	const recordSkippedTail = (failedIndex: number) => {
		const skippedCount = capturePrefix.length - failedIndex - 1;
		if (skippedCount > 0) {
			uploadErrors.push(
				`${skippedCount} later capture attachment(s) were skipped after capture ${failedIndex + 1} failed, preserving a contiguous top-to-bottom prefix.`,
			);
		}
	};
	if (isTauri()) {
		let inlineBytes = 0;
		for (const [index, image] of capturePrefix.entries()) {
			if (image.blob.size > MAX_INLINE_IMAGE_BYTES) {
				uploadErrors.push(
					`Capture ${index + 1} is ${image.blob.size} bytes, above the ${MAX_INLINE_IMAGE_BYTES}-byte desktop attachment limit.`,
				);
				recordSkippedTail(index);
				break;
			}
			if (inlineBytes + image.blob.size > MAX_INLINE_TOTAL_BYTES) {
				uploadErrors.push(
					`Capture ${index + 1} would exceed the ${MAX_INLINE_TOTAL_BYTES}-byte desktop attachment budget.`,
				);
				recordSkippedTail(index);
				break;
			}
			try {
				const data = await blobToBase64(image.blob);
				uploaded.push({ data, media_type: image.mediaType });
				inlineBytes += image.blob.size;
			} catch (error) {
				uploadErrors.push(
					`Capture ${index + 1} could not be encoded for desktop attachment: ${error instanceof Error ? error.message : String(error)}`,
				);
				recordSkippedTail(index);
				break;
			}
		}
		if (images.length > MAX_SEGMENTS) {
			uploadErrors.push(
				`Only ${MAX_SEGMENTS} capture attachments are allowed; ${images.length - MAX_SEGMENTS} were skipped.`,
			);
		}
		return { uploaded, uploadErrors };
	}

	for (const [index, image] of capturePrefix.entries()) {
		const extension =
			image.mediaType === "image/webp"
				? "webp"
				: image.mediaType === "image/jpeg"
					? "jpg"
					: "png";
		const file = new File(
			[image.blob],
			`flowpilot-page-${index + 1}.${extension}`,
			{ type: image.mediaType },
		);
		try {
			const temporaryFile = backend.helperState.fileToTemporaryFile
				? await backend.helperState.fileToTemporaryFile(
						file,
						false,
						undefined,
						"remote",
					)
				: {
						url: await backend.helperState.fileToUrl(
							file,
							false,
							undefined,
							"remote",
						),
					};
			if (!/^https?:\/\//i.test(temporaryFile.url)) {
				throw new Error(
					"Temporary upload did not return a remotely readable URL.",
				);
			}
			uploaded.push({
				url: temporaryFile.url,
				media_type: image.mediaType,
			});
		} catch (error) {
			console.warn("[AppPageSnapshot] failed to upload page capture", error);
			uploadErrors.push(
				`Capture ${index + 1}: ${error instanceof Error ? error.message : String(error)}`,
			);
			recordSkippedTail(index);
			break;
		}
	}
	if (images.length > MAX_SEGMENTS) {
		uploadErrors.push(
			`Only ${MAX_SEGMENTS} capture attachments are allowed; ${images.length - MAX_SEGMENTS} were skipped.`,
		);
	}
	return { uploaded, uploadErrors };
}

export function inlineAppPageSnapshotAttribute(
	appId: string,
	eventId?: string,
): { [INLINE_PAGE_ATTRIBUTE]: string } {
	return { [INLINE_PAGE_ATTRIBUTE]: `${appId}:${eventId ?? ""}` };
}

function delay(timeoutMs: number): Promise<void> {
	return new Promise((resolve) => setTimeout(resolve, timeoutMs));
}

/** Wait for two animation frames, with a timer fallback for hidden or minimized WebViews. */
export function waitForCapturePaint(deadlineAt: number): Promise<void> {
	return new Promise((resolve) => {
		let settled = false;
		const frameIds: number[] = [];
		const finish = () => {
			if (settled) return;
			settled = true;
			clearTimeout(timer);
			if (typeof window.cancelAnimationFrame === "function") {
				for (const id of frameIds) window.cancelAnimationFrame(id);
			}
			resolve();
		};
		const remaining = Math.max(0, deadlineAt - Date.now());
		const timer = setTimeout(
			finish,
			Math.min(PAINT_FALLBACK_TIMEOUT_MS, remaining),
		);
		if (typeof window.requestAnimationFrame !== "function" || remaining === 0) {
			return;
		}
		frameIds.push(
			window.requestAnimationFrame(() => {
				frameIds.push(window.requestAnimationFrame(finish));
			}),
		);
	});
}

async function waitWithinDeadline(
	work: Promise<unknown>,
	deadlineAt: number,
): Promise<void> {
	const remaining = deadlineAt - Date.now();
	if (remaining <= 0) return;
	await Promise.race([work.catch(() => undefined), delay(remaining)]);
}

async function settlePageResources(
	element: HTMLElement,
	deadlineAt: number,
): Promise<void> {
	const fontReady = element.ownerDocument.fonts?.ready;
	if (fontReady) await waitWithinDeadline(fontReady, deadlineAt);
	const pendingImages = Array.from(element.querySelectorAll("img"))
		.filter((image) => !image.complete)
		.map((image) =>
			typeof image.decode === "function"
				? image.decode()
				: new Promise<void>((resolve) => {
						image.addEventListener("load", () => resolve(), { once: true });
						image.addEventListener("error", () => resolve(), { once: true });
					}),
		);
	if (pendingImages.length > 0) {
		await waitWithinDeadline(Promise.allSettled(pendingImages), deadlineAt);
	}
	await waitForCapturePaint(deadlineAt);
}

async function waitForRenderedPage(
	appId: string,
	eventId: string | undefined,
	timeoutMs: number,
): Promise<AppPageSnapshotSource | null> {
	const deadline = Date.now() + timeoutMs;
	while (true) {
		const handle = findLivePage(appId, { eventId });
		const pageCanvas = handle?.getContainer?.() ?? null;
		if (
			handle &&
			pageCanvas &&
			!handle.isLoading() &&
			pageCanvas.dataset.flowpilotPageLoading !== "true" &&
			pageCanvas.isConnected &&
			pageCanvas.getBoundingClientRect().width > 0
		) {
			await settlePageResources(pageCanvas, deadline);
			const currentHandle = findLivePage(appId, { eventId });
			if (
				currentHandle &&
				pageCanvas.isConnected &&
				currentHandle === handle &&
				currentHandle.getContainer?.() === pageCanvas &&
				!currentHandle.isLoading() &&
				pageCanvas.dataset.flowpilotPageLoading !== "true"
			) {
				return { handle, element: pageCanvas };
			}
		}
		const remaining = deadline - Date.now();
		if (remaining <= 0) break;
		await delay(Math.min(100, remaining));
	}
	return null;
}

/** True only while the exact page instance used for a capture remains the selected live page. */
export function isAppPageSnapshotSourceCurrent(
	source: AppPageSnapshotSource | undefined,
	appId: string,
	eventId?: string,
): boolean {
	if (!source?.element.isConnected) return false;
	const current = findLivePage(appId, {
		eventId,
		pageId: source.handle.pageId,
	});
	return (
		current === source.handle && current.getContainer?.() === source.element
	);
}

function canvasToSnapshot(
	canvas: HTMLCanvasElement,
	mediaType: string,
	quality?: number,
): Promise<AppPageSnapshot | null> {
	return new Promise((resolve) => {
		canvas.toBlob(
			(blob) =>
				resolve(
					blob
						? {
								blob,
								mediaType: blob.type || mediaType,
							}
						: null,
				),
			mediaType,
			quality,
		);
	});
}

function resolvedBackgroundColor(element: HTMLElement): string {
	let current: HTMLElement | null = element;
	while (current) {
		const color = window.getComputedStyle(current).backgroundColor;
		if (color && color !== "transparent" && color !== "rgba(0, 0, 0, 0)") {
			return color;
		}
		current = current.parentElement;
	}
	return "#ffffff";
}

function forceCaptureStyle(
	element: HTMLElement,
	property: string,
	value: string,
): void {
	element.style.setProperty(property, value, "important");
}

/** Expand only html2canvas's cloned tree, including a collapsed card's parked wrapper. */
export function normalizePageCaptureClone(
	clonedDocument: Document,
	captureMarker: string,
	width: number,
): void {
	const clonedPage = Array.from(clonedDocument.getElementsByTagName("*")).find(
		(candidate) =>
			candidate.getAttribute(CAPTURE_TARGET_ATTRIBUTE) === captureMarker,
	) as HTMLElement | undefined;
	if (!clonedPage) return;

	forceCaptureStyle(clonedPage, "visibility", "visible");
	forceCaptureStyle(clonedPage, "opacity", "1");
	forceCaptureStyle(clonedPage, "content-visibility", "visible");
	forceCaptureStyle(clonedPage, "height", "auto");
	forceCaptureStyle(clonedPage, "max-height", "none");
	forceCaptureStyle(clonedPage, "overflow", "visible");
	forceCaptureStyle(clonedPage, "width", `${width}px`);

	let ancestor: HTMLElement | null = clonedPage.parentElement;
	while (ancestor) {
		ancestor.removeAttribute("hidden");
		ancestor.removeAttribute("inert");
		ancestor.removeAttribute("aria-hidden");
		forceCaptureStyle(ancestor, "contain", "none");
		forceCaptureStyle(ancestor, "height", "auto");
		forceCaptureStyle(ancestor, "max-height", "none");
		forceCaptureStyle(ancestor, "overflow", "visible");
		forceCaptureStyle(ancestor, "visibility", "visible");
		forceCaptureStyle(ancestor, "opacity", "1");
		forceCaptureStyle(ancestor, "content-visibility", "visible");
		forceCaptureStyle(ancestor, "clip", "auto");
		forceCaptureStyle(ancestor, "clip-path", "none");

		const computedDisplay =
			clonedDocument.defaultView?.getComputedStyle(ancestor).display;
		if (computedDisplay === "none")
			forceCaptureStyle(ancestor, "display", "block");

		if (ancestor.hasAttribute("data-flowpilot-page-parking")) {
			forceCaptureStyle(ancestor, "position", "absolute");
			forceCaptureStyle(ancestor, "inset", "auto");
			forceCaptureStyle(ancestor, "left", "0");
			forceCaptureStyle(ancestor, "top", "0");
			forceCaptureStyle(ancestor, "right", "auto");
			forceCaptureStyle(ancestor, "bottom", "auto");
			forceCaptureStyle(ancestor, "transform", "none");
			forceCaptureStyle(ancestor, "width", `${width}px`);
			forceCaptureStyle(ancestor, "pointer-events", "none");
		}

		if (ancestor === clonedDocument.documentElement) break;
		ancestor = ancestor.parentElement;
	}
}

/**
 * Wait until the real page (including its on-load state) replaces the loading UI, then
 * capture the complete scrollable page. Long pages are returned top-to-bottom as several
 * image attachments so text stays legible to the model.
 */
export async function captureInlineAppPageSnapshots(
	appId: string,
	eventId?: string,
	timeoutMs = DEFAULT_RENDER_TIMEOUT_MS,
): Promise<AppPageSnapshotResult> {
	const source = await waitForRenderedPage(appId, eventId, timeoutMs);
	if (!source)
		return {
			images: [],
			complete: false,
			totalHeight: 0,
			failureReason: `The page runtime did not finish rendering within ${Math.round(timeoutMs / 1000)}s (its on-load workflow may still be running, or no matching page instance is registered).`,
		};
	const result = await capturePageElementSnapshots(
		source.element,
		appId,
		eventId,
	);
	return { ...result, source };
}

/**
 * Rasterize one exact page container. interact_app_page uses this with the DRIVEN instance's own
 * element so the evidence can never come from a different live render of the same page.
 */
export async function capturePageElementSnapshots(
	element: HTMLElement,
	appId: string,
	eventId?: string,
): Promise<AppPageSnapshotResult> {
	try {
		await settlePageResources(
			element,
			Date.now() + DIRECT_CAPTURE_SETTLE_TIMEOUT_MS,
		);
		if (!element.isConnected) {
			return {
				images: [],
				complete: false,
				totalHeight: 0,
				failureReason: "The selected page instance unmounted before capture.",
			};
		}
		const { default: html2canvas } = await import("html2canvas-pro");
		const width = Math.ceil(
			Math.max(element.scrollWidth, element.clientWidth, 1),
		);
		const totalHeight = Math.ceil(
			Math.max(element.scrollHeight, element.clientHeight, 1),
		);
		const backgroundColor = resolvedBackgroundColor(element);
		const preferredScale = Math.min(window.devicePixelRatio || 1, 2);
		const widthScale = MAX_CAPTURE_WIDTH_PX / width;
		const budgetScale = Math.sqrt(MAX_CAPTURE_PIXELS / (width * totalHeight));
		// Keep a readable lower bound when height alone would force a thumbnail-scale image.
		// Very long pages become an explicitly partial capture after MAX_SEGMENTS instead.
		const scale = Math.min(
			preferredScale,
			widthScale,
			Math.max(Math.min(0.5, widthScale), budgetScale),
		);
		const segmentHeight = Math.max(
			1,
			Math.floor(MAX_SEGMENT_HEIGHT_PX / scale),
		);
		const totalSegments = Math.max(1, Math.ceil(totalHeight / segmentHeight));
		const segmentLimit = Math.min(MAX_SEGMENTS, totalSegments);
		const largeCapture = width * totalHeight * scale * scale > 8_000_000;

		const captureMarker = `${appId}:${eventId ?? ""}:${Date.now()}:${Math.random()}`;
		element.setAttribute(CAPTURE_TARGET_ATTRIBUTE, captureMarker);
		const images: AppPageSnapshot[] = [];
		let segmentFailure: string | undefined;
		try {
			for (let index = 0; index < segmentLimit; index += 1) {
				const top = index * segmentHeight;
				const height = Math.min(segmentHeight, totalHeight - top);
				try {
					// Rasterize each vertical slice directly. Building a full-page canvas first
					// exceeds WebKit canvas dimensions and memory on otherwise valid long pages.
					const canvas = await html2canvas(element, {
						backgroundColor,
						height,
						imageTimeout: 3_000,
						logging: false,
						onclone: (clonedDocument) =>
							normalizePageCaptureClone(clonedDocument, captureMarker, width),
						scale,
						useCORS: true,
						width,
						windowHeight: Math.max(window.innerHeight || 0, height),
						windowWidth: width,
						y: top,
					});
					const snapshot = await canvasToSnapshot(
						canvas,
						largeCapture ? "image/webp" : "image/png",
						largeCapture ? 0.9 : undefined,
					);
					if (!snapshot) {
						throw new Error("the browser returned an empty encoded image");
					}
					images.push(snapshot);
				} catch (error) {
					segmentFailure = `Capture segment ${index + 1} of ${totalSegments} failed: ${error instanceof Error ? error.message : String(error)}`;
					break;
				}
			}
		} finally {
			if (element.getAttribute(CAPTURE_TARGET_ATTRIBUTE) === captureMarker) {
				element.removeAttribute(CAPTURE_TARGET_ATTRIBUTE);
			}
		}
		const truncated = totalSegments > MAX_SEGMENTS;
		const partialReasons = [
			segmentFailure,
			truncated
				? `The page needs ${totalSegments} capture segments; only the first ${MAX_SEGMENTS} are attached to keep the tool result bounded.`
				: undefined,
		].filter((reason): reason is string => Boolean(reason));

		return {
			images,
			complete:
				!segmentFailure && !truncated && images.length === totalSegments,
			totalHeight,
			...(partialReasons.length > 0
				? { failureReason: partialReasons.join(" ") }
				: {}),
		};
	} catch (error) {
		console.warn("[AppPageSnapshot] capture failed:", error);
		return {
			images: [],
			complete: false,
			totalHeight: 0,
			failureReason: `Rasterizing the rendered page failed: ${error instanceof Error ? error.message : String(error)}`,
		};
	}
}
