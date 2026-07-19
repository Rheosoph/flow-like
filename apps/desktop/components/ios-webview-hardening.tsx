"use client";

import { useEffect } from "react";
import { isMobileDevice, isTauriRuntime } from "../lib/platform";

const MOBILE_VIEWPORT_CONTENT =
	"width=device-width, initial-scale=1, maximum-scale=1, viewport-fit=cover, interactive-widget=resizes-content";
const MAX_SAFE_TOP_PX = 96;
const MAX_SAFE_BOTTOM_PX = 64;
const POLL_MAX_RETRIES = 40;
const POLL_INTERVAL_MS = 50;

function upsertViewportMeta(content: string) {
	let meta = document.querySelector(
		'meta[name="viewport"]',
	) as HTMLMetaElement | null;

	if (!meta) {
		meta = document.createElement("meta");
		meta.name = "viewport";
		document.head.appendChild(meta);
	}

	meta.setAttribute("content", content);
}

function clamp(value: number, min: number, max: number): number {
	return Math.min(Math.max(value, min), max);
}

let appliedSafeTop = 0;
let appliedSafeBottom = 0;

/**
 * Probe CSS env(safe-area-inset-*) by measuring a hidden element.
 * This is more reliable than visualViewport.offsetTop which is always 0
 * when contentInsetAdjustmentBehavior = .never on the native side.
 */
function probeCSSEnvInsets(): { top: number; bottom: number } {
	const probe = document.createElement("div");
	probe.style.cssText = [
		"position:fixed",
		"left:-9999px",
		"top:0",
		"width:1px",
		"height:1px",
		"padding-top:env(safe-area-inset-top, 0px)",
		"padding-bottom:env(safe-area-inset-bottom, 0px)",
		"visibility:hidden",
		"pointer-events:none",
	].join(";");
	document.body.appendChild(probe);
	const cs = getComputedStyle(probe);
	const top = Math.round(Number.parseFloat(cs.paddingTop) || 0);
	const bottom = Math.round(Number.parseFloat(cs.paddingBottom) || 0);
	probe.remove();
	return { top, bottom };
}

/**
 * Read values injected by native code. On Android, prefer the synchronous
 * JavascriptInterface bridge (FlowLikeInsets) since evaluateJavascript values
 * may be wiped between about:blank and the real page load.
 */
function nativeInsets(): { top: number; bottom: number } {
	const w = window as Window & {
		__FL_NATIVE_SAFE_TOP?: number;
		__FL_NATIVE_SAFE_BOTTOM?: number;
		FlowLikeInsets?: { getTopPx(): number; getBottomPx(): number };
	};

	let top =
		typeof w.__FL_NATIVE_SAFE_TOP === "number" ? w.__FL_NATIVE_SAFE_TOP : 0;
	let bottom =
		typeof w.__FL_NATIVE_SAFE_BOTTOM === "number"
			? w.__FL_NATIVE_SAFE_BOTTOM
			: 0;

	if (w.FlowLikeInsets) {
		try {
			const dpr = window.devicePixelRatio || 1;
			const bridgeTop = Math.ceil(w.FlowLikeInsets.getTopPx() / dpr);
			const bridgeBottom = Math.ceil(w.FlowLikeInsets.getBottomPx() / dpr);
			top = Math.max(top, bridgeTop);
			bottom = Math.max(bottom, bridgeBottom);
		} catch {
			/* bridge not ready yet */
		}
	}

	return { top, bottom };
}

function applySafeAreaInsets() {
	const env = probeCSSEnvInsets();
	const native = nativeInsets();

	const top = clamp(Math.max(env.top, native.top), 0, MAX_SAFE_TOP_PX);
	const bottom = clamp(
		Math.max(env.bottom, native.bottom),
		0,
		MAX_SAFE_BOTTOM_PX,
	);

	if (top > appliedSafeTop || (appliedSafeTop === 0 && top > 0)) {
		appliedSafeTop = top;
	}
	if (bottom > appliedSafeBottom || (appliedSafeBottom === 0 && bottom > 0)) {
		appliedSafeBottom = bottom;
	}

	document.documentElement.style.setProperty(
		"--fl-native-safe-top",
		`${appliedSafeTop}px`,
	);
	document.documentElement.style.setProperty(
		"--fl-native-safe-bottom",
		`${appliedSafeBottom}px`,
	);

	return appliedSafeTop > 0 || appliedSafeBottom > 0;
}

/**
 * Poll until CSS env() or native insets provide non-zero values.
 * WebKit bug #183106: env() values are 0px on first render and only
 * populate after a layout pass completes.
 */
function pollForInsets() {
	let retries = 0;
	const tick = () => {
		if (applySafeAreaInsets()) return;
		if (++retries < POLL_MAX_RETRIES) {
			setTimeout(tick, POLL_INTERVAL_MS);
		}
	};
	requestAnimationFrame(tick);
}

function syncViewportHeight() {
	const vv = window.visualViewport;
	const viewportHeight = Math.round(vv?.height ?? window.innerHeight);
	document.documentElement.style.setProperty(
		"--fl-mobile-vvh",
		`${viewportHeight}px`,
	);
}

export function IOSWebviewHardening() {
	useEffect(() => {
		if (!isTauriRuntime() || !isMobileDevice()) return;

		upsertViewportMeta(MOBILE_VIEWPORT_CONTENT);
		applySafeAreaInsets();
		syncViewportHeight();
		pollForInsets();

		const handleOrientation = () => {
			appliedSafeTop = 0;
			appliedSafeBottom = 0;
			applySafeAreaInsets();
			syncViewportHeight();
			pollForInsets();
		};

		// Re-apply on Next.js client-side navigation (pushState / replaceState / popstate).
		const origPushState = history.pushState.bind(history);
		const origReplaceState = history.replaceState.bind(history);

		const onNavigation = () => {
			requestAnimationFrame(() => {
				applySafeAreaInsets();
				syncViewportHeight();
			});
		};

		history.pushState = (...args: Parameters<typeof origPushState>) => {
			origPushState(...args);
			onNavigation();
		};
		history.replaceState = (...args: Parameters<typeof origReplaceState>) => {
			origReplaceState(...args);
			onNavigation();
		};

		window.addEventListener("popstate", onNavigation);
		window.visualViewport?.addEventListener("resize", syncViewportHeight);
		window.visualViewport?.addEventListener("scroll", syncViewportHeight);
		window.addEventListener("orientationchange", handleOrientation);
		window.addEventListener("resize", syncViewportHeight);

		return () => {
			history.pushState = origPushState;
			history.replaceState = origReplaceState;
			window.removeEventListener("popstate", onNavigation);
			window.visualViewport?.removeEventListener("resize", syncViewportHeight);
			window.visualViewport?.removeEventListener("scroll", syncViewportHeight);
			window.removeEventListener("orientationchange", handleOrientation);
			window.removeEventListener("resize", syncViewportHeight);
		};
	}, []);

	return null;
}
