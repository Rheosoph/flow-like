"use client";

import type { Emoji, EmojiMartData } from "@emoji-mart/data";
import { useTranslation } from "@flow-like/locales";
import BotIcon from "lucide-react/dist/esm/icons/bot.js";
import SmilePlusIcon from "lucide-react/dist/esm/icons/smile-plus.js";
import { useEffect, useId, useMemo, useRef, useState } from "react";
import { cn } from "../../../lib/utils";
import { Input } from "../../ui/input";
import {
	Popover,
	PopoverAnchor,
	PopoverContent,
	PopoverTrigger,
} from "../../ui/popover";

const COMMON_REACTIONS = [
	"👍",
	"❤️",
	"😂",
	"🎉",
	"🤔",
	"👀",
	"🙌",
	"🔥",
	"😊",
	"😍",
	"😎",
	"🤯",
	"😮",
	"😢",
	"😅",
	"👎",
	"👏",
	"🙏",
	"🤝",
	"💪",
	"✅",
	"💯",
	"🚀",
	"💡",
];

interface ReactionProps {
	reactions?: readonly string[];
	onToggle?: (emoji: string) => void;
	disabled?: boolean;
}

const PICKER_CLASS = "w-80 max-w-[calc(100vw-2rem)] space-y-2 p-3";

function EmojiChoices({
	reactions,
	onSelect,
	disabled,
}: {
	reactions?: readonly string[];
	onSelect: (emoji: string) => void;
	disabled?: boolean;
}) {
	const { t } = useTranslation("chat");
	const [search, setSearch] = useState("");
	const [emojis, setEmojis] = useState<Emoji[]>([]);
	const [loadFailed, setLoadFailed] = useState(false);
	useEffect(() => {
		let active = true;
		import("@emoji-mart/data")
			.then((module) => {
				if (active)
					setEmojis(Object.values((module.default as EmojiMartData).emojis));
			})
			.catch(() => {
				if (active) setLoadFailed(true);
			});
		return () => {
			active = false;
		};
	}, []);
	const results = useMemo(() => {
		const query = search.trim().toLowerCase();
		if (!query)
			return COMMON_REACTIONS.map((native) => ({ native, name: native }));
		return emojis
			.filter((emoji) =>
				[
					emoji.name,
					emoji.id,
					...emoji.keywords,
					...emoji.skins.map((skin) => skin.native),
				].some((term) => term.toLowerCase().includes(query)),
			)
			.flatMap((emoji) =>
				emoji.skins.map((skin) => ({ native: skin.native, name: emoji.name })),
			)
			.slice(0, 80);
	}, [emojis, search]);
	return (
		<>
			<Input
				value={search}
				onChange={(event) => setSearch(event.target.value)}
				placeholder={t("searchEmojis", "Search emojis")}
				aria-label={t("searchEmojis", "Search emojis")}
			/>
			<div className="grid max-h-60 grid-cols-[repeat(auto-fit,minmax(2rem,1fr))] gap-0.5 overflow-x-hidden overflow-y-auto overscroll-contain p-0.5 [scrollbar-width:thin]">
				{results.map(({ native, name }) => (
					<button
						key={native}
						type="button"
						title={name}
						aria-label={t("toggleReaction", "React with {{emoji}}", {
							emoji: native,
						})}
						aria-pressed={reactions?.includes(native) ?? false}
						disabled={disabled}
						className="flex h-9 min-w-0 items-center justify-center rounded-md text-xl hover:bg-accent focus-visible:outline-2 focus-visible:outline-ring aria-pressed:bg-primary/15 disabled:opacity-50"
						onClick={() => onSelect(native)}
					>
						{native}
					</button>
				))}
			</div>
			{results.length === 0 && (
				<output className="block text-sm text-muted-foreground">
					{loadFailed
						? t("emojiSearchUnavailable", "Emoji search is unavailable")
						: emojis.length
							? t("noEmojisFound", "No emojis found")
							: t("loadingEmojis", "Loading emojis…")}
				</output>
			)}
		</>
	);
}

function ReactionBadge({
	emoji,
	selected,
	fromWorkflow,
	reactions,
	onToggle,
	onReplace,
	disabled,
}: {
	emoji: string;
	selected: boolean;
	fromWorkflow: boolean;
	reactions?: readonly string[];
	onToggle?: (emoji: string) => void;
	onReplace?: (previousEmoji: string, emoji: string) => void;
	disabled?: boolean;
}) {
	const { t } = useTranslation("chat");
	const descriptionId = useId();
	const popupId = useId();
	const button = useRef<HTMLButtonElement>(null);
	const [open, setOpen] = useState(false);
	const mode = useRef<"hover" | "keyboard">("hover");
	const openTimer = useRef<ReturnType<typeof setTimeout> | undefined>(
		undefined,
	);
	const closeTimer = useRef<ReturnType<typeof setTimeout> | undefined>(
		undefined,
	);
	const canChange = Boolean(selected ? onReplace : onToggle) && !disabled;
	const clearTimers = () => {
		clearTimeout(openTimer.current);
		clearTimeout(closeTimer.current);
	};
	useEffect(
		() => () => {
			clearTimeout(openTimer.current);
			clearTimeout(closeTimer.current);
		},
		[],
	);
	useEffect(() => {
		if (!canChange) setOpen(false);
	}, [canChange]);
	const closeAfterHover = () => {
		clearTimers();
		if (mode.current === "hover")
			closeTimer.current = setTimeout(() => setOpen(false), 200);
	};
	const attribution = fromWorkflow
		? t("workflowReactedWith", "Workflow reacted with {{emoji}}", { emoji })
		: undefined;
	const hint =
		canChange && selected
			? t(
					"editReactionHint",
					"Click to remove. Hover or press Arrow Down to change.",
				)
			: undefined;
	return (
		<Popover
			open={open}
			onOpenChange={(next) => {
				clearTimers();
				setOpen(next);
			}}
		>
			<PopoverAnchor asChild>
				<button
					ref={button}
					type="button"
					aria-label={t("toggleReaction", "React with {{emoji}}", { emoji })}
					aria-pressed={selected}
					aria-describedby={attribution || hint ? descriptionId : undefined}
					aria-haspopup={canChange ? "dialog" : undefined}
					aria-expanded={canChange ? open : undefined}
					aria-controls={open ? popupId : undefined}
					data-state={open ? "open" : "closed"}
					title={open ? undefined : (attribution ?? hint)}
					disabled={disabled || !onToggle}
					onClick={() => {
						clearTimers();
						setOpen(false);
						onToggle?.(emoji);
					}}
					onPointerEnter={(event) => {
						if (!canChange || event.pointerType !== "mouse") return;
						clearTimers();
						if (open) return;
						mode.current = "hover";
						openTimer.current = setTimeout(() => setOpen(true), 200);
					}}
					onPointerLeave={closeAfterHover}
					onKeyDown={(event) => {
						if (
							canChange &&
							(event.key === "ArrowDown" || event.key === "F2")
						) {
							event.preventDefault();
							clearTimers();
							mode.current = "keyboard";
							setOpen(true);
						}
					}}
					className="relative inline-flex size-8 shrink-0 items-center justify-center rounded-full border border-border bg-popover text-lg hover:bg-muted focus-visible:outline-2 focus-visible:outline-ring aria-pressed:border-primary/25 disabled:cursor-default disabled:opacity-60"
				>
					{emoji}
					{fromWorkflow && (
						<span className="absolute -bottom-0.5 -right-0.5 rounded-full border border-border bg-popover p-0.5">
							<BotIcon
								aria-hidden="true"
								className="size-2.5 text-muted-foreground"
							/>
						</span>
					)}
					{(attribution || hint) && (
						<span id={descriptionId} className="sr-only">
							{[attribution, hint].filter(Boolean).join(". ")}
						</span>
					)}
				</button>
			</PopoverAnchor>
			<PopoverContent
				id={popupId}
				side="top"
				align="end"
				className={PICKER_CLASS}
				aria-label={t("changeReaction", "Change reaction")}
				onPointerEnter={clearTimers}
				onPointerLeave={closeAfterHover}
				onFocusCapture={() => {
					clearTimers();
					mode.current = "keyboard";
				}}
				onOpenAutoFocus={(event) => {
					if (mode.current === "hover") event.preventDefault();
				}}
				onCloseAutoFocus={(event) => {
					event.preventDefault();
					if (mode.current === "keyboard") button.current?.focus();
				}}
			>
				<EmojiChoices
					reactions={reactions}
					disabled={disabled}
					onSelect={(next) => {
						clearTimers();
						setOpen(false);
						if (!selected) onToggle?.(next);
						else if (next !== emoji) onReplace?.(emoji, next);
					}}
				/>
			</PopoverContent>
		</Popover>
	);
}

export function MessageReactions({
	reactions,
	workflowReactions,
	onToggle,
	onReplace,
	disabled,
	alignEnd,
	className,
}: ReactionProps & {
	workflowReactions?: readonly string[];
	onReplace?: (previousEmoji: string, emoji: string) => void;
	alignEnd?: boolean;
	className?: string;
}) {
	const { t } = useTranslation("chat");
	const displayed = [
		...new Set([...(reactions ?? []), ...(workflowReactions ?? [])]),
	];
	if (!displayed.length) return null;
	return (
		<fieldset
			className={cn(
				"mt-2 flex flex-wrap gap-1.5",
				alignEnd && "justify-end",
				className,
			)}
			aria-label={t("messageReactions", "Message reactions")}
		>
			{displayed.map((emoji) => (
				<ReactionBadge
					key={emoji}
					emoji={emoji}
					selected={reactions?.includes(emoji) ?? false}
					fromWorkflow={workflowReactions?.includes(emoji) ?? false}
					reactions={reactions}
					onToggle={onToggle}
					onReplace={onReplace}
					disabled={disabled}
				/>
			))}
		</fieldset>
	);
}

export function MessageReactionPicker({
	reactions,
	onToggle,
	disabled,
	triggerClassName,
}: ReactionProps & {
	onToggle: (emoji: string) => void;
	triggerClassName?: string;
}) {
	const { t } = useTranslation("chat");
	const [open, setOpen] = useState(false);
	return (
		<Popover open={open} onOpenChange={setOpen}>
			<PopoverTrigger asChild>
				<button
					type="button"
					disabled={disabled}
					aria-label={t("addReaction", "Add reaction")}
					title={t("addReaction", "Add reaction")}
					className={cn(
						"inline-flex size-8 items-center justify-center rounded-md text-muted-foreground transition-colors hover:bg-muted hover:text-foreground focus-visible:outline-2 focus-visible:outline-ring disabled:opacity-50",
						triggerClassName,
					)}
				>
					<SmilePlusIcon className="size-4" />
				</button>
			</PopoverTrigger>
			<PopoverContent
				side="top"
				align="end"
				className={PICKER_CLASS}
				aria-label={t("addReaction", "Add reaction")}
			>
				<EmojiChoices
					reactions={reactions}
					disabled={disabled}
					onSelect={(emoji) => {
						onToggle(emoji);
						setOpen(false);
					}}
				/>
			</PopoverContent>
		</Popover>
	);
}
