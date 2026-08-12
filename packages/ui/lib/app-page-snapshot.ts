/**
 * Captures an inline FlowPilot app page for vision-capable agents.
 *
 * The page itself scrolls inside the inline card, so capturing the visible card only
 * misses most long pages. We rasterize the page canvas (`[data-page-id]`) at its full
 * content height and split the result into readable vertical images.
 */

const INLINE_PAGE_ATTRIBUTE = "data-flowpilot-inline-page";
const CAPTURE_TARGET_ATTRIBUTE = "data-flowpilot-capture-target";
export const INLINE_PAGE_REVEAL_EVENT = "flowpilot:inline-page-reveal";
const DEFAULT_RENDER_TIMEOUT_MS = 30_000;
const MAX_CAPTURE_WIDTH_PX = 1_600;
const MAX_CAPTURE_PIXELS = 16_000_000;
const MAX_SEGMENT_HEIGHT_PX = 2_000;
const MAX_SEGMENTS = 12;

export interface AppPageSnapshot {
	blob: Blob;
	mediaType: string;
}

export interface AppPageSnapshotResult {
	images: AppPageSnapshot[];
	complete: boolean;
	totalHeight: number;
}

export function inlineAppPageSnapshotAttribute(
	appId: string,
	eventId?: string,
): { [INLINE_PAGE_ATTRIBUTE]: string } {
	return { [INLINE_PAGE_ATTRIBUTE]: `${appId}:${eventId ?? ""}` };
}

function inlinePageSelector(appId: string, eventId?: string): string {
	return `[${INLINE_PAGE_ATTRIBUTE}="${CSS.escape(`${appId}:${eventId ?? ""}`)}"]`;
}

function nextPaint(): Promise<void> {
	return new Promise((resolve) =>
		requestAnimationFrame(() => requestAnimationFrame(() => resolve())),
	);
}

async function waitForRenderedPage(
	appId: string,
	eventId: string | undefined,
	timeoutMs: number,
): Promise<HTMLElement | null> {
	window.dispatchEvent(
		new CustomEvent(INLINE_PAGE_REVEAL_EVENT, {
			detail: { appId, eventId },
		}),
	);
	await nextPaint();
	const deadline = Date.now() + timeoutMs;
	while (Date.now() < deadline) {
		const root = document.querySelector<HTMLElement>(
			inlinePageSelector(appId, eventId),
		);
		const pageCanvas = root?.querySelector<HTMLElement>("[data-page-id]");
		if (
			pageCanvas &&
			(!eventId || pageCanvas.dataset.flowpilotPageEventId === eventId) &&
			pageCanvas.dataset.flowpilotPageLoading !== "true" &&
			pageCanvas.getBoundingClientRect().width > 0
		) {
			await document.fonts?.ready.catch(() => undefined);
			await nextPaint();
			if (
				pageCanvas.isConnected &&
				(!eventId || pageCanvas.dataset.flowpilotPageEventId === eventId) &&
				pageCanvas.dataset.flowpilotPageLoading !== "true"
			) {
				return pageCanvas;
			}
		}
		await new Promise((resolve) => setTimeout(resolve, 100));
	}
	return null;
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

async function splitCanvas(
	canvas: HTMLCanvasElement,
): Promise<AppPageSnapshot[]> {
	const segmentCount = Math.max(
		1,
		Math.min(MAX_SEGMENTS, Math.ceil(canvas.height / MAX_SEGMENT_HEIGHT_PX)),
	);
	const segmentHeight = Math.ceil(canvas.height / segmentCount);
	const images: AppPageSnapshot[] = [];
	const largeCapture = canvas.width * canvas.height > 8_000_000;

	for (let top = 0; top < canvas.height; top += segmentHeight) {
		const height = Math.min(segmentHeight, canvas.height - top);
		const segment = document.createElement("canvas");
		segment.width = canvas.width;
		segment.height = height;
		const context = segment.getContext("2d");
		if (!context) continue;
		context.drawImage(
			canvas,
			0,
			top,
			canvas.width,
			height,
			0,
			0,
			canvas.width,
			height,
		);
		const snapshot = await canvasToSnapshot(
			segment,
			largeCapture ? "image/webp" : "image/png",
			largeCapture ? 0.92 : undefined,
		);
		if (snapshot) images.push(snapshot);
	}

	return images;
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
	const element = await waitForRenderedPage(appId, eventId, timeoutMs);
	if (!element) return { images: [], complete: false, totalHeight: 0 };

	try {
		const { default: html2canvas } = await import("html2canvas-pro");
		const width = Math.max(element.scrollWidth, element.clientWidth, 1);
		const totalHeight = Math.max(element.scrollHeight, element.clientHeight, 1);
		const backgroundColor = resolvedBackgroundColor(element);
		const preferredScale = Math.min(window.devicePixelRatio || 1, 2);
		// Keep the entire page in the capture. Exceptionally long pages are scaled down
		// to stay within browser canvas/provider limits instead of silently truncating
		// content below the inline card's viewport.
		const scale = Math.min(
			preferredScale,
			MAX_CAPTURE_WIDTH_PX / width,
			(MAX_SEGMENT_HEIGHT_PX * MAX_SEGMENTS) / totalHeight,
			Math.sqrt(MAX_CAPTURE_PIXELS / (width * totalHeight)),
		);

		const captureMarker = `${appId}:${eventId ?? ""}:${Date.now()}:${Math.random()}`;
		element.setAttribute(CAPTURE_TARGET_ATTRIBUTE, captureMarker);
		let canvas: HTMLCanvasElement;
		try {
			canvas = await html2canvas(element, {
				backgroundColor,
				height: totalHeight,
				logging: false,
				onclone: (clonedDocument) => {
					const clonedPage = clonedDocument.querySelector<HTMLElement>(
						`[${CAPTURE_TARGET_ATTRIBUTE}="${CSS.escape(captureMarker)}"]`,
					);
					let ancestor = clonedPage?.parentElement;
					while (ancestor && ancestor !== clonedDocument.body) {
						// The live inline card must stay clipped and scrollable. Only its cloned
						// capture tree is expanded so html2canvas can paint off-screen content.
						ancestor.style.setProperty("contain", "none", "important");
						ancestor.style.setProperty("height", "auto", "important");
						ancestor.style.setProperty("max-height", "none", "important");
						ancestor.style.setProperty("overflow", "visible", "important");
						ancestor = ancestor.parentElement;
					}
				},
				scale,
				useCORS: true,
				width,
				windowHeight: totalHeight,
				windowWidth: width,
			});
		} finally {
			if (element.getAttribute(CAPTURE_TARGET_ATTRIBUTE) === captureMarker) {
				element.removeAttribute(CAPTURE_TARGET_ATTRIBUTE);
			}
		}
		const images = await splitCanvas(canvas);
		const expectedSegments = Math.max(
			1,
			Math.min(MAX_SEGMENTS, Math.ceil(canvas.height / MAX_SEGMENT_HEIGHT_PX)),
		);

		return {
			images,
			complete: images.length === expectedSegments,
			totalHeight,
		};
	} catch (error) {
		console.warn("[AppPageSnapshot] capture failed:", error);
		return { images: [], complete: false, totalHeight: 0 };
	}
}
