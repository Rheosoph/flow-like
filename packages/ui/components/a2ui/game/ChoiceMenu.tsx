"use client";

import { cn } from "../../../lib/utils";
import { Button } from "../../ui/button";
import { useComponentEventTrigger } from "../ActionHandler";
import type { ComponentProps } from "../ComponentRegistry";
import { useData } from "../DataContext";
import { resolveInlineStyle, resolveStyle } from "../StyleResolver";
import type {
	BoundValue,
	ChoiceComponent,
	ChoiceMenuComponent,
} from "../types";

function useResolved<T>(boundValue: BoundValue | undefined): T | undefined {
	const { resolve } = useData();
	if (!boundValue) return undefined;
	return resolve(boundValue) as T;
}

export function A2UIChoiceMenu({
	elementRef,
	component,
	style,
	componentId,
}: ComponentProps<ChoiceMenuComponent>) {
	const { resolve } = useData();
	const choicesRaw = useResolved<ChoiceComponent[]>(component.choices);
	const title = useResolved<string>(component.title);
	const layout = useResolved<string>(component.layout) ?? "vertical";
	const triggerEvent = useComponentEventTrigger(componentId);

	const choices = choicesRaw ?? [];

	const handleChoiceClick = (choice: ChoiceComponent, index: number) => {
		void triggerEvent("choiceSelect", component, {
			choiceId: choice.id,
			choiceIndex: index,
		});
	};

	const layoutClasses: Record<string, string> = {
		vertical: "flex flex-col gap-2",
		horizontal: "flex flex-row gap-2 flex-wrap",
		grid: "grid grid-cols-2 gap-2",
	};

	const layoutClass = layoutClasses[layout] ?? layoutClasses.vertical;

	return (
		<div
			ref={elementRef}
			className={cn("p-4", resolveStyle(style))}
			style={resolveInlineStyle(style)}
		>
			{title && <h3 className="text-lg font-semibold mb-4">{title}</h3>}
			<div className={layoutClass}>
				{choices.map((choice, index) => {
					const choiceText = choice.text
						? (resolve(choice.text) as string)
						: "";
					const disabled = choice.disabled
						? (resolve(choice.disabled) as boolean)
						: false;
					return (
						<Button
							key={choice.id}
							variant="outline"
							disabled={disabled}
							onClick={() => handleChoiceClick(choice, index)}
							className="justify-start text-left"
						>
							{choiceText}
						</Button>
					);
				})}
			</div>
		</div>
	);
}
