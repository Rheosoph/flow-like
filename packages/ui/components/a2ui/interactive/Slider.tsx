"use client";

import { useTranslation } from "@flow-like/locales";
import { cn } from "../../../lib/utils";
import { Label } from "../../ui/label";
import { Slider } from "../../ui/slider";
import { useComponentEventTrigger, useOnAction } from "../ActionHandler";
import type { ComponentProps } from "../ComponentRegistry";
import { useData } from "../DataContext";
import { resolveInlineStyle, resolveStyle } from "../StyleResolver";
import {
	useBoundInputValue,
	valueRevisionOf,
} from "../hooks/use-bound-input-value";
import {
	resolveEventDebounceMs,
	useDebouncedTrigger,
} from "../hooks/use-debounced-trigger";
import type { BoundValue, SliderComponent } from "../types";

/** Events added after slider shipped never inherit `*` or `actions[0]`. */
const EXACT_ONLY = { legacyFallback: false, wildcardFallback: false };

function useResolved<T>(boundValue: BoundValue | undefined): T | undefined {
	const { resolve } = useData();
	if (!boundValue) return undefined;
	return resolve(boundValue) as T;
}

export function A2UISlider({
	elementRef,
	component,
	style,
	componentId,
	surfaceId,
}: ComponentProps<SliderComponent>) {
	const { t } = useTranslation("common");
	const onAction = useOnAction();
	const triggerEvent = useComponentEventTrigger(componentId);
	const disabled = useResolved<boolean>(component.disabled);
	const min = useResolved<number>(component.min) ?? 0;
	const max = useResolved<number>(component.max) ?? 100;
	const step = useResolved<number>(component.step) ?? 1;
	const showValue = useResolved<boolean>(component.showValue);
	const debounceMs = resolveEventDebounceMs(
		useResolved<number>(component.debounceMs),
	);
	const [value, setValue] = useBoundInputValue<number>(component.value, min, {
		revision: valueRevisionOf(component),
	});
	const { schedule, cancel } = useDebouncedTrigger(debounceMs);

	const handleChange = (newValues: number[]) => {
		const newValue = newValues[0];
		setValue(newValue);
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
	const handleCommit = (newValues: number[]) => {
		cancel();
		void triggerEvent("change", component, { value: newValues[0] });
	};

	return (
		<div
			ref={elementRef}
			className={cn("space-y-3", resolveStyle(style))}
			style={resolveInlineStyle(style)}
		>
			{showValue && (
				<div className="flex justify-between items-center">
					<Label>{t("value2", "Value")}</Label>
					<span className="text-sm text-muted-foreground">{value}</span>
				</div>
			)}
			<Slider
				value={[value]}
				min={min}
				max={max}
				step={step}
				disabled={disabled}
				onValueChange={handleChange}
				onValueCommit={handleCommit}
			/>
		</div>
	);
}
