"use client";

import { useTranslation } from "@flow-like/locales";
import CheckIcon from "lucide-react/dist/esm/icons/check.js";
import ChevronDownIcon from "lucide-react/dist/esm/icons/chevron-down.js";
import CpuIcon from "lucide-react/dist/esm/icons/cpu.js";
import { useId, useState } from "react";
import { cn } from "../../../lib/utils";
import { Input } from "../../ui/input";
import { Popover, PopoverContent, PopoverTrigger } from "../../ui/popover";

export interface ChatBoxModel {
	readonly value: string;
	readonly options: readonly { id: string; label: string }[];
	readonly onChange: (id: string) => void;
}

export function ChatModelPicker({ value, options, onChange }: ChatBoxModel) {
	const { t } = useTranslation("chat");
	const [open, setOpen] = useState(false);
	const [search, setSearch] = useState("");
	const headingId = useId();
	const selected = options.find((option) => option.id === value) ?? options[0];
	if (!selected) return null;
	const triggerLabel = t("chatModelLabel", "Model: {{model}}", {
		model: selected.label,
	});
	const visible = options.filter((option) =>
		option.label
			.toLocaleLowerCase()
			.includes(search.trim().toLocaleLowerCase()),
	);

	return (
		<Popover
			open={open}
			onOpenChange={(next) => {
				setOpen(next);
				if (!next) setSearch("");
			}}
		>
			<PopoverTrigger asChild>
				<button
					type="button"
					aria-label={triggerLabel}
					title={triggerLabel}
					className="inline-flex h-11 min-w-0 shrink items-center gap-1 rounded-lg px-2 text-xs text-muted-foreground outline-none transition-colors hover:bg-accent focus-visible:ring-2 focus-visible:ring-primary/40 data-[state=open]:bg-accent sm:h-8"
				>
					<CpuIcon className="size-4 shrink-0" aria-hidden="true" />
					<span className="hidden max-w-32 truncate sm:inline">
						{selected.label}
					</span>
					<ChevronDownIcon className="size-3 shrink-0" aria-hidden="true" />
				</button>
			</PopoverTrigger>
			<PopoverContent
				side="top"
				align="start"
				sideOffset={8}
				className="w-72 p-1.5"
			>
				<p id={headingId} className="px-2 py-1.5 text-xs font-medium">
					{t("chatModelMenuTitle", "Model")}
				</p>
				<Input
					aria-label={t("searchChatModels", "Search models")}
					placeholder={t("searchChatModels", "Search models")}
					value={search}
					onChange={(event) => setSearch(event.target.value)}
					className="mb-1.5 h-8"
				/>
				<fieldset
					aria-labelledby={headingId}
					className="m-0 max-h-64 space-y-0.5 overflow-y-auto border-0 p-0"
				>
					{visible.map((option) => (
						<button
							key={option.id}
							type="button"
							aria-pressed={option.id === selected.id}
							onClick={() => {
								onChange(option.id);
								setOpen(false);
								setSearch("");
							}}
							className={cn(
								"flex w-full items-center justify-between gap-2 rounded-lg px-2 py-2 text-left text-sm outline-none hover:bg-accent focus-visible:bg-accent",
								option.id === selected.id && "bg-primary/10",
							)}
						>
							<span className="min-w-0 break-words">{option.label}</span>
							{option.id === selected.id && (
								<CheckIcon
									className="size-4 shrink-0 text-primary"
									aria-hidden="true"
								/>
							)}
						</button>
					))}
					{visible.length === 0 && (
						<p className="px-2 py-3 text-xs text-muted-foreground">
							{t("noMatchingChatModels", "No matching models.")}
						</p>
					)}
				</fieldset>
			</PopoverContent>
		</Popover>
	);
}
