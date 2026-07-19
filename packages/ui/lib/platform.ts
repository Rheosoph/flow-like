/**
 * Platform detection utilities
 */

/**
 * Detect Tauri environment (WebView)
 * Checks for common Tauri globals:
 * - __TAURI__ in Tauri v1
 * - __TAURI_INTERNALS__ / __TAURI_IPC__ in some builds
 */
export const isTauri = (): boolean => {
	if (typeof window === "undefined") return false;
	const w = window as any;
	return !!(w.__TAURI__ || w.__TAURI_IPC__ || w.__TAURI_INTERNALS__);
};

let _webkitLite: boolean | undefined;

/**
 * True on WebKit engines that rasterize blurred shadows, CSS filters, and
 * oklch/color-mix dramatically slower than Blink: macOS WKWebView (Tauri),
 * Linux WebKitGTK, and desktop Safari. False on Chromium (Chrome, Edge,
 * Windows WebView2, Opera).
 *
 * Used to gate expensive decorative styling in the flow board (per-pin/per-node
 * blurred glows, oklch gradients, infinite edge animations) so heavy boards stay
 * fluid on WebKit while Chromium keeps the full look.
 */
export const isWebkitLite = (): boolean => {
	if (typeof navigator === "undefined") return false;
	if (_webkitLite !== undefined) return _webkitLite;
	const ua = navigator.userAgent;
	_webkitLite = /AppleWebKit/.test(ua) && !/Chrome|Chromium|Edg|OPR/.test(ua);
	return _webkitLite;
};
