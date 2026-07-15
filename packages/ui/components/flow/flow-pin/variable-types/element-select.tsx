import { CheckIcon, ChevronDown, Layers } from "lucide-react";
import { useCallback, useMemo, useState } from "react";
import {
	Command,
	CommandEmpty,
	CommandGroup,
	CommandInput,
	CommandItem,
	CommandList,
} from "../../../../components/ui/command";
import {
	Popover,
	PopoverContent,
	PopoverTrigger,
} from "../../../../components/ui/popover";
import type { IPin } from "../../../../lib/schema/flow/pin";
import {
	convertJsonToUint8Array,
	parseUint8ArrayToJson,
} from "../../../../lib/uint8";
import type { FlowSelectorDataRef } from "../../flow-selector-data";

interface ElementSelectProps {
	readonly pin: IPin;
	readonly value: number[] | undefined | null;
	readonly setValue: (value: unknown) => void;
	readonly selectorDataRef?: FlowSelectorDataRef;
}

function getRawElementId(value?: string): string | undefined {
	if (!value) return undefined;
	const slashIndex = value.lastIndexOf("/");
	return slashIndex >= 0 ? value.slice(slashIndex + 1) : value;
}

export function ElementSelect({
	pin,
	value,
	setValue,
	selectorDataRef,
}: ElementSelectProps) {
	const [open, setOpen] = useState(false);
	const [, refreshSnapshot] = useState(0);

	const handleOpenChange = useCallback(
		(isOpen: boolean) => {
			setOpen(isOpen);
			if (!isOpen) return;

			refreshSnapshot((version) => version + 1);
			const loadPromise = selectorDataRef?.current.loadElements(true);
			loadPromise?.finally(() => refreshSnapshot((version) => version + 1));
		},
		[selectorDataRef],
	);

	const currentValue = parseUint8ArrayToJson(value) as string | undefined;
	const elements = selectorDataRef?.current.elementOptions ?? [];
	const selectedElement = elements.find(
		(el) => el.id === currentValue || el.rawId === currentValue,
	);
	const loading = open && (selectorDataRef?.current.elementsLoading ?? false);

	const triggerLabel = useMemo(() => {
		if (loading && !currentValue) return "Loading...";
		return (
			selectedElement?.rawId ??
			getRawElementId(currentValue) ??
			"Select element"
		);
	}, [currentValue, loading, selectedElement?.rawId]);

	return (
		<div className="flex flex-row items-center justify-start w-fit max-w-full ml-1 overflow-hidden">
			<Popover open={open} onOpenChange={handleOpenChange}>
				<PopoverTrigger asChild>
					<button
						type="button"
						className="flex flex-row items-center gap-0.5 w-fit max-w-full p-0 border-0 text-xs bg-card text-start h-4 overflow-hidden cursor-pointer"
					>
						<Layers className="size-2 min-w-2 min-h-2 text-muted-foreground mr-0.5 shrink-0" />
						<small className="text-start text-[10px] m-0! truncate">
							{triggerLabel}
						</small>
						<ChevronDown className="size-2 min-w-2 min-h-2 text-card-foreground shrink-0" />
					</button>
				</PopoverTrigger>
				<PopoverContent className="w-60 p-0" align="start">
					<Command>
						<CommandInput placeholder="Search elements..." />
						<CommandList>
							<CommandEmpty>No elements found.</CommandEmpty>
							<CommandGroup heading={pin.friendly_name}>
								{elements.map((element) => (
									<CommandItem
										key={element.id}
										value={`${element.pageName ?? ""} ${element.rawId} ${element.type}`}
										onSelect={() => {
											setValue(convertJsonToUint8Array(element.id));
											setOpen(false);
										}}
									>
										<div className="flex flex-col gap-0.5 min-w-0">
											<div className="flex items-center gap-1">
												<span className="truncate text-xs">
													{element.rawId}
												</span>
												<span className="text-[10px] text-muted-foreground shrink-0">
													{element.type}
												</span>
											</div>
											{element.pageName && (
												<span className="text-[10px] text-muted-foreground truncate">
													{element.pageName}
												</span>
											)}
										</div>
										{currentValue === element.id && (
											<CheckIcon className="ml-auto size-3 shrink-0" />
										)}
									</CommandItem>
								))}
							</CommandGroup>
						</CommandList>
					</Command>
				</PopoverContent>
			</Popover>
		</div>
	);
}
