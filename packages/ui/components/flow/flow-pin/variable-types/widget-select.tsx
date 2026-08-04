import { useQuery } from "@tanstack/react-query";
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
import {
	encodePackageWidgetRef,
	listAppPackageWidgets,
} from "../../../../lib/package-widgets";
import type { IPin } from "../../../../lib/schema/flow/pin";
import {
	convertJsonToUint8Array,
	parseUint8ArrayToJson,
} from "../../../../lib/uint8";
import { useBackend } from "../../../../state/backend-state";

interface WidgetOption {
	readonly selector: string;
	/** Trigger label — kept short, the node header shares its row with a pin. */
	readonly label: string;
	/** Disambiguates same-named widgets from different packages in the list. */
	readonly packageId?: string;
}

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
	const enabled = appId !== "";
	const { data: widgets, isLoading } = useInvoke(
		backend.widgetState.getWidgets,
		backend.widgetState,
		[appId],
		enabled,
	);

	// Widgets of the packages added to the app (§6.1) — same list the builder
	// palette shows; empty on hosts without per-app package listing.
	const { data: packageWidgets } = useQuery({
		queryKey: ["app-package-widgets", appId],
		queryFn: () =>
			listAppPackageWidgets(
				{
					listPackages: backend.appState.listPackages?.bind(backend.appState),
					getPackage: (packageId) =>
						backend.registryState.getPackage(packageId),
				},
				appId,
			),
		enabled,
	});

	const selectedValue = parseUint8ArrayToJson(value);
	const selectedSelector =
		typeof selectedValue === "string" ? selectedValue : undefined;

	const projectOptions = useMemo<WidgetOption[]>(
		() =>
			(widgets ?? []).map(([, widgetId, metadata]) => ({
				selector: widgetId,
				label:
					typeof metadata?.name === "string" && metadata.name.trim()
						? metadata.name.trim()
						: widgetId,
			})),
		[widgets],
	);

	const packageOptions = useMemo<WidgetOption[]>(
		() =>
			(packageWidgets ?? []).map((entry) => ({
				selector: encodePackageWidgetRef(entry.packageId, entry.widget.id),
				label: entry.widget.name,
				packageId: entry.packageId,
			})),
		[packageWidgets],
	);

	// Project widgets are stored by id, package widgets by `pkg:` ref — boards
	// written before either encoding stored the plain widget name.
	const selectedOption = useMemo(
		() =>
			[...projectOptions, ...packageOptions].find(
				(option) =>
					option.selector === selectedSelector ||
					option.label === selectedSelector,
			),
		[projectOptions, packageOptions, selectedSelector],
	);

	const triggerLabel =
		selectedOption?.label ??
		selectedSelector ??
		(isLoading ? "Loading" : "Select widget");

	return (
		<div
			className="flex flex-row items-center justify-start max-w-full ml-1 overflow-hidden"
			onMouseDown={(e) => e.stopPropagation()}
			onPointerDown={(e) => e.stopPropagation()}
		>
			<Select
				value={selectedOption?.selector ?? selectedSelector}
				onValueChange={(selector) =>
					setValue(convertJsonToUint8Array(selector))
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
					{projectOptions.length > 0 && (
						<SelectGroup>
							<SelectLabel>{pin.friendly_name}</SelectLabel>
							{projectOptions.map((option) => (
								<SelectItem key={option.selector} value={option.selector}>
									{option.label}
								</SelectItem>
							))}
						</SelectGroup>
					)}
					{packageOptions.length > 0 && (
						<SelectGroup>
							<SelectLabel>Packages</SelectLabel>
							{packageOptions.map((option) => (
								<SelectItem key={option.selector} value={option.selector}>
									{option.label}
									<span className="text-muted-foreground">
										· {option.packageId}
									</span>
								</SelectItem>
							))}
						</SelectGroup>
					)}
				</SelectContent>
			</Select>
		</div>
	);
}
