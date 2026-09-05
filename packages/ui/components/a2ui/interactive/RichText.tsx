"use client";

import { useCallback, useEffect, useMemo, useRef, useState } from "react";
import { cn } from "../../../lib/utils";
import {
	type EditorUploadConfig,
	EditorUploadContext,
} from "../../editor/upload-context";
import { Label } from "../../ui/label";
import { TextEditor } from "../../ui/text-editor";
import {
	useActionContext,
	useComponentEventTrigger,
	useOnAction,
} from "../ActionHandler";
import type { ComponentProps } from "../ComponentRegistry";
import { useData } from "../DataContext";
import { resolveInlineStyle, resolveStyle } from "../StyleResolver";
import { useDebouncedTrigger } from "../hooks/use-debounced-trigger";
import type { BoundValue, RichTextComponent } from "../types";

/** `richText` shipped with every event already declared, so none of them inherit `*` or `actions[0]`. */
const EXACT_ONLY = { legacyFallback: false, wildcardFallback: false };

/** Authoring pauses are longer than typing pauses — a document is edited in bursts, not keystrokes. */
const DEFAULT_RICH_TEXT_DEBOUNCE_MS = 600;
const MIN_RICH_TEXT_DEBOUNCE_MS = 100;

function resolveDebounceMs(value: number | undefined): number {
	if (typeof value !== "number" || !Number.isFinite(value) || value <= 0) {
		return DEFAULT_RICH_TEXT_DEBOUNCE_MS;
	}
	return Math.max(MIN_RICH_TEXT_DEBOUNCE_MS, value);
}

function useResolved<T>(boundValue: BoundValue | undefined): T | undefined {
	const { resolve } = useData();
	if (!boundValue) return undefined;
	return resolve(boundValue) as T;
}

function resolveStringOrBound(
	value: string | BoundValue | undefined,
): string | undefined {
	if (!value) return undefined;
	if (typeof value === "string") return value;
	if ("literalString" in value) return value.literalString;
	if ("literalNumber" in value) return String(value.literalNumber);
	return undefined;
}

function toCssLength(value: string | number | undefined): string | undefined {
	if (value === undefined) return undefined;
	return typeof value === "number" ? `${value}px` : value;
}

const PLATE_JSON_PREFIX = "plate_json::";

/**
 * A freshly created document is not the empty string — it is one empty paragraph — so the
 * placeholder has to look past the envelope.
 */
export function isBlankDocument(value: string | undefined): boolean {
	if (!value) return true;
	if (!value.startsWith(PLATE_JSON_PREFIX)) return value.trim().length === 0;
	try {
		const nodes = JSON.parse(value.slice(PLATE_JSON_PREFIX.length));
		if (!Array.isArray(nodes)) return true;
		return nodes.every((node) => collectText(node).trim().length === 0);
	} catch {
		return false;
	}
}

function collectText(node: unknown): string {
	if (!node || typeof node !== "object") return "";
	const record = node as Record<string, unknown>;
	// A media node is content even though it carries no text.
	if (typeof record.url === "string" && record.url.length > 0) return "media";
	let text = typeof record.text === "string" ? record.text : "";
	if (Array.isArray(record.children)) {
		for (const child of record.children) {
			text += collectText(child);
		}
	}
	return text;
}

export function A2UIRichText({
	elementRef,
	component,
	style,
	componentId,
	surfaceId,
}: ComponentProps<RichTextComponent>) {
	const onAction = useOnAction();
	const triggerEvent = useComponentEventTrigger(componentId);
	const { appId } = useActionContext();
	const { setByPath } = useData();

	const resolvedValue = useResolved<string>(component.value);
	const disabled = useResolved<boolean>(component.disabled);
	const readOnly = useResolved<boolean>(component.readOnly);
	const error = useResolved<boolean>(component.error);
	const uploadPrefix = useResolved<string>(component.uploadPrefix);
	const uploadScope = useResolved<string>(component.uploadScope);
	const minHeight = useResolved<string | number>(component.minHeight);
	const maxHeight = useResolved<string | number>(component.maxHeight);
	const debounceMs = resolveDebounceMs(
		useResolved<number>(component.debounceMs),
	);

	const label = resolveStringOrBound(
		component.label as string | BoundValue | undefined,
	);
	const helperText = resolveStringOrBound(
		component.helperText as string | BoundValue | undefined,
	);
	const placeholder = resolveStringOrBound(
		component.placeholder as string | BoundValue | undefined,
	);

	const { schedule, cancel } = useDebouncedTrigger(debounceMs);
	const isPathBound = Boolean(component.value && "path" in component.value);

	// The editor is uncontrolled once mounted: remounting it on every keystroke would
	// destroy the selection. `seed` only advances when the value changes from outside.
	const [seed, setSeed] = useState(resolvedValue ?? "");
	const latestRef = useRef(resolvedValue ?? "");
	const committedRef = useRef(resolvedValue ?? "");

	useEffect(() => {
		const next = resolvedValue ?? "";
		if (next === latestRef.current) return;
		latestRef.current = next;
		committedRef.current = next;
		setSeed(next);
	}, [resolvedValue]);

	const uploadConfig = useMemo<EditorUploadConfig>(
		() => ({
			appId,
			prefix: uploadPrefix?.trim() || `a2ui/${surfaceId}/${componentId}`,
			scope: uploadScope === "user" ? "user" : "app",
			onUploaded: (media) => {
				void triggerEvent(
					"imageUploaded",
					component,
					{
						path: media.path,
						url: media.url,
						name: media.name,
						size: media.size,
						type: media.type,
					},
					EXACT_ONLY,
				);
			},
			onUploadError: (name, message) => {
				void triggerEvent(
					"imageUploadError",
					component,
					{ name, message },
					EXACT_ONLY,
				);
			},
		}),
		[
			appId,
			component,
			componentId,
			surfaceId,
			triggerEvent,
			uploadPrefix,
			uploadScope,
		],
	);

	const handleChange = useCallback(
		(content: string) => {
			latestRef.current = content;

			if (isPathBound && component.value && "path" in component.value) {
				setByPath(component.value.path, content);
			}

			// Mirrors TextField: the raw action is what `Get Element Value` reads back.
			onAction?.({
				type: "userAction",
				name: "change",
				surfaceId,
				sourceComponentId: componentId,
				timestamp: Date.now(),
				context: { value: content },
			});

			schedule(() => {
				if (committedRef.current === content) return;
				committedRef.current = content;
				void triggerEvent("change", component, { value: content }, EXACT_ONLY);
			});
		},
		[
			component,
			componentId,
			isPathBound,
			onAction,
			schedule,
			setByPath,
			surfaceId,
			triggerEvent,
		],
	);

	const handleBlur = useCallback(() => {
		cancel();
		const content = latestRef.current;
		if (committedRef.current !== content) {
			committedRef.current = content;
			void triggerEvent("change", component, { value: content }, EXACT_ONLY);
		}
		void triggerEvent("blur", component, { value: content }, EXACT_ONLY);
	}, [cancel, component, triggerEvent]);

	const isEditable = !readOnly && !disabled;
	const isEmptyDocument = isBlankDocument(seed);

	return (
		<div
			ref={elementRef}
			className={cn("flex w-full flex-col gap-1.5", resolveStyle(style))}
			style={resolveInlineStyle(style)}
		>
			{label && <Label htmlFor={componentId}>{label}</Label>}
			<div
				id={componentId}
				onBlur={isEditable ? handleBlur : undefined}
				className={cn(
					"relative w-full overflow-y-auto rounded-md border bg-background",
					error ? "border-destructive" : "border-input",
					disabled && "pointer-events-none opacity-60",
				)}
				style={{
					minHeight: toCssLength(minHeight) ?? "12rem",
					maxHeight: toCssLength(maxHeight),
				}}
			>
				{placeholder && isEmptyDocument && (
					<p className="pointer-events-none absolute top-2 left-4 z-10 text-sm text-muted-foreground">
						{placeholder}
					</p>
				)}
				<EditorUploadContext.Provider value={uploadConfig}>
					{isEditable ? (
						<TextEditor
							appId={appId}
							initialContent={seed}
							onChange={handleChange}
							editable
							uploadPrefix={uploadConfig.prefix}
							uploadScope={uploadConfig.scope}
						/>
					) : (
						<div className="px-4 py-2">
							<TextEditor
								appId={appId}
								initialContent={seed}
								editable={false}
							/>
						</div>
					)}
				</EditorUploadContext.Provider>
			</div>
			{helperText && (
				<p
					className={cn(
						"text-xs",
						error ? "text-destructive" : "text-muted-foreground",
					)}
				>
					{helperText}
				</p>
			)}
		</div>
	);
}
