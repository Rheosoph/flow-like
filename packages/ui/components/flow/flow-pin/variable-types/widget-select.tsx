import { ChevronDown } from "lucide-react";
import { useMemo } from "react";
import {
	Select,
	SelectContent,
	SelectGroup,
	SelectItem,
	SelectLabel,
	SelectTrigger,
} from "../../../../components/ui/select";
import { useInvoke } from "../../../../hooks";
import type { IPin } from "../../../../lib/schema/flow/pin";
import {
	convertJsonToUint8Array,
	parseUint8ArrayToJson,
} from "../../../../lib/uint8";
import { useBackend } from "../../../../state/backend-state";

export function WidgetVariable({
	pin,
	value,
	appId,
	setValue,
}: Readonly<{
	pin: IPin;
	value: number[] | undefined | null;
	appId: string;
	setValue: (value: unknown) => void;
}>) {
	const backend = useBackend();
	const { data: widgets, isLoading } = useInvoke(
		backend.widgetState.getWidgets,
		backend.widgetState,
		[appId],
		appId !== "",
	);

	const selectedValue = parseUint8ArrayToJson(value);
	const selectedWidgetId =
		typeof selectedValue === "string" ? selectedValue : undefined;

	const widgetOptions = useMemo(
		() =>
			(widgets ?? []).map(([, widgetId, metadata]) => {
				const label =
					typeof metadata?.name === "string" && metadata.name.trim()
						? metadata.name.trim()
						: widgetId;
				return { widgetId, label };
			}),
		[widgets],
	);

	const selectedWidget = widgetOptions.find(
		(widget) =>
			widget.widgetId === selectedWidgetId || widget.label === selectedWidgetId,
	);
	const triggerLabel =
		selectedWidget?.label ??
		selectedWidgetId ??
		(isLoading ? "Loading" : "Select widget");

	return (
		<div
			className="flex flex-row items-center justify-start max-w-full ml-1 overflow-hidden"
			onMouseDown={(e) => e.stopPropagation()}
			onPointerDown={(e) => e.stopPropagation()}
		>
			<Select
				value={selectedWidget?.widgetId ?? selectedWidgetId}
				onValueChange={(widgetId) =>
					setValue(convertJsonToUint8Array(widgetId))
				}
			>
				<SelectTrigger
					noChevron
					size="sm"
					className="w-fit! max-w-full! p-0 border-0 text-xs bg-card! text-start max-h-fit h-4 gap-0.5 flex-row items-center overflow-hidden"
				>
					<small className="text-start text-[10px] m-0! truncate">
						{triggerLabel}
					</small>
					<ChevronDown className="size-2 min-w-2 min-h-2 text-card-foreground mt-0.5 shrink-0" />
				</SelectTrigger>
				<SelectContent>
					<SelectGroup>
						<SelectLabel>{pin.friendly_name}</SelectLabel>
						{widgetOptions.map((widget) => (
							<SelectItem key={widget.widgetId} value={widget.widgetId}>
								{widget.label}
							</SelectItem>
						))}
					</SelectGroup>
				</SelectContent>
			</Select>
		</div>
	);
}
