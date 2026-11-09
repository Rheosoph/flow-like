"use client";

import { useTranslation } from "@flow-like/locales";
import {
	ArrowUpRightIcon,
	ChevronDownIcon,
	GlobeIcon,
	Loader2Icon,
	SparklesIcon,
	Trash2Icon,
	UserRoundIcon,
} from "lucide-react";
import { useMemo, useState } from "react";
import { formatRelativeTime } from "../../lib/date";
import { cn } from "../../lib/utils";
import type {
	BotFact,
	BotFactEvidence,
} from "../../state/global-chat/bot-memory-facts";
import { Button } from "../ui/button";
import { MemoryFacets, memoryFacets } from "./bot-memory-facets";
import {
	memoryKindStyle,
	memoryLabel,
	useMemoryKindLabel,
} from "./bot-memory-kinds";
import { MemoryMapFrame } from "./bot-memory-map-frame";
import { type MemoryFormat, parseInstant } from "./bot-memory-model";

const sourceIcons = {
	user_stated: UserRoundIcon,
	bot_inferred: SparklesIcon,
	tool_untrusted: GlobeIcon,
};

function sourceIcon(source: string) {
	return Object.hasOwn(sourceIcons, source)
		? sourceIcons[source as keyof typeof sourceIcons]
		: SparklesIcon;
}

function Evidence({
	evidence,
	group,
	onJump,
}: Readonly<{
	evidence: BotFactEvidence;
	group: boolean;
	onJump: (seq: number) => void;
}>) {
	const { t } = useTranslation("bots");
	return (
		<figure className="min-w-0">
			{evidence.quote && (
				<blockquote className="relative whitespace-pre-wrap wrap-anywhere pl-3 text-xs leading-relaxed text-muted-foreground before:absolute before:inset-y-0.5 before:left-0 before:w-0.5 before:rounded-full before:bg-border">
					{evidence.quote}
				</blockquote>
			)}
			<figcaption className="mt-1 flex flex-wrap items-center gap-x-1.5 gap-y-1 pl-3 text-[11px] text-muted-foreground">
				{evidence.speaker?.name && (
					<>
						<span className="min-w-0 wrap-anywhere font-medium text-foreground/80">
							{evidence.speaker.name}
						</span>
						<span aria-hidden>·</span>
					</>
				)}
				{group ? (
					<span>
						{t("crew.invitations.groupChat", "a group chat")} · #{evidence.seq}
					</span>
				) : (
					<button
						type="button"
						onClick={() => onJump(evidence.seq)}
						aria-label={t("memory.openMessage", "Open message #{{seq}}", {
							seq: evidence.seq,
						})}
						className="-my-1 inline-flex min-h-7 items-center gap-0.5 rounded-md font-medium text-foreground/70 underline-offset-4 hover:text-primary hover:underline focus-visible:outline-2 focus-visible:outline-ring"
					>
						{t("memory.message", "Message #{{seq}}", { seq: evidence.seq })}
						<ArrowUpRightIcon className="size-3" aria-hidden />
					</button>
				)}
			</figcaption>
		</figure>
	);
}

function DetailRow({
	label,
	children,
}: Readonly<{ label: string; children: React.ReactNode }>) {
	return (
		<>
			<dt className="text-muted-foreground">{label}</dt>
			<dd className="min-w-0 wrap-anywhere">{children}</dd>
		</>
	);
}

export function BotMemoryCard({
	fact,
	now,
	format,
	busy,
	disabled,
	rail = false,
	selected = false,
	onForget,
	onJump,
	onLocate,
}: Readonly<{
	fact: BotFact;
	now: number;
	format: MemoryFormat;
	busy: boolean;
	disabled: boolean;
	/** Draws the card's dot on the timeline rail. */
	rail?: boolean;
	selected?: boolean;
	onForget: (id: string) => void;
	onJump: (seq: number) => void;
	/** Focuses the place on a map outside the card; without it the card opens its own. */
	onLocate?: (id: string) => void;
}>) {
	const { t } = useTranslation("bots");
	const kindLabel = useMemoryKindLabel();
	const [mapOpen, setMapOpen] = useState(false);
	const style = memoryKindStyle(fact.kind);
	const Icon = style.icon;
	const SourceIcon = sourceIcon(fact.source);
	const facets = useMemo(() => memoryFacets(fact, now), [fact, now]);
	const forget = t("memory.forget", "Forget this fact");
	const sources: Record<string, string> = {
		user_stated: t("memory.sources.user_stated", "Said by a person"),
		bot_inferred: t("memory.sources.bot_inferred", "Inferred by the bot"),
		tool_untrusted: t("memory.sources.tool_untrusted", "External content"),
	};
	const confidence = Number.isFinite(fact.confidence)
		? Math.round(Math.max(0, Math.min(1, fact.confidence)) * 100)
		: null;
	const recorded = parseInstant(fact.recorded_at);
	const group = Boolean(fact.conversation_scope);
	const about =
		fact.subject && fact.subject !== "user"
			? fact.subject
					.split(".")
					.map((part) => memoryLabel(part.trim()))
					.join(" › ")
			: null;
	return (
		<li
			data-memory-id={fact.id}
			className={cn("relative min-w-0 scroll-mt-4", rail && "pl-6")}
		>
			{rail && (
				<span
					aria-hidden
					className={cn(
						"absolute top-[1.4rem] left-[3px] size-[9px] rounded-full ring-[3px] ring-(--memory-canvas,var(--background))",
						style.solid,
					)}
				/>
			)}
			<article
				className={cn(
					"overflow-hidden rounded-2xl border bg-card shadow-xs transition-[border-color,box-shadow] duration-200",
					selected
						? "border-primary/45 shadow-sm ring-2 ring-primary/15"
						: "border-border/65 hover:border-border hover:shadow-sm",
				)}
			>
				<div
					className={cn(
						"bg-linear-to-b to-transparent to-70% p-4 pb-3.5",
						style.wash,
					)}
				>
					<div className="mb-2.5 flex items-center gap-2">
						<span
							className={cn(
								"inline-flex min-w-0 items-center gap-1.5 rounded-lg px-2 py-1 text-[11px] font-medium",
								style.color,
							)}
						>
							<Icon className="size-3.5 shrink-0" aria-hidden />
							<span className="truncate">{kindLabel(fact.kind)}</span>
						</span>
						{recorded !== null && (
							<time
								dateTime={fact.recorded_at}
								title={`${t("memory.recorded", "Saved")} ${format.full(recorded)}`}
								className="ml-auto shrink-0 text-[11px] text-muted-foreground"
							>
								{formatRelativeTime(recorded, "short", "", {
									now,
									locale: format.locale,
								})}
							</time>
						)}
						<Button
							type="button"
							variant="ghost"
							size="icon"
							disabled={disabled}
							onClick={() => onForget(fact.id)}
							aria-label={forget}
							title={forget}
							className={cn(
								"-mr-1.5 size-8 shrink-0 rounded-lg text-muted-foreground/70 hover:bg-destructive/10 hover:text-destructive",
								recorded === null && "ml-auto",
							)}
						>
							{busy ? (
								<Loader2Icon className="size-3.5 animate-spin" aria-hidden />
							) : (
								<Trash2Icon className="size-3.5" aria-hidden />
							)}
						</Button>
					</div>
					<p className="whitespace-pre-wrap wrap-anywhere text-pretty text-[0.9375rem] font-medium leading-snug tracking-[-0.006em] text-foreground">
						{fact.text}
					</p>
					<MemoryFacets
						facets={facets}
						now={now}
						format={format}
						style={style}
						placeAction={
							onLocate
								? { onClick: () => onLocate(fact.id) }
								: {
										onClick: () => setMapOpen((open) => !open),
										expanded: mapOpen,
									}
						}
					>
						{mapOpen && !onLocate && (
							<MemoryMapFrame
								facts={[fact]}
								className="h-44 border-t border-border/50"
							/>
						)}
					</MemoryFacets>
					{fact.evidence[0] && (
						<div className="mt-3">
							<Evidence
								evidence={fact.evidence[0]}
								group={group}
								onJump={onJump}
							/>
						</div>
					)}
				</div>
				<details className="group/details border-t border-border/45">
					<summary className="flex min-h-10 cursor-pointer list-none items-center justify-between gap-2 px-4 text-[11px] text-muted-foreground transition-colors hover:bg-muted/35 hover:text-foreground focus-visible:outline-2 focus-visible:-outline-offset-2 focus-visible:outline-ring [&::-webkit-details-marker]:hidden">
						<span className="inline-flex min-w-0 items-center gap-1.5">
							<SourceIcon className="size-3 shrink-0" aria-hidden />
							<span className="truncate">
								{t("memory.details", "Memory details")}
							</span>
						</span>
						<span className="flex shrink-0 items-center gap-2">
							{fact.evidence.length > 1 && (
								<span>
									{t("memory.sourceCount", "{{count}} sources", {
										count: fact.evidence.length,
									})}
								</span>
							)}
							<ChevronDownIcon
								className="size-3.5 transition-transform group-open/details:rotate-180 motion-reduce:transition-none"
								aria-hidden
							/>
						</span>
					</summary>
					<div className="space-y-3 px-4 pb-4">
						<dl className="grid grid-cols-[auto_minmax(0,1fr)] gap-x-4 gap-y-2 rounded-xl bg-muted/35 px-3 py-2.5 text-xs">
							<DetailRow label={t("memory.source", "Source")}>
								{Object.hasOwn(sources, fact.source)
									? sources[fact.source]
									: memoryLabel(fact.source)}
							</DetailRow>
							{about && (
								<DetailRow label={t("memory.about", "About")}>
									{about}
								</DetailRow>
							)}
							{recorded !== null && (
								<DetailRow label={t("memory.recorded", "Saved")}>
									<time dateTime={fact.recorded_at}>
										{format.full(recorded)}
									</time>
								</DetailRow>
							)}
							{confidence !== null && (
								<DetailRow label={t("memory.confidence", "Bot confidence")}>
									<span className="flex items-center gap-2 tabular-nums">
										<span
											aria-hidden
											className="h-1.5 w-16 overflow-hidden rounded-full bg-muted"
										>
											<span
												className={cn("block h-full rounded-full", style.solid)}
												style={{ width: `${confidence}%` }}
											/>
										</span>
										{confidence}%
									</span>
								</DetailRow>
							)}
							{facets.validity?.from != null && (
								<DetailRow label={t("memory.validFrom", "Valid from")}>
									{format.full(facets.validity.from)}
								</DetailRow>
							)}
							{facets.validity?.to != null && (
								<DetailRow label={t("memory.validTo", "Valid until")}>
									{format.full(facets.validity.to)}
								</DetailRow>
							)}
							{facets.ttl && (
								<DetailRow label={t("memory.expires", "Expires")}>
									{format.full(facets.ttl.expiresAt)}
								</DetailRow>
							)}
						</dl>
						{fact.evidence.slice(1).map((evidence) => (
							<Evidence
								key={`${evidence.message_id}-${evidence.seq}`}
								evidence={evidence}
								group={group}
								onJump={onJump}
							/>
						))}
					</div>
				</details>
			</article>
		</li>
	);
}
