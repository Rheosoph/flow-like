"use client";

import { cn } from "../../../lib/utils";
import { TextEditor } from "../../ui/text-editor";
import type { ComponentProps } from "../ComponentRegistry";
import { useData } from "../DataContext";
import { resolveInlineStyle, resolveStyle } from "../StyleResolver";
import type { BoundValue, MarkdownComponent } from "../types";

function useResolved<T>(boundValue: BoundValue | undefined): T | undefined {
	const { resolve } = useData();
	if (!boundValue) return undefined;
	return resolve(boundValue) as T;
}

export function A2UIMarkdown({
	elementRef,
	component,
	style,
}: ComponentProps<MarkdownComponent>) {
	const content = useResolved<string>(component.content);

	if (!content) return null;

	return (
		<div
			ref={elementRef}
			className={cn(
				"prose prose-sm dark:prose-invert max-w-none",
				resolveStyle(style),
			)}
			style={resolveInlineStyle(style)}
		>
			<TextEditor initialContent={content} isMarkdown editable={false} />
		</div>
	);
}
