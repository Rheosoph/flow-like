"use client";

import {
	type LanguageInfo,
	useLanguage,
	useTranslation,
} from "@flow-like/locales";
import { motion } from "framer-motion";
import { Check, Loader2 } from "lucide-react";
import { useCallback, useState } from "react";
import { toast } from "sonner";
import { cn } from "../lib/utils";
import { AnimatedLanguageIcon } from "./animated-icons/animated-language";
import {
	Command,
	CommandEmpty,
	CommandGroup,
	CommandInput,
	CommandItem,
	CommandList,
} from "./ui/command";
import { Popover, PopoverContent, PopoverTrigger } from "./ui/popover";
import { SidebarMenuButton } from "./ui/sidebar";

const MotionSidebarMenuButton = motion.create(SidebarMenuButton);

const iconVariants = {
	initial: { scale: 1, rotate: 0 },
	hover: {
		scale: 1.1,
		rotate: 5,
		transition: { type: "spring", stiffness: 400, damping: 10 },
	},
};

/** `pt-BR` reads as `PT` on the tile; the native name carries the region. */
function tile(code: string): string {
	return code.split("-")[0].toUpperCase();
}

/**
 * Sidebar footer entry that switches the UI language. Non-source catalogs are
 * separate chunks, so a pick is asynchronous — the row it happened on owns the
 * spinner until its catalog is in memory.
 */
export function LanguageSwitcher() {
	const { t } = useTranslation("common");
	const { current, available, setLanguage } = useLanguage();
	const [open, setOpen] = useState(false);
	const [pending, setPending] = useState<string | undefined>(undefined);

	const select = useCallback(
		async (code: string) => {
			if (code === current.code) {
				setOpen(false);
				return;
			}
			setPending(code);
			try {
				await setLanguage(code);
				setOpen(false);
			} catch (error) {
				toast.error(
					`${t("language")}: ${error instanceof Error ? error.message : String(error)}`,
				);
			} finally {
				setPending(undefined);
			}
		},
		[current.code, setLanguage, t],
	);

	return (
		<Popover open={open} onOpenChange={setOpen}>
			<PopoverTrigger asChild>
				<MotionSidebarMenuButton
					tooltip={`${t("language")} · ${current.nativeName}`}
					initial="initial"
					whileHover="hover"
					animate={open ? "hover" : "initial"}
				>
					<motion.div variants={iconVariants}>
						<AnimatedLanguageIcon />
					</motion.div>
					<span className="flex w-full flex-row items-center justify-between gap-2">
						<span className="truncate">{t("language")}</span>
						<span className="shrink-0 rounded-sm border border-border/60 bg-muted/60 px-1.5 py-px font-mono text-[10px] leading-4 tracking-wider text-muted-foreground">
							{tile(current.code)}
						</span>
					</span>
				</MotionSidebarMenuButton>
			</PopoverTrigger>
			<PopoverContent
				side="right"
				align="end"
				sideOffset={10}
				className="w-64 overflow-hidden p-0"
			>
				<div className="flex flex-row items-center gap-2.5 border-b border-border/60 bg-linear-to-br from-primary/15 via-primary/5 to-transparent px-3 py-2.5">
					<span className="flex size-7 shrink-0 items-center justify-center rounded-lg bg-background/70 ring-1 ring-primary/20">
						<AnimatedLanguageIcon />
					</span>
					<span className="flex min-w-0 flex-col">
						<span className="text-sm font-medium leading-tight">
							{t("language")}
						</span>
						<span className="truncate text-xs text-muted-foreground">
							{current.nativeName}
						</span>
					</span>
				</div>
				<Command>
					<CommandInput
						placeholder={t("searchLanguage")}
						className="h-9 text-sm"
					/>
					<CommandList className="max-h-72">
						<CommandEmpty className="text-sm text-muted-foreground">
							{t("noLanguageFound")}
						</CommandEmpty>
						<CommandGroup className="p-1.5">
							{available.map((language, index) => (
								<LanguageRow
									key={language.code}
									language={language}
									index={index}
									active={language.code === current.code}
									pending={pending === language.code}
									disabled={pending !== undefined}
									onSelect={select}
								/>
							))}
						</CommandGroup>
					</CommandList>
				</Command>
			</PopoverContent>
		</Popover>
	);
}

function LanguageRow({
	language,
	index,
	active,
	pending,
	disabled,
	onSelect,
}: Readonly<{
	language: LanguageInfo;
	index: number;
	active: boolean;
	pending: boolean;
	disabled: boolean;
	onSelect: (code: string) => void;
}>) {
	return (
		<CommandItem
			// Endonym and exonym both hit: "Deutsch" and "German" find `de`.
			value={`${language.code} ${language.nativeName} ${language.name}`}
			disabled={disabled && !pending}
			onSelect={() => onSelect(language.code)}
			// Framer cannot drive cmdk's item, so the stagger runs on CSS.
			style={{
				animationDelay: `${Math.min(index, 12) * 25}ms`,
				animationFillMode: "both",
			}}
			className={cn(
				"animate-in fade-in-0 slide-in-from-left-1 duration-200 gap-2.5 rounded-md px-2 py-1.5",
				active && "bg-primary/10 data-[selected=true]:bg-primary/15",
			)}
		>
			<span
				className={cn(
					"flex size-7 shrink-0 items-center justify-center rounded-md border font-mono text-[10px] font-semibold tracking-wider transition-colors",
					active
						? "border-primary/40 bg-primary/15 text-primary"
						: "border-border/60 bg-muted/60 text-muted-foreground",
				)}
			>
				{tile(language.code)}
			</span>
			<span className="flex min-w-0 flex-1 flex-col">
				<span
					className={cn("truncate text-sm", active && "font-medium")}
					lang={language.code}
					dir={language.rtl ? "rtl" : undefined}
				>
					{language.nativeName}
				</span>
				{language.name !== language.nativeName && (
					<span className="truncate text-xs text-muted-foreground">
						{language.name}
					</span>
				)}
			</span>
			{pending ? (
				<Loader2 className="size-4 shrink-0 animate-spin text-primary" />
			) : (
				active && (
					<Check className="size-4 shrink-0 animate-in zoom-in-50 text-primary" />
				)
			)}
		</CommandItem>
	);
}
