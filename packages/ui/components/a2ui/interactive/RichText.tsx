"use client";

import {
	type FocusEvent,
	useCallback,
	useEffect,
	useMemo,
	useRef,
} from "react";
import {
	PLATE_JSON_PREFIX,
	parsePlateDocument,
} from "../../../lib/plate-document";
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
import {
	useBoundInputValue,
	valueRevisionOf,
} from "../hooks/use-bound-input-value";
import { useDebouncedTrigger } from "../hooks/use-debounced-trigger";
import type { BoundValue, RichTextComponent } from "../types";

/** `richText` shipped with every event already declared, so none of them inherit `*` or `actions[0]`. */
const EXACT_ONLY = { legacyFallback: false, wildcardFallback: false };

/** Documents are edited in bursts, so allow a longer pause than a single-line input. */
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

function toCssLength(value: string | number | undefined): string | undefined {
	if (value === undefined) return undefined;
	return typeof value === "number" ? `${value}px` : value;
}

/**
 * A new document contains an empty paragraph. Review metadata does not make its body nonempty.
 */
export function isBlankDocument(value: string | undefined): boolean {
	if (!value) return true;
	if (!value.startsWith(PLATE_JSON_PREFIX)) return value.trim().length === 0;
	const document = parsePlateDocument(value);
	return document
		? document.children.every((node) => collectText(node).trim().length === 0)
		: false;
}

function collectText(node: unknown): string {
	if (!node || typeof node !== "object") return "";
	const record = node as Record<string, unknown>;
	// A media node is content even though it carries no text.
	if (typeof record.url === "string" && record.url.length > 0) return "media";
	if (
		[
			"hr",
			"equation",
			"inline_equation",
			"date",
			"mention",
			"user_mention",
			"toc",
			"placeholder",
		].includes(String(record.type))
	)
		return "content";
	let text = typeof record.text === "string" ? record.text : "";
	if (Array.isArray(record.children)) {
		for (const child of record.children) {
			text += collectText(child);
		}
	}
	return text;
}

export function A2UIRichText(props: ComponentProps<RichTextComponent>) {
	const documentId = useResolved<string>(props.component.documentId);
	return (
		<RichTextInput
			key={`${props.surfaceId}/${props.componentId}/${documentId ?? ""}`}
			{...props}
			documentId={documentId}
		/>
	);
}

function RichTextInput({
	elementRef,
	component,
	style,
	componentId,
	surfaceId,
	documentId,
}: ComponentProps<RichTextComponent> & { documentId?: string }) {
	const onAction = useOnAction();
	const triggerEvent = useComponentEventTrigger(componentId);
	const { appId } = useActionContext();
	const resolvedValue = useResolved<string>(component.value);
	const disabled = useResolved<boolean>(component.disabled);
	const readOnly = useResolved<boolean>(component.readOnly);
	const error = useResolved<boolean>(component.error);
	const uploadPrefix = useResolved<string>(component.uploadPrefix);
	const uploadScope = useResolved<string>(component.uploadScope);
	const minHeight = useResolved<string | number>(component.minHeight);
	const maxHeight = useResolved<string | number>(component.maxHeight);
	const documentRevision = useResolved<string | number>(
		component.documentRevision,
	);
	const currentUser = useResolved<{
		id: string;
		name: string;
		avatarUrl?: string;
	}>(component.currentUser);
	const reviewEnabled = useResolved<boolean>(component.reviewEnabled);
	const debounceMs = resolveDebounceMs(
		useResolved<number>(component.debounceMs),
	);

	const label = useResolved<string>(component.label);
	const helperText = useResolved<string>(component.helperText);
	const placeholder = useResolved<string>(component.placeholder);

	const { schedule, cancel } = useDebouncedTrigger(debounceMs);
	const activeRef = useRef(true);
	useEffect(() => {
		activeRef.current = true;
		return () => {
			activeRef.current = false;
		};
	}, []);
	const latestRef = useRef(resolvedValue ?? "");
	const committedRef = useRef(resolvedValue ?? "");
	const rememberExternalValue = useCallback(
		(next: string) => {
			cancel();
			latestRef.current = next;
			committedRef.current = next;
		},
		[cancel],
	);
	const [value, setValue] = useBoundInputValue<string>(component.value, "", {
		revision: JSON.stringify([valueRevisionOf(component), documentRevision]),
		onExternalValue: rememberExternalValue,
	});
	const documentContext = useMemo(
		() => ({
			...(documentId === undefined ? {} : { documentId }),
			...(documentRevision === undefined ? {} : { documentRevision }),
		}),
		[documentId, documentRevision],
	);
	const isEditable = !readOnly && !disabled;

	const uploadConfig = useMemo<EditorUploadConfig>(
		() => ({
			appId,
			prefix: uploadPrefix?.trim() || `a2ui/${surfaceId}/${componentId}`,
			scope: uploadScope === "user" ? "user" : "app",
			onUploaded: (media) => {
				if (!activeRef.current) return;
				void triggerEvent(
					"imageUploaded",
					component,
					{
						...documentContext,
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
				if (!activeRef.current) return;
				void triggerEvent(
					"imageUploadError",
					component,
					{ ...documentContext, name, message },
					EXACT_ONLY,
				);
			},
		}),
		[
			appId,
			component,
			componentId,
			documentContext,
			surfaceId,
			triggerEvent,
			uploadPrefix,
			uploadScope,
		],
	);

	const handleChange = useCallback(
		(content: string) => {
			if (!activeRef.current || !isEditable) return;
			latestRef.current = content;
			setValue(content);

			// Mirrors TextField: the raw action is what `Get Element Value` reads back.
			onAction?.({
				type: "userAction",
				name: "change",
				surfaceId,
				sourceComponentId: componentId,
				timestamp: Date.now(),
				context: { ...documentContext, value: content },
			});

			schedule(() => {
				if (committedRef.current === content) return;
				committedRef.current = content;
				void triggerEvent(
					"change",
					component,
					{ ...documentContext, value: content },
					EXACT_ONLY,
				);
			});
		},
		[
			component,
			componentId,
			documentContext,
			isEditable,
			onAction,
			schedule,
			setValue,
			surfaceId,
			triggerEvent,
		],
	);

	const handleBlur = useCallback(
		(event: FocusEvent<HTMLDivElement>) => {
			if (
				event.relatedTarget &&
				event.currentTarget.contains(event.relatedTarget)
			) {
				return;
			}
			cancel();
			const content = latestRef.current;
			if (committedRef.current !== content) {
				committedRef.current = content;
				void triggerEvent(
					"change",
					component,
					{ ...documentContext, value: content },
					EXACT_ONLY,
				);
			}
			void triggerEvent(
				"blur",
				component,
				{ ...documentContext, value: content },
				EXACT_ONLY,
			);
		},
		[cancel, component, documentContext, triggerEvent],
	);

	const isEmptyDocument = isBlankDocument(value);
	const editorId = `${surfaceId}-${componentId}-editor`;
	const labelId = `${editorId}-label`;
	const helperId = `${editorId}-helper`;
	const editorProps = {
		id: editorId,
		"aria-label": label ? undefined : "Document",
		"aria-labelledby": label ? labelId : undefined,
		"aria-describedby": helperText ? helperId : undefined,
		"aria-invalid": error || undefined,
		"aria-disabled": disabled || undefined,
		"aria-readonly": readOnly || undefined,
	};

	return (
		<div
			ref={elementRef}
			className={cn("flex w-full flex-col gap-1.5", resolveStyle(style))}
			style={resolveInlineStyle(style)}
		>
			{label && (
				<Label id={labelId} htmlFor={editorId}>
					{label}
				</Label>
			)}
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
							initialContent={value}
							documentId={documentId}
							currentUser={currentUser}
							reviewEnabled={reviewEnabled}
							editorProps={editorProps}
							onChange={handleChange}
							editable
							uploadPrefix={uploadConfig.prefix}
							uploadScope={uploadConfig.scope}
						/>
					) : (
						<div className="px-4 py-2">
							<TextEditor
								appId={appId}
								initialContent={value}
								documentId={documentId}
								currentUser={currentUser}
								reviewEnabled={reviewEnabled}
								editorProps={editorProps}
								editable={false}
							/>
						</div>
					)}
				</EditorUploadContext.Provider>
			</div>
			{helperText && (
				<p
					id={helperId}
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
