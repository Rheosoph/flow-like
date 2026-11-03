"use client";

import { useTranslation } from "@flow-like/locales";
import { ArrowUpIcon, BotIcon } from "lucide-react";
import { useCallback, useMemo, useRef, useState } from "react";
import { cn } from "../../lib/utils";
import { IdentityAvatar } from "../identity/identity-avatar";
import { Button } from "../ui/button";
import {
	type MentionQuery,
	type Mentionable,
	filterMentionables,
	insertMention,
	mentionQuery,
	mentionsIn,
} from "./mentions";

const MAX_HEIGHT = 160;

function fitHeight(input: HTMLTextAreaElement | null, value: string) {
	if (!input) return;
	input.style.height = "auto";
	if (value) {
		input.style.height = `${Math.min(input.scrollHeight, MAX_HEIGHT)}px`;
	}
}

function MentionMenu({
	options,
	active,
	onPick,
}: Readonly<{
	options: readonly Mentionable[];
	active: number;
	onPick: (option: Mentionable) => void;
}>) {
	const { t } = useTranslation("bots");
	return (
		<div
			aria-label={t("crew.mentions.label", "Mention someone")}
			className="absolute right-0 bottom-full left-0 z-10 mb-1.5 overflow-hidden rounded-xl border bg-popover p-1 text-popover-foreground shadow-md"
		>
			{options.map((option, index) => (
				<button
					key={option.id}
					type="button"
					aria-current={index === active}
					onMouseDown={(event) => {
						event.preventDefault();
						onPick(option);
					}}
					className={cn(
						"flex w-full items-center gap-2 rounded-lg px-2 py-1.5 text-left text-sm outline-none",
						index === active && "bg-accent text-accent-foreground",
					)}
				>
					<IdentityAvatar subject={option.id} size={22} />
					<span className="min-w-0 flex-1 truncate">{option.name}</span>
					{option.kind === "bot" && (
						<BotIcon
							className="size-3.5 text-muted-foreground"
							aria-label={t("crew.mentions.bot", "Bot")}
						/>
					)}
				</button>
			))}
		</div>
	);
}

export function ConversationComposer({
	recipient,
	hint,
	disabledReason,
	mentionables,
	onSend,
}: Readonly<{
	recipient: string;
	hint?: string | null;
	/** Shown instead of a hint while writing is not possible. */
	disabledReason?: string | null;
	/** Who `@` suggests; none turns the picker off. */
	mentionables?: readonly Mentionable[];
	onSend: (text: string, mentions: string[]) => void;
}>) {
	const { t } = useTranslation("bots");
	const [value, setValue] = useState("");
	const picked = useRef(new Map<string, string>());
	const [trigger, setTrigger] = useState<MentionQuery | null>(null);
	const [active, setActive] = useState(0);
	const suggestions = useMemo(
		() =>
			trigger && mentionables?.length
				? filterMentionables(mentionables, trigger.query)
				: [],
		[mentionables, trigger],
	);
	const inputRef = useRef<HTMLTextAreaElement>(null);
	const disabled = Boolean(disabledReason);
	const placeholder = t("crew.composer.placeholder", "Message {{name}}", {
		name: recipient,
	});

	const submit = useCallback(() => {
		const text = value.trim();
		if (!text || disabled) return;
		onSend(text, mentionsIn(text, picked.current));
		setValue("");
		setTrigger(null);
		picked.current.clear();
		fitHeight(inputRef.current, "");
	}, [disabled, onSend, value]);

	const pick = useCallback(
		(option: Mentionable) => {
			const input = inputRef.current;
			if (!trigger || !input) return;
			const next = insertMention(
				value,
				trigger,
				input.selectionStart ?? value.length,
				option.name,
			);
			picked.current.set(option.id, option.name);
			setValue(next.text);
			setTrigger(null);
			requestAnimationFrame(() => {
				input.focus();
				input.setSelectionRange(next.caret, next.caret);
			});
		},
		[trigger, value],
	);

	const track = (target: HTMLTextAreaElement) => {
		setTrigger(mentionQuery(target.value, target.selectionStart ?? 0));
		setActive(0);
	};

	return (
		<form
			className="shrink-0 border-t px-3.5 pt-2.5 pb-3.5"
			onSubmit={(event) => {
				event.preventDefault();
				submit();
			}}
		>
			<p className="mx-1 mb-2 truncate text-xs text-muted-foreground">
				{disabledReason ??
					hint ??
					t("crew.composer.to", "To {{name}}", { name: recipient })}
			</p>
			<div className="relative flex items-end gap-1.5 rounded-[14px] border bg-card p-1.5 focus-within:border-primary">
				{suggestions.length > 0 && (
					<MentionMenu options={suggestions} active={active} onPick={pick} />
				)}
				<textarea
					ref={inputRef}
					rows={1}
					value={value}
					disabled={disabled}
					onChange={(event) => {
						setValue(event.target.value);
						fitHeight(event.target, event.target.value);
						track(event.target);
					}}
					onKeyUp={(event) => {
						if (event.key.startsWith("Arrow") && suggestions.length === 0) {
							track(event.currentTarget);
						}
					}}
					onKeyDown={(event) => {
						if (suggestions.length > 0 && !event.nativeEvent.isComposing) {
							if (event.key === "ArrowDown" || event.key === "ArrowUp") {
								event.preventDefault();
								const step = event.key === "ArrowDown" ? 1 : -1;
								setActive(
									(index) =>
										(index + step + suggestions.length) % suggestions.length,
								);
								return;
							}
							if (event.key === "Enter" || event.key === "Tab") {
								event.preventDefault();
								pick(suggestions[active] ?? suggestions[0]);
								return;
							}
							if (event.key === "Escape") {
								event.preventDefault();
								setTrigger(null);
								return;
							}
						}
						if (
							event.key === "Enter" &&
							!event.shiftKey &&
							!event.nativeEvent.isComposing
						) {
							event.preventDefault();
							submit();
						}
					}}
					aria-label={placeholder}
					placeholder={placeholder}
					className="min-h-8.5 flex-1 resize-none bg-transparent px-2 py-1.5 text-sm outline-none placeholder:text-muted-foreground disabled:cursor-not-allowed"
				/>
				<Button
					type="submit"
					size="icon"
					className="size-8.5 shrink-0 rounded-[10px]"
					disabled={disabled || !value.trim()}
					aria-label={t("crew.composer.send", "Send")}
					title={t("crew.composer.send", "Send")}
				>
					<ArrowUpIcon aria-hidden />
				</Button>
			</div>
		</form>
	);
}
