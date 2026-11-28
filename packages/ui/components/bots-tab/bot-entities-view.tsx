"use client";

import { useTranslation } from "@flow-like/locales";
import { SearchXIcon, UsersIcon } from "lucide-react";
import { useMemo, useState } from "react";
import { cn } from "../../lib/utils";
import type {
	BotEntity,
	BotFact,
	BotFactsOwner,
} from "../../state/global-chat/bot-memory-facts";
import { BotEntityDetail, type EntityFactActions } from "./bot-entity-detail";
import {
	type EntityGroup,
	entityAliases,
	entityKey,
	entityLabel,
	groupEntities,
} from "./bot-memory-entities-model";
import { entityStyle, useEntityKindLabel } from "./bot-memory-entity-kinds";

function EntityRow({
	entity,
	selected,
	onSelect,
}: Readonly<{
	entity: BotEntity;
	selected: boolean;
	onSelect: () => void;
}>) {
	const { t } = useTranslation("bots");
	const style = entityStyle(entity);
	const Icon = style.icon;
	const you = t("memory.entities.you", "You");
	const aliases = entityAliases(entity, you);
	return (
		<li>
			<button
				type="button"
				aria-pressed={selected}
				onClick={onSelect}
				className={cn(
					"flex w-full min-w-0 items-center gap-3 rounded-xl border px-2.5 py-2 text-left transition-colors focus-visible:outline-2 focus-visible:outline-ring",
					selected
						? "border-primary/45 bg-card shadow-xs ring-2 ring-primary/15"
						: "border-transparent hover:bg-card/70",
				)}
			>
				<span
					className={cn(
						"flex size-8 shrink-0 items-center justify-center rounded-xl",
						style.color,
					)}
				>
					<Icon className="size-4" aria-hidden />
				</span>
				<span className="min-w-0 flex-1">
					<span className="flex items-center gap-1.5">
						<span className="truncate text-sm font-medium">
							{entityLabel(entity, you)}
						</span>
						{entity.conversation_scope && (
							<UsersIcon
								className="size-3 shrink-0 text-muted-foreground"
								aria-label={t("memory.entities.room", "Group chat")}
							/>
						)}
					</span>
					{aliases.length > 0 && (
						<span className="block truncate text-[11px] text-muted-foreground">
							{t("memory.entities.also", "also: {{names}}", {
								names: aliases.join(", "),
							})}
						</span>
					)}
				</span>
				<span
					className="shrink-0 rounded-full bg-muted px-2 py-0.5 text-[11px] tabular-nums text-muted-foreground"
					aria-label={t("memory.entities.factCount", "{{count}} facts", {
						count: entity.fact_count,
						defaultValue_one: "{{count}} fact",
					})}
				>
					{entity.fact_count}
				</span>
			</button>
		</li>
	);
}

function EntityGroups({
	groups,
	selectedKey,
	onSelect,
}: Readonly<{
	groups: EntityGroup[];
	selectedKey: string | null;
	onSelect: (entity: BotEntity) => void;
}>) {
	const kindLabel = useEntityKindLabel();
	return (
		<div className="flex flex-col gap-3">
			{groups.map((group) => (
				<section key={group.key} aria-label={kindLabel(group.key)}>
					{group.key !== "owner" && (
						<h4 className="mb-1 px-2.5 text-[11px] font-semibold text-muted-foreground">
							{kindLabel(group.key)}
							<span className="ml-1.5 font-normal tabular-nums">
								{group.entities.length}
							</span>
						</h4>
					)}
					<ul className="flex flex-col gap-0.5">
						{group.entities.map((entity) => (
							<EntityRow
								key={entityKey(entity)}
								entity={entity}
								selected={entityKey(entity) === selectedKey}
								onSelect={() => onSelect(entity)}
							/>
						))}
					</ul>
				</section>
			))}
		</div>
	);
}

/** The people, places and things the facts are about, with the selected one's connections and facts. */
export function BotEntitiesView({
	owner,
	entities,
	facts,
	query,
	actions,
	onChanged,
}: Readonly<{
	owner: BotFactsOwner;
	entities: BotEntity[];
	facts: BotFact[];
	query: string;
	actions: EntityFactActions;
	onChanged: () => void;
}>) {
	const { t } = useTranslation("bots");
	const [pickedKey, setPickedKey] = useState<string | null>(null);
	const you = t("memory.entities.you", "You");
	const groups = useMemo(
		() => groupEntities(entities, query, you),
		[entities, query, you],
	);
	const selected =
		entities.find((entity) => entityKey(entity) === pickedKey) ?? null;

	if (groups.length === 0) {
		return (
			<div className="mt-3 flex flex-col items-center gap-3 rounded-2xl border border-dashed border-border bg-card/50 px-5 py-8 text-center">
				<SearchXIcon className="size-6 text-muted-foreground" aria-hidden />
				<p className="text-sm font-medium">
					{t("memory.entities.noResults", "Nobody matches")}
				</p>
			</div>
		);
	}

	return (
		<div className="@container/entities pt-3">
			<div className="grid gap-4 @2xl/entities:grid-cols-[minmax(0,16rem)_minmax(0,1fr)] @2xl/entities:items-start">
				<div className={cn(selected && "hidden @2xl/entities:block")}>
					<EntityGroups
						groups={groups}
						selectedKey={pickedKey}
						onSelect={(entity) => setPickedKey(entityKey(entity))}
					/>
				</div>
				{selected ? (
					<BotEntityDetail
						owner={owner}
						entity={selected}
						entities={entities}
						facts={facts}
						actions={actions}
						onSelect={(entity) => setPickedKey(entityKey(entity))}
						onBack={() => setPickedKey(null)}
						onChanged={(next) => {
							if (next !== undefined) setPickedKey(next || null);
							onChanged();
						}}
					/>
				) : (
					<p className="hidden rounded-2xl border border-dashed border-border bg-card/50 px-5 py-10 text-center text-xs text-muted-foreground @2xl/entities:block">
						{t(
							"memory.entities.pick",
							"Pick someone or something to see how it connects.",
						)}
					</p>
				)}
			</div>
		</div>
	);
}
