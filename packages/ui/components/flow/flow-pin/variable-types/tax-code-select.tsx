import { useTranslation } from "@flow-like/locales";
import { CheckIcon, ChevronDown } from "lucide-react";
import { useMemo, useState } from "react";
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
import { useSearch } from "../../../../hooks/use-search-index";
import type { IPin } from "../../../../lib/schema/flow/pin";
import {
	convertJsonToUint8Array,
	parseUint8ArrayToJson,
} from "../../../../lib/uint8";
import { cn } from "../../../../lib/utils";
import {
	type StripeTaxCode,
	TAX_CODE_GROUPS,
	type TaxCodeGroup,
	isStripeTaxCode,
	useStripeTaxCodes,
} from "../../../payments/tax-codes";
import {
	PIN_CHEVRON_CLASS,
	PIN_TRIGGER_CLASS,
	PinEditorRow,
	PinLabel,
} from "../pin-chrome";

const SEARCH_OPTIONS = {
	fields: ["id", "name", "description"],
	boost: { id: 3, name: 3 },
} as const;

function TaxCodeItem({
	code,
	selected,
	onSelect,
}: Readonly<{
	code: StripeTaxCode;
	selected: boolean;
	onSelect: (id: string) => void;
}>) {
	return (
		<CommandItem
			value={code.id}
			onSelect={() => onSelect(code.id)}
			className="items-start"
		>
			<div className="flex min-w-0 flex-col gap-0.5">
				<span className="text-xs">{code.name}</span>
				<span className="line-clamp-2 text-[10px] text-muted-foreground">
					{code.description}
				</span>
				<span className="font-mono text-[10px] text-muted-foreground">
					{code.id}
				</span>
			</div>
			{selected && <CheckIcon className="ml-auto size-3 shrink-0" />}
		</CommandItem>
	);
}

export function TaxCodeSelect({
	pin,
	value,
	setValue,
}: Readonly<{
	pin: IPin;
	value: number[] | undefined | null;
	setValue: (value: unknown) => void;
}>) {
	const { t } = useTranslation("flow");
	const [open, setOpen] = useState(false);
	const [search, setSearch] = useState("");
	const codes = useStripeTaxCodes();
	const matches = useSearch(codes.data, search, SEARCH_OPTIONS);

	const stored = parseUint8ArrayToJson(value);
	const current = typeof stored === "string" ? stored.trim() : "";
	const selected = codes.data?.find((code) => code.id === current);
	const typed = search.trim().toLowerCase();
	const customCode =
		isStripeTaxCode(typed) && !codes.data?.some((code) => code.id === typed)
			? typed
			: undefined;

	const headings: Record<TaxCodeGroup, string> = {
		common: t("taxCodeGroupCommon", "Common"),
		digital: t("taxCodeGroupDigital", "Digital products"),
		services: t("taxCodeGroupServices", "Services"),
		events: t("taxCodeGroupEvents", "Events and admissions"),
		physical: t("taxCodeGroupPhysical", "Physical goods"),
	};

	const grouped = useMemo(
		() =>
			TAX_CODE_GROUPS.map(
				(group) =>
					[group, matches.filter((code) => code.group === group)] as const,
			).filter(([, items]) => items.length > 0),
		[matches],
	);

	const choose = (id: string) => {
		setValue(convertJsonToUint8Array(id));
		setOpen(false);
		setSearch("");
	};

	const triggerLabel =
		selected?.name || current || t("selectTaxCategory", "Select tax category");

	return (
		<PinEditorRow>
			<Popover open={open} onOpenChange={setOpen}>
				<PopoverTrigger asChild>
					<button
						type="button"
						title={selected ? `${selected.name} (${selected.id})` : undefined}
						className={cn("flex cursor-pointer", PIN_TRIGGER_CLASS)}
					>
						<PinLabel text={triggerLabel} />
						<ChevronDown className={PIN_CHEVRON_CLASS} />
					</button>
				</PopoverTrigger>
				<PopoverContent className="w-80 p-0" align="start">
					<Command shouldFilter={false}>
						<CommandInput
							value={search}
							onValueChange={setSearch}
							placeholder={t(
								"searchTaxCodes",
								"Search categories or txcd_ codes...",
							)}
						/>
						<CommandList>
							<CommandEmpty>
								{codes.isLoading
									? t("loading", "Loading...")
									: t("noTaxCodesFound", "No tax category found.")}
							</CommandEmpty>
							{customCode && (
								<CommandGroup heading={pin.friendly_name}>
									<CommandItem
										value={customCode}
										onSelect={() => choose(customCode)}
									>
										<span className="text-xs">
											{t("useTaxCode", "Use {{code}}", { code: customCode })}
										</span>
									</CommandItem>
								</CommandGroup>
							)}
							{grouped.map(([group, items]) => (
								<CommandGroup key={group} heading={headings[group]}>
									{items.map((code) => (
										<TaxCodeItem
											key={code.id}
											code={code}
											selected={code.id === current}
											onSelect={choose}
										/>
									))}
								</CommandGroup>
							))}
						</CommandList>
					</Command>
				</PopoverContent>
			</Popover>
		</PinEditorRow>
	);
}
