/**
 * Captures rendered chat-widget DOM subtrees as PNG data URLs so the model can
 * see the widget state the user is looking at. Same html2canvas-pro pattern as
 * the WidgetBuilder screenshot (proven against a2ui component trees).
 */

const WIDGET_ATTRIBUTE = "data-chat-widget-instance";

export function widgetSnapshotAttribute(instanceId: string): {
	[WIDGET_ATTRIBUTE]: string;
} {
	return { [WIDGET_ATTRIBUTE]: instanceId };
}

export async function captureWidgetSnapshot(
	element: HTMLElement,
): Promise<string | null> {
	try {
		const { default: html2canvas } = await import("html2canvas-pro");
		const canvas = await html2canvas(element, {
			backgroundColor: null,
			scale: Math.min(window.devicePixelRatio ?? 1, 2),
			logging: false,
			useCORS: true,
		});
		return canvas.toDataURL("image/png");
	} catch (error) {
		console.warn("[WidgetSnapshot] capture failed:", error);
		return null;
	}
}

/**
 * Capture snapshots for the given widget instance ids, in order. Instances
 * that are not currently rendered (or fail to rasterize) are skipped.
 */
export async function captureWidgetSnapshots(
	instanceIds: string[],
): Promise<string[]> {
	const snapshots = await Promise.all(
		instanceIds.map((instanceId) => {
			const element = document.querySelector<HTMLElement>(
				`[${WIDGET_ATTRIBUTE}="${CSS.escape(instanceId)}"]`,
			);
			return element ? captureWidgetSnapshot(element) : null;
		}),
	);
	return snapshots.filter((snapshot): snapshot is string => snapshot !== null);
}
