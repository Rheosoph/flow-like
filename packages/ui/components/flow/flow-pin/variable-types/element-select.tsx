import { CheckIcon, ChevronDown, Layers } from "lucide-react";
import { useEffect, useMemo, useState } from "react";
import { useBackend } from "../../../..";
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
import type { SurfaceComponent } from "../../../a2ui/types";

interface ElementSelectProps {
	readonly pin: IPin;
	readonly value: number[] | undefined | null;
	readonly appId: string;
	readonly setValue: (value: unknown) => void;
}

interface ElementOption {
	id: string;
	rawId: string;
	type: string;
	label: string;
	pageName?: string;
	pagePath?: string;
}

function flattenElements(components: SurfaceComponent[]): ElementOption[] {
	const elements: ElementOption[] = [];

	for (const component of components) {
		const componentObj = component.component;
		if (typeof componentObj === "object" && componentObj !== null) {
			const type =
				((componentObj as unknown as Record<string, unknown>).type as string) ||
				"unknown";
			elements.push({
				id: component.id,
				type,
				label: component.id,
				rawId: component.id,
			});
		}
	}

	return elements;
}

export function ElementSelect({
	pin,
	value,
	appId,
	setValue,
}: ElementSelectProps) {
	const backend = useBackend();
	const [elements, setElements] = useState<ElementOption[]>([]);
	const [loading, setLoading] = useState(true);

	useEffect(() => {
		async function loadElements() {
			setLoading(true);
			try {
				const [routes, events, pages] = await Promise.all([
					backend.routeState.getRoutes(appId),
					backend.eventState.getEvents(appId),
					backend.pageState.getPages(appId),
				]);
				const eventsMap = new Map(events.map((e) => [e.id, e]));
				const pagesById = new Map(pages.map((page) => [page.pageId, page]));
				const allElements: ElementOption[] = [];
				const seenIds = new Set<string>();

				const addPageElements = async (
					pageId: string,
					pageName?: string,
					pagePath?: string,
					boardId?: string,
				) => {
					try {
						const page = await backend.pageState.getPage(
							appId,
							pageId,
							boardId,
						);
						if (page?.components) {
							const pageElements = flattenElements(page.components);
							for (const el of pageElements) {
								const optionId = `${pageId}/${el.id}`;
								if (!seenIds.has(optionId)) {
									seenIds.add(optionId);
									allElements.push({
										...el,
										id: optionId,
										rawId: el.id,
										label: pageName ? `${pageName} / ${el.label}` : el.label,
										pageName,
										pagePath,
									});
								}
							}
						}
					} catch {
						// Skip pages that fail to load
					}
				};

				for (const route of routes) {
					const event = eventsMap.get(route.eventId);
					const pageId = event?.default_page_id;
					if (pageId) {
						const pageInfo = pagesById.get(pageId);
						await addPageElements(
							pageId,
							pageInfo?.name,
							route.path,
							pageInfo?.boardId,
						);
					}
				}

				for (const pageInfo of pages) {
					await addPageElements(
						pageInfo.pageId,
						pageInfo.name,
						undefined,
						pageInfo.boardId,
					);
				}

				setElements(allElements);
			} catch (error) {
				console.error("Failed to load page elements:", error);
			} finally {
				setLoading(false);
			}
		}

		loadElements();
	}, [backend, appId]);

	const [open, setOpen] = useState(false);
	const currentValue = parseUint8ArrayToJson(value) as string | undefined;
	const selectedElement = elements.find(
		(el) => el.id === currentValue || el.rawId === currentValue,
	);

	const triggerLabel = useMemo(() => {
		if (loading) return "Loading...";
		return selectedElement?.rawId ?? "Select element";
	}, [loading, selectedElement?.rawId]);

	return (
		<div className="flex flex-row items-center justify-start w-fit max-w-full ml-1 overflow-hidden">
			<Popover open={open} onOpenChange={setOpen}>
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
												<span className="truncate text-xs">{element.rawId}</span>
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
