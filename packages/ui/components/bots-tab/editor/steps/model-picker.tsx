"use client";

import { useTranslation } from "@flow-like/locales";
import {
	Check,
	ChevronsUpDown,
	Cpu,
	Plus,
	Search,
	Sparkles,
} from "lucide-react";
import { type ReactNode, useId, useMemo, useRef, useState } from "react";
import { useAddModelToProfile } from "../../../../hooks/use-add-model-to-profile";
import { cn } from "../../../../lib/utils";
import { Input } from "../../../ui/input";
import {
	DeploymentLabel,
	ProviderGlyph,
	providerLabel,
} from "../../../ui/model-kit";
import { Popover, PopoverContent, PopoverTrigger } from "../../../ui/popover";
import { Skeleton } from "../../../ui/skeleton";
import { Tabs, TabsList, TabsTrigger } from "../../../ui/tabs";
import { BotTag } from "../bot-editor-parts";
import {
	type BotModelOption,
	type ModelCatalogSpec,
	useCatalogModelOptions,
} from "./use-bot-models";

/** The choice listed first that stands for `null`, such as letting Flow-Like pick. */
export interface BotPickerDefault {
	label: string;
	hint: string;
}

type PickerTab = "mine" | "all";

export function BotModelTags({
	option,
}: Readonly<{ option: Pick<BotModelOption, "local" | "free"> }>) {
	const { t } = useTranslation("bots");
	return (
		<>
			{option.local && (
				<BotTag className="text-[0.625rem]">{t("editor.brain.local")}</BotTag>
			)}
			{option.free && (
				<BotTag className="text-[0.625rem]">{t("editor.brain.free")}</BotTag>
			)}
		</>
	);
}

function haystack(option: BotModelOption): string {
	const modelId = option.bit.parameters?.provider?.model_id;
	return [
		option.label,
		option.id,
		providerLabel(option.bit),
		typeof modelId === "string" ? modelId : "",
	]
		.join(" ")
		.toLocaleLowerCase();
}

/** The person's own models first, then hosted ones, which work wherever the bot runs. */
function catalogOrder(
	options: readonly BotModelOption[],
	mine: ReadonlySet<string>,
): BotModelOption[] {
	return [...options].sort(
		(a, b) =>
			Number(mine.has(b.id)) - Number(mine.has(a.id)) ||
			Number(a.local) - Number(b.local) ||
			a.label.localeCompare(b.label),
	);
}

/**
 * A searchable model picker sized for a form. It lists the profile's models and the whole catalog;
 * with nothing fitting in the profile it opens on the catalog, so people without the model catalog
 * page can still choose. A catalog model is used at once and added to the profile in the background.
 */
export function BotModelPicker({
	value,
	options,
	catalog,
	onChange,
	label,
	fallback,
	placeholder,
	exclude = [],
	trigger,
	onAddSettled,
	profileId,
}: Readonly<{
	value: string | null;
	/** The profile's models that fit. */
	options: readonly BotModelOption[];
	catalog: ModelCatalogSpec;
	onChange: (id: string | null) => void;
	label: string;
	/** Offered first; picking it sets `null`. */
	fallback?: BotPickerDefault;
	/** Shown on the field while nothing is chosen and there is no `fallback`. */
	placeholder?: string;
	exclude?: readonly string[];
	/** Replaces the field-style trigger, for example with an add button. */
	trigger?: ReactNode;
	/** A catalog pick finished being added to the profile, or failed to be. */
	onAddSettled?: (id: string, added: boolean) => void;
	/** Adds picks to this profile instead of the active one, models it already holds included. */
	profileId?: string;
}>) {
	const { t } = useTranslation("bots");
	const [open, setOpen] = useState(false);
	const [picked, setPicked] = useState<BotModelOption | null>(null);
	const contentRef = useRef<HTMLDivElement>(null);
	const selected =
		options.find((option) => option.id === value) ??
		(picked?.id === value ? picked : undefined);
	const choose = (option: BotModelOption | null, added?: Promise<boolean>) => {
		setPicked(option);
		onChange(option?.id ?? null);
		if (option && added && onAddSettled) {
			void added.then((ok) => onAddSettled(option.id, ok));
		}
		setOpen(false);
	};
	return (
		<Popover open={open} onOpenChange={setOpen} modal>
			<PopoverTrigger asChild>
				{trigger ?? (
					<BotModelField
						value={value}
						selected={selected}
						label={label}
						emptyLabel={
							fallback?.label ?? placeholder ?? t("editor.brain.automatic")
						}
					/>
				)}
			</PopoverTrigger>
			<PopoverContent
				ref={contentRef}
				align="start"
				collisionPadding={8}
				className="w-[min(26rem,calc(100vw-2rem))] overflow-hidden p-0"
				onOpenAutoFocus={(event) => {
					event.preventDefault();
					contentRef.current?.querySelector("input")?.focus();
				}}
			>
				{open && (
					<ModelPickerPanel
						value={value}
						options={options}
						catalog={catalog}
						label={label}
						fallback={fallback}
						exclude={exclude}
						profileId={profileId}
						onChoose={choose}
					/>
				)}
			</PopoverContent>
		</Popover>
	);
}

function ModelPickerPanel({
	value,
	options,
	catalog,
	label,
	fallback,
	exclude,
	profileId,
	onChoose,
}: Readonly<{
	value: string | null;
	options: readonly BotModelOption[];
	catalog: ModelCatalogSpec;
	label: string;
	fallback?: BotPickerDefault;
	exclude: readonly string[];
	profileId?: string;
	/** `added` settles once a catalog pick is in the profile, or could not be put there. */
	onChoose: (option: BotModelOption | null, added?: Promise<boolean>) => void;
}>) {
	const { t } = useTranslation("bots");
	const headingId = useId();
	const [search, setSearch] = useState("");
	const [chosenTab, setChosenTab] = useState<PickerTab | null>(null);
	const hasMine = options.length > 0;
	const tab: PickerTab = chosenTab ?? (hasMine ? "mine" : "all");
	const all = useCatalogModelOptions(catalog, tab === "all");
	const { allows, add } = useAddModelToProfile(undefined, profileId);
	const mine = useMemo(
		() => new Set(options.map((option) => option.id)),
		[options],
	);
	const query = search.trim().toLocaleLowerCase();
	const visible = useMemo(
		() =>
			(tab === "mine" ? options : catalogOrder(all.options, mine)).filter(
				(option) =>
					(option.id === value || !exclude.includes(option.id)) &&
					haystack(option).includes(query),
			),
		[tab, options, all.options, mine, value, exclude, query],
	);
	const pick = (option: BotModelOption) => {
		if (mine.has(option.id) && !profileId) onChoose(option);
		else if (allows(option.bit)) onChoose(option, add(option.bit));
	};
	const loadingCatalog = tab === "all" && all.isLoading;
	return (
		<div className="flex max-h-[min(32rem,var(--radix-popover-content-available-height,70vh))] flex-col">
			<div className="flex flex-col gap-2 border-b p-3">
				<div className="flex min-h-8 items-center justify-between gap-2">
					<p id={headingId} className="truncate text-sm font-medium">
						{label}
					</p>
					{hasMine && (
						<Tabs
							value={tab}
							onValueChange={(next) => setChosenTab(next as PickerTab)}
						>
							<TabsList className="h-8">
								<TabsTrigger value="mine" className="px-2.5 text-xs">
									{t("editor.brain.yourModels", "Your models")}
								</TabsTrigger>
								<TabsTrigger value="all" className="px-2.5 text-xs">
									{t("editor.brain.allModels", "All models")}
								</TabsTrigger>
							</TabsList>
						</Tabs>
					)}
				</div>
				<div className="relative">
					<Search
						className="pointer-events-none absolute top-1/2 left-2.5 size-3.5 -translate-y-1/2 text-muted-foreground"
						aria-hidden
					/>
					<Input
						aria-label={t("editor.brain.searchModels")}
						placeholder={t("editor.brain.searchModels")}
						value={search}
						onChange={(event) => setSearch(event.target.value)}
						className="h-8 pl-8"
					/>
				</div>
			</div>
			{!hasMine && (
				<p className="mx-3 mt-3 flex items-start gap-2 rounded-lg bg-muted/60 px-3 py-2 text-xs leading-relaxed text-muted-foreground">
					<Sparkles
						className="mt-0.5 size-3.5 shrink-0 text-primary"
						aria-hidden
					/>
					{t(
						"editor.brain.catalogNotice",
						"None of your models fit here yet. Pick one from the catalog and it is added to your models.",
					)}
				</p>
			)}
			<div className="min-h-0 flex-1 overflow-y-auto">
				<fieldset
					aria-labelledby={headingId}
					aria-busy={loadingCatalog || undefined}
					className="m-0 min-w-0 space-y-0.5 border-0 p-1.5"
				>
					{fallback && !query && (
						<FallbackChoice
							fallback={fallback}
							selected={value === null}
							onSelect={() => onChoose(null)}
						/>
					)}
					{visible.map((option) => (
						<ModelChoice
							key={option.id}
							option={option}
							selected={option.id === value}
							inProfile={mine.has(option.id)}
							onSelect={() => pick(option)}
						/>
					))}
					{loadingCatalog && <ModelChoiceSkeletons />}
					{!loadingCatalog && visible.length === 0 && (
						<p className="px-2 py-4 text-center text-xs text-muted-foreground">
							{tab === "all" && all.isError
								? t(
										"editor.brain.catalogFailed",
										"The model catalog couldn't be loaded.",
									)
								: t("editor.brain.noModels")}
						</p>
					)}
				</fieldset>
			</div>
		</div>
	);
}

function BotModelField({
	value,
	selected,
	label,
	emptyLabel,
	...props
}: Readonly<{
	value: string | null;
	selected: BotModelOption | undefined;
	label: string;
	/** Shown while the value is `null`. */
	emptyLabel: string;
}>) {
	const { t } = useTranslation("bots");
	const name =
		value === null
			? emptyLabel
			: (selected?.label ?? t("editor.brain.unknownModel", { id: value }));
	return (
		<button
			type="button"
			aria-label={`${label}: ${name}`}
			className="flex h-10 w-full min-w-0 items-center gap-2 rounded-lg border border-input bg-background px-3 text-left text-sm outline-none transition-colors hover:bg-muted/50 focus-visible:ring-2 focus-visible:ring-ring data-[state=open]:border-ring"
			{...props}
		>
			{selected ? (
				<ProviderGlyph bit={selected.bit} size={20} className="rounded-md" />
			) : (
				<Cpu className="size-4 shrink-0 text-muted-foreground" aria-hidden />
			)}
			<span
				className={cn(
					"min-w-0 flex-1 truncate",
					value === null && "text-muted-foreground",
				)}
			>
				{name}
			</span>
			{selected && <BotModelTags option={selected} />}
			<ChevronsUpDown
				className="size-3.5 shrink-0 text-muted-foreground"
				aria-hidden
			/>
		</button>
	);
}

const CHOICE =
	"group flex w-full items-center gap-3 rounded-lg px-2 py-2 text-left outline-none transition-colors hover:bg-accent focus-visible:bg-accent";

function FallbackChoice({
	fallback,
	selected,
	onSelect,
}: Readonly<{
	fallback: BotPickerDefault;
	selected: boolean;
	onSelect: () => void;
}>) {
	return (
		<button
			type="button"
			aria-pressed={selected}
			onClick={onSelect}
			className={cn(CHOICE, selected && "bg-primary/10")}
		>
			<span className="grid size-8 shrink-0 place-items-center rounded-[10px] border border-dashed border-border bg-muted/50 text-muted-foreground">
				<Sparkles className="size-4" aria-hidden />
			</span>
			<span className="flex min-w-0 flex-1 flex-col">
				<span className="text-sm font-medium">{fallback.label}</span>
				<span className="text-xs text-muted-foreground">{fallback.hint}</span>
			</span>
			<ChoiceMark selected={selected} />
		</button>
	);
}

function ModelChoice({
	option,
	selected,
	inProfile,
	onSelect,
}: Readonly<{
	option: BotModelOption;
	selected: boolean;
	inProfile: boolean;
	onSelect: () => void;
}>) {
	const { t, i18n } = useTranslation("bots");
	const meta = option.bit.meta?.[i18n.language] ?? option.bit.meta?.en;
	const description = meta?.description?.trim();
	const addLabel = t("editor.brain.addAndUse", "Add to your models and use");
	return (
		<button
			type="button"
			aria-pressed={selected}
			onClick={onSelect}
			title={inProfile ? undefined : addLabel}
			className={cn(CHOICE, selected && "bg-primary/10")}
		>
			<ProviderGlyph bit={option.bit} size={32} />
			<span className="flex min-w-0 flex-1 flex-col gap-0.5">
				<span className="flex min-w-0 items-center gap-1.5">
					<span className="truncate text-sm font-medium">{option.label}</span>
					{option.free && (
						<BotTag className="shrink-0 px-1.5 py-0.5 text-[0.625rem]">
							{t("editor.brain.free")}
						</BotTag>
					)}
				</span>
				<span className="flex min-w-0 items-center gap-2 text-[0.6875rem] text-muted-foreground">
					{!option.local && (
						<span className="truncate">{providerLabel(option.bit)}</span>
					)}
					<DeploymentLabel
						kind={option.local ? "local" : "hosted"}
						className="shrink-0"
					/>
				</span>
				{description && (
					<span className="line-clamp-1 text-xs text-muted-foreground/80">
						{description}
					</span>
				)}
			</span>
			{selected || inProfile ? (
				<ChoiceMark selected={selected} />
			) : (
				<span className="grid size-6 shrink-0 place-items-center rounded-full border border-border text-muted-foreground transition-colors group-hover:border-primary group-hover:text-primary">
					<Plus className="size-3.5" aria-hidden />
					<span className="sr-only">{addLabel}</span>
				</span>
			)}
		</button>
	);
}

function ChoiceMark({ selected }: Readonly<{ selected: boolean }>) {
	return (
		<span className="grid size-6 shrink-0 place-items-center">
			{selected && <Check className="size-4 text-primary" aria-hidden />}
		</span>
	);
}

function ModelChoiceSkeletons() {
	return (
		<>
			{[0, 1, 2].map((row) => (
				<div key={row} className="flex items-center gap-3 px-2 py-2">
					<Skeleton className="size-8 shrink-0 rounded-[10px]" />
					<div className="flex flex-1 flex-col gap-1.5">
						<Skeleton className="h-3.5 w-2/5" />
						<Skeleton className="h-3 w-3/5" />
					</div>
				</div>
			))}
		</>
	);
}
