"use client";

import { useTranslation } from "@flow-like/locales";
import { Check, ChevronsUpDown, Cpu } from "lucide-react";
import { type ReactNode, useId, useState } from "react";
import { cn } from "../../../../lib/utils";
import { Input } from "../../../ui/input";
import { Popover, PopoverContent, PopoverTrigger } from "../../../ui/popover";
import { BotTag } from "../bot-editor-parts";
import type { BotModelOption } from "./use-bot-models";

/** Language models also say where they run and whether they cost anything. */
type BotPickerOption = Pick<BotModelOption, "id" | "label"> &
	Partial<Pick<BotModelOption, "local" | "free">>;

/** The choice listed first that stands for `null`, such as letting Flow-Like pick. */
export interface BotPickerDefault {
	label: string;
	hint: string;
}

export function BotModelTags({
	option,
}: Readonly<{ option: BotPickerOption }>) {
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

/** A searchable model list in the style of the chat's model picker, sized for a form. */
export function BotModelPicker({
	value,
	options,
	onChange,
	label,
	fallback,
	exclude = [],
	trigger,
}: Readonly<{
	value: string | null;
	options: readonly BotPickerOption[];
	onChange: (id: string | null) => void;
	label: string;
	/** Offered first; picking it sets `null`. */
	fallback?: BotPickerDefault;
	exclude?: readonly string[];
	/** Replaces the field-style trigger, for example with an add button. */
	trigger?: ReactNode;
}>) {
	const { t } = useTranslation("bots");
	const [open, setOpen] = useState(false);
	const [search, setSearch] = useState("");
	const headingId = useId();
	const query = search.trim().toLocaleLowerCase();
	const visible = options.filter(
		(option) =>
			(option.id === value || !exclude.includes(option.id)) &&
			option.label.toLocaleLowerCase().includes(query),
	);
	const choose = (id: string | null) => {
		onChange(id);
		setOpen(false);
		setSearch("");
	};
	return (
		<Popover
			open={open}
			onOpenChange={(next) => {
				setOpen(next);
				if (!next) setSearch("");
			}}
		>
			<PopoverTrigger asChild>
				{trigger ?? (
					<BotModelField
						value={value}
						options={options}
						label={label}
						emptyLabel={fallback?.label ?? t("editor.brain.automatic")}
					/>
				)}
			</PopoverTrigger>
			<PopoverContent align="start" className="w-80 p-1.5">
				<p id={headingId} className="px-2 py-1.5 text-xs font-medium">
					{label}
				</p>
				<Input
					aria-label={t("editor.brain.searchModels")}
					placeholder={t("editor.brain.searchModels")}
					value={search}
					onChange={(event) => setSearch(event.target.value)}
					className="mb-1.5 h-8"
				/>
				<fieldset
					aria-labelledby={headingId}
					className="m-0 max-h-64 space-y-0.5 overflow-y-auto border-0 p-0"
				>
					{fallback && !query && (
						<ModelChoice
							label={fallback.label}
							hint={fallback.hint}
							selected={value === null}
							onSelect={() => choose(null)}
						/>
					)}
					{visible.map((option) => (
						<ModelChoice
							key={option.id}
							label={option.label}
							selected={option.id === value}
							onSelect={() => choose(option.id)}
						>
							<BotModelTags option={option} />
						</ModelChoice>
					))}
					{visible.length === 0 && (
						<p className="px-2 py-3 text-xs text-muted-foreground">
							{t("editor.brain.noModels")}
						</p>
					)}
				</fieldset>
			</PopoverContent>
		</Popover>
	);
}

function BotModelField({
	value,
	options,
	label,
	emptyLabel,
	...props
}: Readonly<{
	value: string | null;
	options: readonly BotPickerOption[];
	label: string;
	/** Shown while the value is `null`. */
	emptyLabel: string;
}>) {
	const { t } = useTranslation("bots");
	const selected = options.find((option) => option.id === value);
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
			<Cpu className="size-4 shrink-0 text-muted-foreground" aria-hidden />
			<span className="min-w-0 flex-1 truncate">{name}</span>
			{selected && <BotModelTags option={selected} />}
			<ChevronsUpDown
				className="size-3.5 shrink-0 text-muted-foreground"
				aria-hidden
			/>
		</button>
	);
}

function ModelChoice({
	label,
	hint,
	selected,
	onSelect,
	children,
}: Readonly<{
	label: string;
	hint?: string;
	selected: boolean;
	onSelect: () => void;
	children?: ReactNode;
}>) {
	return (
		<button
			type="button"
			aria-pressed={selected}
			onClick={onSelect}
			className={cn(
				"flex w-full items-center justify-between gap-2 rounded-lg px-2 py-2 text-left text-sm outline-none hover:bg-accent focus-visible:bg-accent",
				selected && "bg-primary/10",
			)}
		>
			<span className="flex min-w-0 flex-col">
				<span className="wrap-break-word">{label}</span>
				{hint && <span className="text-xs text-muted-foreground">{hint}</span>}
			</span>
			<span className="flex shrink-0 items-center gap-1">
				{children}
				{selected && <Check className="size-4 text-primary" aria-hidden />}
			</span>
		</button>
	);
}
