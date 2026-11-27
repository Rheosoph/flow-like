"use client";

import { useTranslation } from "@flow-like/locales";
import { LockIcon } from "lucide-react";
import {
	type KeyboardEvent,
	type ReactNode,
	type RefObject,
	useCallback,
	useMemo,
	useState,
} from "react";
import { cn } from "../../lib/utils";
import { TeamTile } from "./org-chart/team-tile";
import {
	type TeamTagOption,
	type TeamTagQuery,
	insertTeamTag,
	leadingTagQuery,
	matchTeamTags,
} from "./team-tag-model";

function TeamTagMenu({
	options,
	active,
	onPick,
}: Readonly<{
	options: readonly TeamTagOption[];
	active: number;
	onPick: (option: TeamTagOption) => void;
}>) {
	const { t } = useTranslation("bots");
	return (
		<div
			aria-label={t("teams.requests.picker.label", "Ask a team")}
			className="absolute right-0 bottom-full left-0 z-10 mb-1.5 overflow-hidden rounded-xl border bg-popover p-1 text-popover-foreground shadow-md"
		>
			{options.map((option, index) => (
				<button
					key={option.id}
					type="button"
					aria-current={index === active}
					aria-disabled={Boolean(option.lockedLevel)}
					onMouseDown={(event) => {
						event.preventDefault();
						if (!option.lockedLevel) onPick(option);
					}}
					className={cn(
						"flex w-full items-center gap-2 rounded-lg px-2 py-1.5 text-left text-sm outline-none",
						index === active && "bg-accent text-accent-foreground",
						option.lockedLevel && "cursor-not-allowed opacity-60",
					)}
				>
					<TeamTile
						teamId={option.id}
						name={option.name}
						imageUrl={option.imageUrl}
						size={22}
					/>
					<span className="min-w-0 flex-1 truncate">{option.name}</span>
					<span
						className={cn(
							"shrink-0 text-xs",
							index === active
								? "text-accent-foreground/80"
								: "text-muted-foreground",
						)}
					>
						{`#${option.tag}`}
					</span>
					{option.lockedLevel && (
						<span className="flex shrink-0 items-center gap-1 text-xs capitalize">
							<LockIcon className="size-3" aria-hidden />
							{t("teams.chats.leadClassified", "Lead classified {{level}}", {
								level: option.lockedLevel,
							})}
						</span>
					)}
					{option.leadName && !option.lockedLevel && (
						<span
							className={cn(
								"hidden max-w-[40%] shrink-0 truncate text-xs sm:inline",
								index === active
									? "text-accent-foreground/80"
									: "text-muted-foreground",
							)}
						>
							{t("teams.requests.picker.lead", "Lead: {{name}}", {
								name: option.leadName,
							})}
						</span>
					)}
				</button>
			))}
		</div>
	);
}

export interface TeamTagPicker {
	menu: ReactNode;
	track: (input: HTMLTextAreaElement) => void;
	handleKeyDown: (event: KeyboardEvent<HTMLTextAreaElement>) => boolean;
	handleKeyUp: (event: KeyboardEvent<HTMLTextAreaElement>) => void;
}

/**
 * `#` suggestions for a textarea: while the draft's leading `#tag` is being typed, lists the
 * teams whose tag starts with it, arrows move, Enter or Tab inserts `#Tag `, Escape closes.
 * Without teams it does nothing.
 */
export function useTeamTagPicker({
	teams,
	value,
	onChange,
	inputRef,
}: Readonly<{
	teams?: readonly TeamTagOption[];
	value: string;
	onChange: (value: string) => void;
	inputRef: RefObject<HTMLTextAreaElement | null>;
}>): TeamTagPicker {
	const [trigger, setTrigger] = useState<TeamTagQuery | null>(null);
	const [active, setActive] = useState(0);
	const suggestions = useMemo(
		() => (trigger && teams?.length ? matchTeamTags(teams, trigger.query) : []),
		[teams, trigger],
	);

	const pick = useCallback(
		(option: TeamTagOption) => {
			const input = inputRef.current;
			if (!trigger || !input) return;
			const next = insertTeamTag(value, trigger, option.tag);
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
				teams?.length
					? leadingTagQuery(input.value, input.selectionStart ?? 0)
					: null,
			);
			setActive(0);
		},
		[teams],
	);

	const handleKeyDown = (event: KeyboardEvent<HTMLTextAreaElement>) => {
		if (suggestions.length === 0 || event.nativeEvent.isComposing) return false;
		if (event.key === "ArrowDown" || event.key === "ArrowUp") {
			const step = event.key === "ArrowDown" ? 1 : -1;
			setActive(
				(index) => (index + step + suggestions.length) % suggestions.length,
			);
		} else if (event.key === "Enter" || event.key === "Tab") {
			const option = suggestions[active] ?? suggestions[0];
			if (!option.lockedLevel) pick(option);
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
				<TeamTagMenu options={suggestions} active={active} onPick={pick} />
			) : null,
		track,
		handleKeyDown,
		handleKeyUp,
	};
}
