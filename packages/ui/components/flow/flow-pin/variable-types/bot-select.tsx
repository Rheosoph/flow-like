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
import { useBotLabels, useBots } from "../../../../hooks/use-bots";
import { useSearch } from "../../../../hooks/use-search-index";
import type { IPin } from "../../../../lib/schema/flow/pin";
import {
	convertJsonToUint8Array,
	parseUint8ArrayToJson,
} from "../../../../lib/uint8";
import { cn } from "../../../../lib/utils";
import type { IBot } from "../../../../state/backend-state/bot-state";
import { useBotLocationText } from "../../../bots-tab/bot-labels";
import { BotAvatar } from "../../../bots/bot-avatar";
import {
	PIN_CHEVRON_CLASS,
	PIN_TRIGGER_CLASS,
	PinEditorRow,
	PinLabel,
} from "../pin-chrome";

interface BotOption {
	id: string;
	name: string;
	description: string;
	location: string;
	bot: IBot;
}

const SEARCH_OPTIONS = {
	fields: ["name", "description", "id"],
	boost: { name: 3 },
} as const;

function BotOptionItem({
	option,
	selected,
	onSelect,
}: Readonly<{
	option: BotOption;
	selected: boolean;
	onSelect: (id: string) => void;
}>) {
	return (
		<CommandItem
			value={option.id}
			onSelect={() => onSelect(option.id)}
			className="items-center gap-2"
		>
			<BotAvatar
				appearance={option.bot.appearance}
				size={20}
				animated={false}
				decorative
			/>
			<div className="flex min-w-0 flex-col gap-0.5">
				<span className="truncate text-xs">{option.name}</span>
				<span className="truncate text-[10px] text-muted-foreground">
					{option.location}
				</span>
			</div>
			{selected && <CheckIcon className="ml-auto size-3 shrink-0" />}
		</CommandItem>
	);
}

export function BotSelect({
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
	const bots = useBots();
	const labels = useBotLabels();
	const locationText = useBotLocationText();

	const options = useMemo<BotOption[]>(
		() =>
			(bots.data ?? [])
				.filter((bot) => !bot.local)
				.map((bot) => {
					const label = labels(bot);
					return {
						id: bot.id,
						name: label.name,
						description: label.description,
						location: locationText(bot.location).label,
						bot,
					};
				}),
		[bots.data, labels, locationText],
	);
	const matches = useSearch(options, search, SEARCH_OPTIONS);

	const stored = parseUint8ArrayToJson(value);
	const current = typeof stored === "string" ? stored.trim() : "";
	const selected = options.find((option) => option.id === current);

	const choose = (id: string) => {
		setValue(convertJsonToUint8Array(id));
		setOpen(false);
		setSearch("");
	};

	const triggerLabel =
		selected?.name || current || t("selectBot", "Select bot");

	return (
		<PinEditorRow>
			<Popover open={open} onOpenChange={setOpen}>
				<PopoverTrigger asChild>
					<button
						type="button"
						title={selected ? `${selected.name} (${selected.id})` : current}
						className={cn("flex cursor-pointer", PIN_TRIGGER_CLASS)}
					>
						<PinLabel text={triggerLabel} />
						<ChevronDown className={PIN_CHEVRON_CLASS} />
					</button>
				</PopoverTrigger>
				<PopoverContent className="w-72 p-0" align="start">
					<Command shouldFilter={false}>
						<CommandInput
							value={search}
							onValueChange={setSearch}
							placeholder={t("searchBots", "Search bots...")}
						/>
						<CommandList>
							<CommandEmpty>
								{bots.isLoading
									? t("loading", "Loading...")
									: t("noBotsFound", "No bot found.")}
							</CommandEmpty>
							{current && !selected && (
								<CommandGroup heading={t("currentBot", "Current value")}>
									<CommandItem
										value={`current:${current}`}
										onSelect={() => setOpen(false)}
									>
										<span className="truncate font-mono text-xs">
											{current}
										</span>
										<CheckIcon className="ml-auto size-3 shrink-0" />
									</CommandItem>
								</CommandGroup>
							)}
							{matches.length > 0 && (
								<CommandGroup>
									{matches.map((option) => (
										<BotOptionItem
											key={option.id}
											option={option}
											selected={option.id === current}
											onSelect={choose}
										/>
									))}
								</CommandGroup>
							)}
						</CommandList>
					</Command>
				</PopoverContent>
			</Popover>
		</PinEditorRow>
	);
}
