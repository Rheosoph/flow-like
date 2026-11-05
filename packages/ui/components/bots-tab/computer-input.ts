import type { IBotInputEvent } from "../../state/backend-state/bot-state";

export interface FrameSize {
	width: number;
	height: number;
}

export interface BoxRect {
	left: number;
	top: number;
	width: number;
	height: number;
}

/**
 * Maps a pointer position on an `object-fit: contain` image to pixels of the frame. Positions in
 * the letterbox bars are outside the picture: `null`, or the nearest edge pixel when `clamp`.
 */
export function mapToFrame(
	point: { clientX: number; clientY: number },
	box: BoxRect,
	frame: FrameSize,
	clamp = false,
): { x: number; y: number } | null {
	if (
		frame.width <= 0 ||
		frame.height <= 0 ||
		box.width <= 0 ||
		box.height <= 0
	)
		return null;
	const scale = Math.min(box.width / frame.width, box.height / frame.height);
	const drawnWidth = frame.width * scale;
	const drawnHeight = frame.height * scale;
	const left = box.left + (box.width - drawnWidth) / 2;
	const top = box.top + (box.height - drawnHeight) / 2;
	let x = (point.clientX - left) / scale;
	let y = (point.clientY - top) / scale;
	const inside = x >= 0 && y >= 0 && x <= frame.width && y <= frame.height;
	if (!inside && !clamp) return null;
	x = Math.min(Math.max(x, 0), frame.width - 1);
	y = Math.min(Math.max(y, 0), frame.height - 1);
	return { x: Math.round(x), y: Math.round(y) };
}

const NAMED_KEYS: Readonly<Record<string, string>> = {
	Enter: "Enter",
	Backspace: "Backspace",
	Tab: "Tab",
	Escape: "Escape",
	Delete: "Delete",
	ArrowUp: "ArrowUp",
	ArrowDown: "ArrowDown",
	ArrowLeft: "ArrowLeft",
	ArrowRight: "ArrowRight",
	Home: "Home",
	End: "End",
	PageUp: "PageUp",
	PageDown: "PageDown",
};

export interface KeyLike {
	key: string;
	ctrlKey: boolean;
	altKey: boolean;
	shiftKey: boolean;
	metaKey: boolean;
}

const MODIFIER_KEYS = new Set(["Control", "Alt", "Shift", "Meta", "CapsLock"]);

/**
 * A key press as the device takes it: typed characters are `text`, everything else (named keys,
 * shortcuts) is a `key` chord such as `Control+c`. Bare modifiers send nothing.
 */
export function keyToInput(event: KeyLike): IBotInputEvent | null {
	if (MODIFIER_KEYS.has(event.key)) return null;
	const named =
		NAMED_KEYS[event.key] ??
		(/^F([1-9]|1[0-2])$/.test(event.key) ? event.key : null);
	const shortcut = event.ctrlKey || event.altKey || event.metaKey;
	if (!named && !shortcut) {
		return [...event.key].length === 1
			? { type: "text", text: event.key }
			: null;
	}
	const key = named ?? (event.key === " " ? "Space" : event.key.toLowerCase());
	if (!named && [...event.key].length !== 1) return null;
	const parts = [
		event.ctrlKey && "Control",
		event.altKey && "Alt",
		event.shiftKey && (named || shortcut) && "Shift",
		event.metaKey && "Meta",
	].filter(Boolean);
	return { type: "key", chord: [...parts, key].join("+") };
}

export function wheelToInput(
	delta: { deltaX: number; deltaY: number },
	at: { x: number; y: number },
): IBotInputEvent {
	return {
		type: "scroll",
		x: at.x,
		y: at.y,
		dx: Math.round(delta.deltaX),
		dy: Math.round(delta.deltaY),
	};
}

export function buttonName(button: number): "left" | "middle" | "right" {
	return button === 1 ? "middle" : button === 2 ? "right" : "left";
}
