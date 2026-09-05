"use client";

import { useEffect, useState } from "react";
import { cn } from "../../../lib/utils";
import type { ComponentProps, RenderChildFn } from "../ComponentRegistry";
import { useData } from "../DataContext";
import { resolveInlineStyle, resolveStyle } from "../StyleResolver";
import type { BoundValue, DialogueComponent } from "../types";

function useResolved<T>(boundValue: BoundValue | undefined): T | undefined {
	const { resolve } = useData();
	if (!boundValue) return undefined;
	return resolve(boundValue) as T;
}

export function A2UIDialogue({
	elementRef,
	component,
	style,
	renderChild,
}: ComponentProps<DialogueComponent> & { renderChild: RenderChildFn }) {
	const text = useResolved<string>(component.text) ?? "";
	const speakerName = useResolved<string>(component.speakerName);
	const speakerPortraitId = useResolved<string>(component.speakerPortraitId);
	const typewriter = useResolved<boolean>(component.typewriter) ?? false;
	const speed = useResolved<number>(component.typewriterSpeed) ?? 30;
	const [displayedText, setDisplayedText] = useState("");

	useEffect(() => {
		if (!typewriter) {
			setDisplayedText(text);
			return;
		}

		setDisplayedText("");
		let index = 0;
		const interval = setInterval(() => {
			if (index < text.length) {
				setDisplayedText(text.slice(0, index + 1));
				index++;
			} else {
				clearInterval(interval);
			}
		}, 1000 / speed);

		return () => clearInterval(interval);
	}, [text, typewriter, speed]);

	return (
		<div
			ref={elementRef}
			className={cn(
				"relative p-4 bg-background/95 border rounded-lg shadow-lg",
				resolveStyle(style),
			)}
			style={resolveInlineStyle(style)}
		>
			<div className="flex gap-4">
				{speakerPortraitId && (
					<div className="flex-shrink-0">{renderChild(speakerPortraitId)}</div>
				)}
				<div className="flex-1 min-w-0">
					{speakerName && (
						<div className="font-bold text-primary mb-1">{speakerName}</div>
					)}
					<p className="text-foreground whitespace-pre-wrap">{displayedText}</p>
				</div>
			</div>
		</div>
	);
}
