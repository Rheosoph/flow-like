import { captureFileName } from "./capture-file-name";

export const PAGE_SNAPSHOT_IGNORE_ATTRIBUTE = "data-global-chat-overlay";

const MAX_EDGE_PX = 1600;

export function pageSnapshotFileName(date: Date = new Date()): string {
	return captureFileName("page", date, ".png");
}

function canvasToPng(canvas: HTMLCanvasElement): Promise<Blob> {
	return new Promise((resolve, reject) => {
		canvas.toBlob(
			(blob) =>
				blob ? resolve(blob) : reject(new Error("PNG encoding failed")),
			"image/png",
		);
	});
}

/** Rasterizes the visible viewport as a PNG, leaving the global chat overlay out. */
export async function capturePageSnapshot(): Promise<File> {
	const { default: html2canvas } = await import("html2canvas-pro");
	const width = window.innerWidth;
	const height = window.innerHeight;
	const canvas = await html2canvas(document.documentElement, {
		backgroundColor: null,
		scale: Math.min(1, MAX_EDGE_PX / Math.max(width, height)),
		width,
		height,
		x: window.scrollX,
		y: window.scrollY,
		logging: false,
		useCORS: true,
		ignoreElements: (element) =>
			element.hasAttribute(PAGE_SNAPSHOT_IGNORE_ATTRIBUTE),
	});
	const blob = await canvasToPng(canvas);
	return new File([blob], pageSnapshotFileName(), { type: "image/png" });
}
