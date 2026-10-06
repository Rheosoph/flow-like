import {
	type FormSessionState,
	LAYOUT,
	type WorkbenchLayout,
} from "../contracts";

/*
 * Layouts for tests and the visual harness (PLAN §7, §10). The shell measures its own box; tests and
 * the harness have no layout engine, so they hand the shell (and every view) one of these instead.
 */

export interface PointerOptions {
	/** `(pointer: coarse)`. Default false. */
	readonly touch?: boolean;
	/** `(pointer: fine)`. Default: the opposite of `touch`. */
	readonly finePointer?: boolean;
}

/** The layout the shell would report for an interface box of this size: split from 900 px, compare from a 600 px stage. */
export function layoutFor(
	width: number,
	height: number,
	pointer: PointerOptions = {},
): WorkbenchLayout {
	const touch = pointer.touch ?? false;
	const split = width >= LAYOUT.splitMinWidth;
	return {
		width,
		height,
		split,
		touch,
		finePointer: pointer.finePointer ?? !touch,
		compare: split && width - LAYOUT.railWidth >= LAYOUT.compareMinStageWidth,
	};
}

export type ArtboardDevice = "desktop" | "phone";

/**
 * The canvas artboards (SURFACE §2) and the host chrome around the interface box: the desktop host
 * header is 64 px; a phone has the floating mobile header (about 60 px) and the 56 px tab bar.
 */
export const ARTBOARDS = {
	desktop: { width: 1360, height: 900, headerHeight: 64, tabBarHeight: 0 },
	phone: { width: 390, height: 844, headerHeight: 60, tabBarHeight: 56 },
} as const;

/** Hosts swap their header for the mobile header below this width (SURFACE §2). */
export const MOBILE_HOST_BELOW = 768;

export function deviceOf(frameWidth: number): ArtboardDevice {
	return frameWidth < MOBILE_HOST_BELOW ? "phone" : "desktop";
}

/** The interface box inside a frame of this size: the frame minus the host header (and a phone's tab bar). */
export function interfaceBox(frameWidth: number, frameHeight: number) {
	const chrome = ARTBOARDS[deviceOf(frameWidth)];
	return {
		width: frameWidth,
		height: frameHeight - chrome.headerHeight - chrome.tabBarHeight,
	};
}

/** The interface's layout inside a frame of this size; a phone frame has a coarse pointer unless told otherwise. */
export function artboardLayout(
	frameWidth: number,
	frameHeight: number,
	pointer: PointerOptions = {},
): WorkbenchLayout {
	const box = interfaceBox(frameWidth, frameHeight);
	return layoutFor(box.width, box.height, {
		touch: pointer.touch ?? deviceOf(frameWidth) === "phone",
		finePointer: pointer.finePointer,
	});
}

/** 1360 × 836: the interface below the 64 px header of the 1360 × 900 artboard, fine pointer. */
export const DESKTOP_LAYOUT = artboardLayout(
	ARTBOARDS.desktop.width,
	ARTBOARDS.desktop.height,
);

/** 390 × 728: the interface between the mobile header and the tab bar of the 390 × 844 artboard, coarse pointer. */
export const PHONE_LAYOUT = artboardLayout(
	ARTBOARDS.phone.width,
	ARTBOARDS.phone.height,
);

/** A state as the shell sees it once it has measured this layout. */
export function withLayout(
	state: FormSessionState,
	layout: WorkbenchLayout,
): FormSessionState {
	return { ...state, layout };
}
