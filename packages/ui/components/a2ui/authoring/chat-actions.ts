import type { SurfaceComponent } from "../types";
import { isJsonObject } from "./lower";

export const CHAT_SEND_MESSAGE_ACTION = "send_message";
/**
 * `external_link` is renamed before rendering: ActionHandler would `window.open` it itself,
 * which inside the desktop webview does not reliably reach the system browser.
 */
export const CHAT_EXTERNAL_LINK_ACTION = "fl_chat_external_link";
export const MAX_CHAT_MESSAGE_CHARS = 4_000;

const PLACEHOLDER = /\{\{\s*([^{}\s]+)\s*\}\}/g;
const UNSAFE_KEYS: ReadonlySet<string> = new Set([
	"__proto__",
	"constructor",
	"prototype",
]);

/** The normalized URL when `value` is an absolute http(s) URL. */
export function parseHttpUrl(value: unknown): string | undefined {
	if (typeof value !== "string") return undefined;
	try {
		const url = new URL(value.trim());
		return url.protocol === "https:" || url.protocol === "http:"
			? url.href
			: undefined;
	} catch {
		return undefined;
	}
}

/** Chat surfaces load media only from https and inline images. */
export function isAllowedChatAssetSource(value: unknown): value is string {
	if (typeof value !== "string") return false;
	const source = value.trim();
	if (/^data:image\//i.test(source)) return true;
	try {
		return new URL(source).protocol === "https:";
	} catch {
		return false;
	}
}

/** Replace `{{componentId}}` with that component's value; unknown ids stay as written. */
export function interpolateChatMessage(
	template: string,
	lookup: (componentId: string) => string | undefined,
): string {
	const message = template.replace(PLACEHOLDER, (token, id: string) => {
		const value = lookup(id);
		return value === undefined ? token : value;
	});
	return message.slice(0, MAX_CHAT_MESSAGE_CHARS);
}

export function formatChatInputValue(value: unknown): string {
	if (value === null || value === undefined) return "";
	if (typeof value === "string") return value;
	if (typeof value === "number" || typeof value === "boolean") {
		return String(value);
	}
	if (Array.isArray(value)) return value.map(formatChatInputValue).join(", ");
	return JSON.stringify(value);
}

/** The value a `change` event reports: inputs send `value`, toggles send `checked`. */
export function changedInputValue(context: Record<string, unknown>): unknown {
	return Object.hasOwn(context, "value") ? context.value : context.checked;
}

/**
 * The value an untouched input shows: its literal `value`/`checked`, or the data
 * at its bound path (then the binding's default).
 */
export function authoredInputValue(
	component: SurfaceComponent | undefined,
	data: Record<string, unknown>,
): unknown {
	const props = component?.component as unknown as
		| Record<string, unknown>
		| undefined;
	const bound = props?.value ?? props?.checked;
	if (!isJsonObject(bound)) return undefined;
	if (typeof bound.path === "string") {
		return readDataPath(data, bound.path) ?? bound.defaultValue;
	}
	return literalValue(bound);
}

const SCALAR_LITERAL_KEYS = [
	"literalString",
	"literalNumber",
	"literalBool",
] as const;

function literalValue(bound: Record<string, unknown>): unknown {
	const key = SCALAR_LITERAL_KEYS.find((name) => Object.hasOwn(bound, name));
	if (key) return bound[key];
	if (typeof bound.literalJson !== "string") return undefined;
	try {
		return JSON.parse(bound.literalJson);
	} catch {
		return undefined;
	}
}

/** Same path forms as the data model: `$.a.b`, `/a/b`, `a.b`. */
export function readDataPath(
	data: Record<string, unknown>,
	path: string,
): unknown {
	const parts = path
		.trim()
		.replace(/^\$\.?/, "")
		.replace(/^\//, "")
		.split(/[./]/)
		.filter(Boolean);
	let current: unknown = data;
	for (const part of parts) {
		if (UNSAFE_KEYS.has(part)) return undefined;
		if (Array.isArray(current) && /^\d+$/.test(part)) {
			current = current[Number(part)];
		} else if (isJsonObject(current)) {
			current = Object.hasOwn(current, part) ? current[part] : undefined;
		} else {
			return undefined;
		}
	}
	return current;
}

/** Deep-merge `source` into a copy of `target` (objects merge, everything else replaces). */
export function mergeChatData(
	target: Record<string, unknown>,
	source: Record<string, unknown>,
): Record<string, unknown> {
	const merged: Record<string, unknown> = { ...target };
	for (const [key, value] of Object.entries(source)) {
		if (UNSAFE_KEYS.has(key)) continue;
		const previous = merged[key];
		merged[key] =
			isJsonObject(previous) && isJsonObject(value)
				? mergeChatData(previous, value)
				: value;
	}
	return merged;
}

/**
 * Open an http(s) URL outside the app. A click on a real `target="_blank"` anchor is what
 * the desktop shell routes to the system browser (its capture-phase anchor handler); a
 * browser opens a new tab for it.
 */
export function openExternalUrl(
	url: unknown,
	doc: Document = document,
): boolean {
	const href = parseHttpUrl(url);
	if (!href) return false;
	const anchor = doc.createElement("a");
	anchor.href = href;
	anchor.target = "_blank";
	anchor.rel = "noopener noreferrer";
	anchor.hidden = true;
	doc.body.append(anchor);
	try {
		anchor.click();
	} finally {
		anchor.remove();
	}
	return true;
}
