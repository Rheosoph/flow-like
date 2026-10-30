"use client";

import type { Emoji, EmojiMartData } from "@emoji-mart/data";
import { useTranslation } from "@flow-like/locales";
import SmilePlusIcon from "lucide-react/dist/esm/icons/smile-plus.js";
import { useEffect, useMemo, useState } from "react";
import { cn } from "../../../lib/utils";
import { Input } from "../../ui/input";
import { Popover, PopoverContent, PopoverTrigger } from "../../ui/popover";

const COMMON_REACTIONS = ["👍", "❤️", "😂", "🎉", "🤔", "👀", "🙌", "🔥"];

interface ReactionProps {
	reactions?: readonly string[];
	onToggle?: (emoji: string) => void;
	disabled?: boolean;
}

export function MessageReactions({
	reactions,
	onToggle,
	disabled,
	alignEnd,
}: ReactionProps & { alignEnd?: boolean }) {
	const { t } = useTranslation("chat");
	if (!reactions?.length) return null;
	return (
		<fieldset
			className={cn("mt-2 flex flex-wrap gap-1.5", alignEnd && "justify-end")}
			aria-label={t("messageReactions", "Message reactions")}
		>
			{[...new Set(reactions)].map((emoji) => (
				<button
					key={emoji}
					type="button"
					aria-label={t("toggleReaction", "React with {{emoji}}", { emoji })}
					aria-pressed
					disabled={disabled || !onToggle}
					onClick={() => onToggle?.(emoji)}
					className="inline-flex min-h-8 min-w-9 items-center justify-center rounded-full border border-primary/25 bg-primary/10 px-2 text-base hover:bg-primary/20 focus-visible:outline-2 focus-visible:outline-ring disabled:cursor-default disabled:opacity-60"
				>
					{emoji}
				</button>
			))}
		</fieldset>
	);
}

export function MessageReactionPicker({
	reactions,
	onToggle,
	disabled,
}: ReactionProps & { onToggle: (emoji: string) => void }) {
	const { t } = useTranslation("chat");
	const [open, setOpen] = useState(false);
	const [search, setSearch] = useState("");
	const [emojis, setEmojis] = useState<Emoji[]>([]);
	const [loadFailed, setLoadFailed] = useState(false);

	useEffect(() => {
		if (!open) {
			setSearch("");
			return;
		}
		if (emojis.length) return;
		let active = true;
		setLoadFailed(false);
		import("@emoji-mart/data")
			.then((module) => {
				if (active) {
					const data = module.default as EmojiMartData;
					setEmojis(Object.values(data.emojis));
				}
			})
			.catch(() => {
				if (active) setLoadFailed(true);
			});
		return () => {
			active = false;
		};
	}, [open, emojis.length]);

	const results = useMemo(() => {
		const query = search.trim().toLowerCase();
		if (!query) {
			return COMMON_REACTIONS.map((native) => ({ native, name: native }));
		}
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
		<Popover open={open} onOpenChange={setOpen}>
			<PopoverTrigger asChild>
				<button
					type="button"
					disabled={disabled}
					aria-label={t("addReaction", "Add reaction")}
					title={t("addReaction", "Add reaction")}
					className="text-muted-foreground transition-colors hover:text-foreground focus-visible:outline-2 focus-visible:outline-ring disabled:opacity-50"
				>
					<SmilePlusIcon className="size-4" />
				</button>
			</PopoverTrigger>
			<PopoverContent
				align="start"
				className="w-72 space-y-2 p-3"
				aria-label={t("addReaction", "Add reaction")}
			>
				<Input
					value={search}
					onChange={(event) => setSearch(event.target.value)}
					placeholder={t("searchEmojis", "Search emojis")}
					aria-label={t("searchEmojis", "Search emojis")}
				/>
				<div className="grid max-h-60 grid-cols-8 gap-0.5 overflow-y-auto">
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
							className="flex size-8 items-center justify-center rounded-md text-xl hover:bg-accent focus-visible:outline-2 focus-visible:outline-ring aria-pressed:bg-primary/15 disabled:opacity-50"
							onClick={() => {
								onToggle(native);
								setOpen(false);
							}}
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
			</PopoverContent>
		</Popover>
	);
}
