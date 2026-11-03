"use client";

import { useTranslation } from "@flow-like/locales";
import { CheckIcon, Loader2Icon, SearchIcon } from "lucide-react";
import { useMemo, useState } from "react";
import { useBotLabels } from "../../hooks/use-bots";
import { useProjectUserSearch } from "../../hooks/use-project-user-search";
import { userDisplayName, userSecondaryLabel } from "../../lib/user-display";
import { cn } from "../../lib/utils";
import type { IBot } from "../../state/backend-state/bot-state";
import { IdentityAvatar } from "../identity/identity-avatar";
import { LocalBadge } from "./bot-labels";
import { isSelfPrincipal, matchesSearch } from "./crew-model";
import type { CrewDirectory } from "./use-crew-directory";

export interface PickedPrincipal {
	id: string;
	kind: "bot" | "user";
	label: string;
	bot?: IBot;
}

interface PickerRow extends PickedPrincipal {
	detail?: string;
}

function PickerOption({
	row,
	selected,
	multiple,
	disabled,
	onPick,
}: Readonly<{
	row: PickerRow;
	selected: boolean;
	multiple: boolean;
	disabled: boolean;
	onPick: (row: PickerRow) => void;
}>) {
	return (
		<li>
			<button
				type="button"
				aria-pressed={multiple ? selected : undefined}
				disabled={disabled && !selected}
				onClick={() => onPick(row)}
				className="flex w-full items-center gap-3 rounded-lg px-2 py-1.5 text-left outline-none hover:bg-muted focus-visible:ring-2 focus-visible:ring-ring disabled:cursor-not-allowed disabled:opacity-50"
			>
				<IdentityAvatar subject={row.id} size={32} />
				<span className="flex min-w-0 flex-1 flex-col">
					<span className="flex min-w-0 items-center gap-1.5">
						<span className="truncate text-sm font-medium">{row.label}</span>
						{row.bot?.local && <LocalBadge className="shrink-0" />}
					</span>
					{row.detail && (
						<span className="truncate text-xs text-muted-foreground">
							{row.detail}
						</span>
					)}
				</span>
				{multiple && (
					<span
						aria-hidden
						className={cn(
							"flex size-5 shrink-0 items-center justify-center rounded-full border",
							selected && "border-primary bg-primary text-primary-foreground",
						)}
					>
						{selected && <CheckIcon className="size-3.5" />}
					</span>
				)}
			</button>
		</li>
	);
}

function PickerGroup({
	label,
	rows,
	...option
}: Readonly<{
	label: string;
	rows: readonly PickerRow[];
	selected: ReadonlySet<string>;
	multiple: boolean;
	full: boolean;
	onPick: (row: PickerRow) => void;
}>) {
	if (rows.length === 0) return null;
	return (
		<div className="flex flex-col gap-1">
			<span className="px-2 text-xs font-semibold uppercase tracking-wide text-muted-foreground">
				{label}
			</span>
			<ul className="flex flex-col">
				{rows.map((row) => (
					<PickerOption
						key={row.id}
						row={row}
						selected={option.selected.has(row.id)}
						multiple={option.multiple}
						disabled={option.full}
						onPick={option.onPick}
					/>
				))}
			</ul>
		</div>
	);
}

/** Who can be picked: your bots and the people you already share conversations with. */
export interface PickerSource {
	me?: string | null;
	bots: readonly IBot[];
	recentPeopleIds: readonly string[];
	directory: CrewDirectory;
	/** People need an account; without one only your bots on this computer can be picked. */
	people: boolean;
}

/**
 * Your bots, people you already talk to, and anyone the directory finds. People must share an
 * app with you to be added; the server says so when they don't.
 */
export function PrincipalPicker({
	source,
	selected,
	multiple,
	full = false,
	onPick,
}: Readonly<{
	source: PickerSource;
	selected: ReadonlySet<string>;
	multiple: boolean;
	full?: boolean;
	onPick: (principal: PickedPrincipal) => void;
}>) {
	const { t } = useTranslation("bots");
	const botLabels = useBotLabels();
	const { me, bots, recentPeopleIds, directory, people } = source;
	const [query, setQuery] = useState("");
	const search = useProjectUserSearch(undefined, query, people);

	const botRows = useMemo<PickerRow[]>(
		() =>
			bots
				.map((bot) => {
					const labels = botLabels(bot);
					return {
						id: bot.id,
						kind: "bot" as const,
						label: labels.name,
						detail: labels.description,
						bot,
					};
				})
				.filter((row) => matchesSearch(query, [row.label, row.detail])),
		[bots, botLabels, query],
	);

	const peopleRows = useMemo<PickerRow[]>(() => {
		if (!people) return [];
		const recent = recentPeopleIds
			.filter((id) => !isSelfPrincipal(id, me))
			.map((id) => ({
				id,
				kind: "user" as const,
				label: directory(id).name,
			}))
			.filter((row) => matchesSearch(query, [row.label]));
		const known = new Set(recent.map((row) => row.id));
		const found = search.results
			.filter(
				({ user }) =>
					!known.has(user.id) && !user.bot && !isSelfPrincipal(user.id, me),
			)
			.map(({ user }) => ({
				id: user.id,
				kind: "user" as const,
				label: userDisplayName(user),
				detail: userSecondaryLabel(user),
			}));
		return [...recent, ...found];
	}, [directory, me, people, query, recentPeopleIds, search.results]);

	const nothing =
		botRows.length === 0 &&
		peopleRows.length === 0 &&
		!search.isSearchingDirectory;
	const searchLabel = people
		? t("crew.picker.searchLabel", "Search your bots and people")
		: t("local.picker.searchLabel", "Search your bots");

	return (
		<div className="flex min-h-0 flex-col gap-3">
			<label className="flex h-9 items-center gap-2 rounded-lg border bg-card px-2.5 text-muted-foreground focus-within:border-primary">
				<SearchIcon className="size-4 shrink-0" aria-hidden />
				<input
					type="search"
					value={query}
					onChange={(event) => setQuery(event.target.value)}
					aria-label={searchLabel}
					placeholder={searchLabel}
					className="h-8 min-w-0 flex-1 appearance-none bg-transparent text-sm text-foreground outline-none placeholder:text-muted-foreground"
				/>
				{search.isSearchingDirectory && (
					<Loader2Icon className="size-4 shrink-0 animate-spin" aria-hidden />
				)}
			</label>
			<div className="flex max-h-80 min-h-0 flex-col gap-3 overflow-y-auto pr-1">
				<PickerGroup
					label={t("crew.picker.bots", "Your bots")}
					rows={botRows}
					selected={selected}
					multiple={multiple}
					full={full}
					onPick={onPick}
				/>
				<PickerGroup
					label={t("crew.picker.people", "People")}
					rows={peopleRows}
					selected={selected}
					multiple={multiple}
					full={full}
					onPick={onPick}
				/>
				{nothing && people && (
					<p className="px-2 py-4 text-center text-sm text-muted-foreground">
						{search.canSearchDirectory
							? t("crew.picker.noMatches", "No bots or people match.")
							: t(
									"crew.picker.hint",
									"Type a name or email to find people who share an app with you.",
								)}
					</p>
				)}
				{!people && (
					<p className="px-2 py-2 text-center text-xs text-muted-foreground">
						{t(
							"local.picker.hint",
							"Signed out, you can talk with your bots on this computer. Sign in to talk with people.",
						)}
					</p>
				)}
				{search.directoryError && (
					<p className="px-2 text-xs text-destructive">
						{t("crew.picker.searchFailed", "Searching people failed.")}
					</p>
				)}
			</div>
		</div>
	);
}
