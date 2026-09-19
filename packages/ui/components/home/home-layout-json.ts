import { appBuildFingerprint } from "../../lib/app-build/fingerprint";
import {
	MAX_HOME_LAYOUT_BYTES,
	MAX_HOME_WIDGETS,
	MAX_HOME_WIDGET_CLASS_NAME_BYTES,
	homeLayoutByteLength,
	normalizeHomeLayout,
} from "./home-layout";
import {
	type FormattedHomeLayoutJsonResult,
	formatJsonDocument,
	parseJsonDocument,
} from "./home-layout-json-document";
import type { IHomeLayout } from "./types";

export type { FormattedHomeLayoutJsonResult } from "./home-layout-json-document";

export type HomeLayoutJsonResult =
	| { ok: true; layout: IHomeLayout }
	| { ok: false; error: string };

const textEncoder = new TextEncoder();

export function serializeHomeLayout(layout: IHomeLayout) {
	const result = trySerializeHomeLayout(layout);
	if (!result.ok) throw new Error(result.error);
	return result.json;
}

export function trySerializeHomeLayout(
	layout: IHomeLayout,
): FormattedHomeLayoutJsonResult {
	return formatJsonDocument(layout);
}

export function formatHomeLayoutJson(
	source: string,
): FormattedHomeLayoutJsonResult {
	const document = parseJsonDocument(source);
	return document.ok ? formatJsonDocument(document.value) : document;
}

function textLimitError(
	value: string | undefined,
	label: string,
	maximum: number,
	required = false,
) {
	if (value === undefined) return required ? `${label} is required.` : null;
	const bytes = textEncoder.encode(value).byteLength;
	if (required && bytes === 0)
		return `${label} must contain 1 to ${maximum} bytes.`;
	return bytes > maximum ? `${label} must not exceed ${maximum} bytes.` : null;
}

function persistenceError(layout: IHomeLayout) {
	const layoutError =
		textLimitError(layout.title, "Layout title", 256) ??
		textLimitError(layout.description, "Layout description", 2000);
	if (layoutError) return layoutError;

	for (const [index, widget] of layout.widgets.entries()) {
		const prefix = `Widget ${index + 1}`;
		const widgetError =
			textLimitError(widget.id, `${prefix} id`, 128, true) ??
			textLimitError(widget.type, `${prefix} type`, 80, true) ??
			textLimitError(widget.title, `${prefix} title`, 256) ??
			textLimitError(widget.description, `${prefix} description`, 2000) ??
			textLimitError(
				widget.appearance.variant,
				`${prefix} appearance variant`,
				80,
				true,
			) ??
			textLimitError(
				widget.appearance.accent,
				`${prefix} appearance accent`,
				128,
				true,
			) ??
			textLimitError(
				widget.appearance.className,
				`${prefix} appearance className`,
				MAX_HOME_WIDGET_CLASS_NAME_BYTES,
			);
		if (widgetError) return widgetError;
	}

	return null;
}

function jsonValuesEqual(left: unknown, right: unknown) {
	const pending: Array<[unknown, unknown]> = [[left, right]];
	while (pending.length) {
		const pair = pending.pop();
		if (!pair) break;
		const [currentLeft, currentRight] = pair;
		if (Object.is(currentLeft, currentRight)) continue;
		if (
			!currentLeft ||
			!currentRight ||
			typeof currentLeft !== "object" ||
			typeof currentRight !== "object"
		)
			return false;
		if (Array.isArray(currentLeft) || Array.isArray(currentRight)) {
			if (
				!Array.isArray(currentLeft) ||
				!Array.isArray(currentRight) ||
				currentLeft.length !== currentRight.length
			)
				return false;
			for (let index = 0; index < currentLeft.length; index++)
				pending.push([currentLeft[index], currentRight[index]]);
			continue;
		}
		const leftRecord = currentLeft as Record<string, unknown>;
		const rightRecord = currentRight as Record<string, unknown>;
		const leftKeys = Object.keys(leftRecord);
		const rightKeys = Object.keys(rightRecord);
		if (
			leftKeys.length !== rightKeys.length ||
			leftKeys.some((key) => !Object.hasOwn(rightRecord, key))
		)
			return false;
		for (const key of leftKeys)
			pending.push([leftRecord[key], rightRecord[key]]);
	}
	return true;
}

export function homeLayoutsEqual(left: IHomeLayout, right: IHomeLayout) {
	try {
		return jsonValuesEqual(
			JSON.parse(JSON.stringify(left)),
			JSON.parse(JSON.stringify(right)),
		);
	} catch {
		return false;
	}
}

/**
 * Stable change token for a canonical Home layout. The token detects stale editor drafts; it is
 * not an authorization credential.
 */
export function homeLayoutFingerprint(layout: IHomeLayout) {
	return appBuildFingerprint("home-layout-v1", layout);
}

export function parseHomeLayoutJson(source: string): HomeLayoutJsonResult {
	const document = parseJsonDocument(source);
	if (!document.ok) return document;

	try {
		const layout = normalizeHomeLayout(document.value);
		if (!layout) {
			return {
				ok: false,
				error: `Expected a version 1 home layout with a widgets array and no more than ${MAX_HOME_WIDGETS} widgets. Each widget needs a non-empty, unique id and type.`,
			};
		}

		if (homeLayoutByteLength(layout) > MAX_HOME_LAYOUT_BYTES) {
			return {
				ok: false,
				error: "This layout exceeds the 128 KiB save limit.",
			};
		}

		const validationError = persistenceError(layout);
		if (validationError) return { ok: false, error: validationError };

		return { ok: true, layout };
	} catch {
		return { ok: false, error: "Could not validate this Home layout JSON." };
	}
}
