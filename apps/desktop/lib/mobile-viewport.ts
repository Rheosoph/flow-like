export interface VisualViewportMetrics {
	height: number;
	offsetTop: number;
	scale: number;
}

/**
 * Resolve the visible viewport height for browsers where the app owns soft
 * keyboard resizing (Android/Blink and ordinary touch browsers).
 *
 * Scale normalisation prevents pinch zoom from resizing the app shell. The
 * iOS/WKWebView shell deliberately does not consume this value: WebKit already
 * pans focused inputs into view, and combining that pan with a live pixel height
 * moves bottom-pinned chat composers twice.
 */
export function resolveMobileViewportHeight(
	viewport: VisualViewportMetrics | null | undefined,
	layoutViewportHeight: number,
): number {
	const layoutHeight = Math.max(0, Math.round(layoutViewportHeight));
	if (!viewport || !Number.isFinite(viewport.height)) return layoutHeight;

	const scale =
		Number.isFinite(viewport.scale) && viewport.scale > 0 ? viewport.scale : 1;
	const visibleHeight = Math.max(0, viewport.height) * scale;

	return Math.min(Math.round(visibleHeight), layoutHeight);
}
