"use client";

import { useEffect, useRef, useState } from "react";
import { cn } from "../../../lib/utils";
import { Input } from "../../ui/input";
import { Label } from "../../ui/label";
import { Textarea } from "../../ui/textarea";
import { useComponentEventTrigger, useOnAction } from "../ActionHandler";
import type { ComponentProps } from "../ComponentRegistry";
import { useData } from "../DataContext";
import { resolveInlineStyle, resolveStyle } from "../StyleResolver";
import {
	resolveEventDebounceMs,
	useDebouncedTrigger,
} from "../hooks/use-debounced-trigger";
import type { BoundValue, TextFieldComponent } from "../types";
import {
	type EnterKeyState,
	resolveEnterIntent,
	shouldReportCommit,
} from "./text-field-events";

/** Events added after textField shipped never inherit `*` or `actions[0]`. */
const EXACT_ONLY = { legacyFallback: false, wildcardFallback: false };

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

export function A2UITextField({
	component,
	style,
	componentId,
	surfaceId,
}: ComponentProps<TextFieldComponent>) {
	// Use onAction from context to ensure values are stored
	const onAction = useOnAction();
	const triggerEvent = useComponentEventTrigger(componentId);
	const resolvedValue = useResolved<string>(component.value);
	const placeholder = useResolved<string>(component.placeholder);
	const disabled = useResolved<boolean>(component.disabled);
	const error = useResolved<boolean>(component.error);
	const label = resolveStringOrBound(
		component.label as string | BoundValue | undefined,
	);
	const helperText = resolveStringOrBound(
		component.helperText as string | BoundValue | undefined,
	);
	const inputType = useResolved<string>(component.inputType) ?? "text";
	const multiline = useResolved<boolean>(component.multiline);
	const rows = useResolved<number>(component.rows);
	const maxLength = useResolved<number>(component.maxLength);
	const debounceMs = resolveEventDebounceMs(
		useResolved<number>(component.debounceMs),
	);
	const { setByPath } = useData();
	const { schedule, cancel } = useDebouncedTrigger(debounceMs);

	// Check if value is bound to a path (controlled by data context) or literal (local state)
	const isPathBound = component.value && "path" in component.value;

	// Local state for user input - only used when typing, not for display
	const [localValue, setLocalValue] = useState(resolvedValue ?? "");

	// Track if user is actively typing (to avoid overwriting their input)
	const [isUserTyping, setIsUserTyping] = useState(false);

	// The value the `change` event last reported. Tabbing through an untouched
	// field must not run the configured workflow again.
	const committedValueRef = useRef(resolvedValue ?? "");

	// Sync local state when resolved value changes from external updates (setValue/clear)
	// Only sync when user is NOT actively typing
	useEffect(() => {
		if (!isUserTyping) {
			setLocalValue(resolvedValue ?? "");
			committedValueRef.current = resolvedValue ?? "";
		}
	}, [resolvedValue, componentId, isUserTyping]);

	// Reset typing state when component value changes externally
	useEffect(() => {
		setIsUserTyping(false);
	}, [component.value]);

	// For literal values, prefer resolvedValue over localValue (external updates take precedence)
	// For path bindings, use resolvedValue from data context
	// Only use localValue when user is actively typing
	const displayValue = isUserTyping ? localValue : (resolvedValue ?? "");

	const handleChange = (newValue: string) => {
		// Mark that user is actively typing
		setIsUserTyping(true);
		// Update local state for immediate feedback
		setLocalValue(newValue);

		// If bound to a path, also update the data context
		if (isPathBound && component.value && "path" in component.value) {
			setByPath(component.value.path, newValue);
		}

		if (onAction) {
			onAction({
				type: "userAction",
				name: "change",
				surfaceId,
				sourceComponentId: componentId,
				timestamp: Date.now(),
				context: { value: newValue },
			});
		}

		schedule(() => {
			void triggerEvent("input", component, { value: newValue }, EXACT_ONLY);
		});
	};

	const commit = (value: string) => {
		cancel();
		setIsUserTyping(false);
		if (!shouldReportCommit(committedValueRef.current, value)) return;
		committedValueRef.current = value;
		void triggerEvent("change", component, { value });
	};

	// Submitting keeps focus: a composer has to stay usable for the next message.
	const handleKeyDown = (
		event: EnterKeyState & { preventDefault: () => void },
	) => {
		const intent = resolveEnterIntent(event, multiline);
		if (intent.kind !== "submit") return;

		event.preventDefault();
		commit(displayValue);
		void triggerEvent(
			"submit",
			component,
			{ value: displayValue, via: intent.via },
			EXACT_ONLY,
		);
	};

	const InputComponent = multiline ? Textarea : Input;

	return (
		<div
			className={cn("space-y-1.5", resolveStyle(style))}
			style={resolveInlineStyle(style)}
		>
			{label && (
				<Label
					className={cn(
						inputType === "password" &&
							"after:content-['*'] after:ml-0.5 after:text-destructive",
					)}
				>
					{label}
				</Label>
			)}
			<InputComponent
				type={inputType}
				value={displayValue}
				placeholder={placeholder}
				disabled={disabled}
				maxLength={maxLength}
				{...(multiline && rows ? { rows } : {})}
				className={cn(error && "border-destructive")}
				onChange={(e) => handleChange(e.target.value)}
				onFocus={() => {
					void triggerEvent(
						"focus",
						component,
						{ value: displayValue },
						EXACT_ONLY,
					);
				}}
				onBlur={() => {
					commit(displayValue);
					void triggerEvent(
						"blur",
						component,
						{ value: displayValue },
						EXACT_ONLY,
					);
				}}
				onKeyDown={handleKeyDown}
			/>
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
