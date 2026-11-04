"use client";

import { useTranslation } from "@flow-like/locales";
import { BotIcon } from "lucide-react";
import {
	type KeyboardEvent,
	type ReactNode,
	type RefObject,
	useCallback,
	useMemo,
	useState,
} from "react";
import { cn } from "../../lib/utils";
import { IdentityAvatar } from "../identity/identity-avatar";
import {
	type MentionQuery,
	type Mentionable,
	filterMentionables,
	insertMention,
	mentionQuery,
} from "./mentions";

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

export interface MentionPicker {
	/** The suggestion list; place it inside a `relative` box around the input. */
	menu: ReactNode;
	/** Re-reads the `@word` before the caret; call on every change. */
	track: (input: HTMLTextAreaElement) => void;
	/** True when the key went to the open menu, so the input must not act on it. */
	handleKeyDown: (event: KeyboardEvent<HTMLTextAreaElement>) => boolean;
	handleKeyUp: (event: KeyboardEvent<HTMLTextAreaElement>) => void;
}

/**
 * `@` suggestions for a textarea: typing `@` lists `mentionables`, arrows move, Enter or Tab
 * inserts `@Name `, Escape closes. Without mentionables it does nothing.
 */
export function useMentionPicker({
	mentionables,
	value,
	onChange,
	inputRef,
}: Readonly<{
	mentionables?: readonly Mentionable[];
	value: string;
	onChange: (value: string) => void;
	inputRef: RefObject<HTMLTextAreaElement | null>;
}>): MentionPicker {
	const [trigger, setTrigger] = useState<MentionQuery | null>(null);
	const [active, setActive] = useState(0);
	const suggestions = useMemo(
		() =>
			trigger && mentionables?.length
				? filterMentionables(mentionables, trigger.query)
				: [],
		[mentionables, trigger],
	);

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
			onChange(next.text);
			setTrigger(null);
			requestAnimationFrame(() => {
				input.focus();
				input.setSelectionRange(next.caret, next.caret);
			});
		},
		[inputRef, onChange, trigger, value],
	);

	const track = useCallback(
		(input: HTMLTextAreaElement) => {
			setTrigger(
				mentionables?.length
					? mentionQuery(input.value, input.selectionStart ?? 0)
					: null,
			);
			setActive(0);
		},
		[mentionables],
	);

	const handleKeyDown = (event: KeyboardEvent<HTMLTextAreaElement>) => {
		if (suggestions.length === 0 || event.nativeEvent.isComposing) return false;
		if (event.key === "ArrowDown" || event.key === "ArrowUp") {
			const step = event.key === "ArrowDown" ? 1 : -1;
			setActive(
				(index) => (index + step + suggestions.length) % suggestions.length,
			);
		} else if (event.key === "Enter" || event.key === "Tab") {
			pick(suggestions[active] ?? suggestions[0]);
		} else if (event.key === "Escape") {
			setTrigger(null);
		} else {
			return false;
		}
		event.preventDefault();
		return true;
	};

	const handleKeyUp = (event: KeyboardEvent<HTMLTextAreaElement>) => {
		if (event.key.startsWith("Arrow") && suggestions.length === 0) {
			track(event.currentTarget);
		}
	};

	return {
		menu:
			suggestions.length > 0 ? (
				<MentionMenu options={suggestions} active={active} onPick={pick} />
			) : null,
		track,
		handleKeyDown,
		handleKeyUp,
	};
}
