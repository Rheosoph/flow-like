"use client";

import { useTranslation } from "@flow-like/locales";
import {
	CircleAlert,
	CircleCheck,
	Ellipsis,
	Info,
	type LucideIcon,
	OctagonX,
	TriangleAlert,
} from "lucide-react";
import type { ReactNode } from "react";
import {
	DropdownMenu,
	DropdownMenuContent,
	DropdownMenuItem,
	DropdownMenuTrigger,
} from "../../../ui/dropdown-menu";
import { type DevicesT, useAreaPrefs, useAreaTime } from "./area-context";
import { DvButton } from "./dv-button";
import {
	FreshnessStamp,
	type FreshnessStampProps,
	type StampSpec,
	baseSource,
	sameSource,
} from "./freshness-stamp";
import { type Gate, GatedAction } from "./gate-notice";
import { SEVERITY_ICON, SEVERITY_TONE, type SeverityKind } from "./icons";
import { severityLabel } from "./severity-word";
import { TONE_TEXT, cx } from "./tone";

export interface AttentionAction {
	label: string;
	icon?: LucideIcon;
	onSelect?: () => void;
	/** Renders the action as a link. */
	href?: string;
	gate?: Gate | null;
}

/** One open item, already translated (the copy layer builds `sentence`). */
export interface AttentionEntry {
	id: string;
	severity: SeverityKind;
	sentence: ReactNode;
	/** The IA §6.5 condition key; shown only with technical keys on (R3). */
	conditionKey?: string;
	stamp?: FreshnessStampProps;
	/** Unix seconds the condition started. */
	since?: number;
	action?: AttentionAction;
	/** Notice items: the overflow menu's "Snooze for 7 days". */
	onSnooze?: () => void;
}

export interface AttentionDone {
	id: string;
	/** The result sentence ("Backed up to your account as version 1 at 14:02."). */
	sentence: ReactNode;
	/** Unix seconds. */
	doneAt: number;
	conditionKey?: string;
	onDismiss?: () => void;
}

export type AttentionTierId = "now" | "soon" | "later" | "info" | "done";

const TIER_OF: Record<SeverityKind, Exclude<AttentionTierId, "done">> = {
	critical: "now",
	warning: "soon",
	notice: "later",
	info: "info",
};

const TIER_ICON: Record<AttentionTierId, { icon: LucideIcon; tone: string }> = {
	now: { icon: OctagonX, tone: "text-critical" },
	soon: { icon: TriangleAlert, tone: "text-warning" },
	later: { icon: CircleAlert, tone: "text-info" },
	info: { icon: Info, tone: "text-unknown" },
	done: { icon: CircleCheck, tone: "text-good" },
};

const SEVERITY_ORDER: readonly SeverityKind[] = [
	"critical",
	"warning",
	"notice",
	"info",
];

const tierCopy = (t: DevicesT, tier: AttentionTierId) =>
	({
		now: {
			label: t("devices:view.attention.tierNow", "Broken now"),
			note: t(
				"devices:view.attention.tierNowNote",
				"Can't be dismissed. Resolve to clear.",
			),
		},
		soon: {
			label: t("devices:view.attention.tierSoon", "Needs you soon"),
			note: t(
				"devices:view.attention.tierSoonNote",
				"Will break or get worse without action.",
			),
		},
		later: {
			label: t("devices:view.attention.tierLater", "When you have a moment"),
			note: t(
				"devices:view.attention.tierLaterNote",
				"You can snooze these for 7 days on this computer.",
			),
		},
		info: {
			label: t("devices:view.attention.tierInfo", "For your information"),
			note: t(
				"devices:view.attention.tierInfoNote",
				"Not counted. Shown here for context.",
			),
		},
		done: {
			label: t("devices:view.attention.tierDone", "Done in this session"),
			note: t(
				"devices:view.attention.tierDoneNote",
				"Results stay until you dismiss them.",
			),
		},
	})[tier];

/*
 * Below 560 px of list width (phone, the header popover, a side column) the
 * action moves under the sentence: an `auto` action column with a gate reason
 * would leave the sentence a few characters per line.
 */
const ITEM =
	"grid grid-cols-[18px_minmax(0,1fr)_auto] items-start gap-x-3 gap-y-1 border-t border-hairline first:border-t-0 @max-[560px]/attn:grid-cols-[18px_minmax(0,1fr)]";
const ITEM_ACTIONS =
	"flex flex-nowrap items-start justify-end gap-1.5 @max-[560px]/attn:col-start-2 @max-[560px]/attn:mt-1 @max-[560px]/attn:justify-start";
const META =
	"flex flex-wrap items-center gap-x-2.5 gap-y-1 text-xs text-muted-foreground";

const itemPad = (compact: boolean) => (compact ? "px-3 py-2" : "px-4 py-3");
const sentenceSize = (compact: boolean) =>
	compact ? "text-ui" : "text-sm leading-5";

interface TechKeyProps {
	value?: string;
}

function TechKey({ value }: Readonly<TechKeyProps>) {
	const { showTechnicalKeys } = useAreaPrefs();
	if (!showTechnicalKeys || !value) return null;
	return (
		<span data-tech="" className="font-mono text-[11px] text-muted-foreground">
			{value}
		</span>
	);
}

interface ActionControlProps {
	action: AttentionAction;
}

function ActionControl({ action }: Readonly<ActionControlProps>) {
	const control = action.href ? (
		<DvButton size="sm" icon={action.icon} asChild>
			<a href={action.href}>{action.label}</a>
		</DvButton>
	) : (
		<DvButton size="sm" icon={action.icon} onClick={action.onSelect}>
			{action.label}
		</DvButton>
	);
	return (
		<GatedAction
			gate={action.gate}
			className="items-end text-right @max-[560px]/attn:items-start @max-[560px]/attn:text-left *:data-gate-inline:max-w-[30ch]"
		>
			{control}
		</GatedAction>
	);
}

interface SnoozeMenuProps {
	onSnooze: () => void;
}

function SnoozeMenu({ onSnooze }: Readonly<SnoozeMenuProps>) {
	const { t } = useTranslation("devices");
	return (
		<DropdownMenu modal={false}>
			<DropdownMenuTrigger asChild>
				<DvButton
					size="sm"
					variant="ghost"
					iconOnly
					icon={Ellipsis}
					aria-label={t("view.attention.more", "More for this item")}
				/>
			</DropdownMenuTrigger>
			<DropdownMenuContent align="end">
				<DropdownMenuItem onSelect={onSnooze}>
					{t("view.attention.snooze", "Snooze for 7 days")}
				</DropdownMenuItem>
			</DropdownMenuContent>
		</DropdownMenu>
	);
}

export interface AttentionItemProps {
	item: AttentionEntry;
	compact?: boolean;
	/** Inside a tier the severity word is spoken, not shown (the tier says it). */
	inTier?: boolean;
	/** The list's shared stamp: the item repeats its stamp only when it differs (R5). */
	base?: StampSpec | null;
}

function ItemMeta({
	item,
	compact,
	inTier,
	base,
}: Readonly<AttentionItemProps>) {
	const { t } = useTranslation("devices");
	const time = useAreaTime();
	const tone = TONE_TEXT[SEVERITY_TONE[item.severity]];
	const stamp = item.stamp && !sameSource(item.stamp, base) ? item.stamp : null;
	const visible = !inTier || stamp || item.since !== undefined;
	return (
		<p className={cx(META, visible ? "mt-1.5" : "m-0")}>
			<span className={cx("font-medium", tone, inTier && "sr-only")}>
				{severityLabel(t, item.severity)}
			</span>
			{stamp ? (
				<FreshnessStamp {...stamp} compact={compact || stamp.compact} />
			) : null}
			{item.since === undefined ? null : (
				<span>
					{t("view.attention.since", "Since {{time}}", {
						time: time.at(item.since),
					})}
				</span>
			)}
			<TechKey value={item.conditionKey} />
		</p>
	);
}

/** SPEC §4.11: one open item with its primary action (never coral) and gate reason. */
export function AttentionItem(props: Readonly<AttentionItemProps>) {
	const { item } = props;
	const compact = props.compact ?? false;
	const Icon = SEVERITY_ICON[item.severity];
	return (
		<li
			data-attention={item.id}
			data-sev={item.severity}
			className={cx(ITEM, itemPad(compact))}
		>
			<span className={cx("pt-0.5", TONE_TEXT[SEVERITY_TONE[item.severity]])}>
				<Icon aria-hidden className="size-4" />
			</span>
			<div className="min-w-0">
				<p className={cx("max-w-[72ch]", sentenceSize(compact))}>
					{item.sentence}
				</p>
				<ItemMeta {...props} compact={compact} />
			</div>
			<div className={cx("-mt-1", ITEM_ACTIONS)}>
				{item.action ? <ActionControl action={item.action} /> : null}
				{item.severity === "notice" && item.onSnooze ? (
					<SnoozeMenu onSnooze={item.onSnooze} />
				) : null}
			</div>
		</li>
	);
}

export interface AttentionDoneItemProps {
	item: AttentionDone;
	compact?: boolean;
}

export function AttentionDoneItem({
	item,
	compact = false,
}: Readonly<AttentionDoneItemProps>) {
	const { t } = useTranslation("devices");
	const time = useAreaTime();
	return (
		<li
			data-attention={item.id}
			data-done=""
			className={cx(ITEM, itemPad(compact))}
		>
			<span className="pt-0.5 text-good">
				<CircleCheck aria-hidden className="size-4" />
			</span>
			<div className="min-w-0">
				<p className={cx("max-w-[72ch] text-ink-2", sentenceSize(compact))}>
					{item.sentence}
				</p>
				<p className={cx(META, "mt-1.5")}>
					<span>
						{t("view.attention.doneAt", "Done at {{time}}", {
							time: time.at(item.doneAt),
						})}
					</span>
					<TechKey value={item.conditionKey} />
				</p>
			</div>
			<div className={ITEM_ACTIONS}>
				{item.onDismiss ? (
					<DvButton variant="link" size="sm" onClick={item.onDismiss}>
						{t("view.attention.dismiss", "Dismiss")}
					</DvButton>
				) : null}
			</div>
		</li>
	);
}

export interface AttentionTierProps {
	tier: AttentionTierId;
	/** Every item of the tier, shown or not. */
	count: number;
	compact?: boolean;
	children: ReactNode;
}

/** A severity tier: icon, label, full count and (not compact) its rule. */
export function AttentionTier({
	tier,
	count,
	compact = false,
	children,
}: Readonly<AttentionTierProps>) {
	const { t } = useTranslation("devices");
	const copy = tierCopy(t, tier);
	const { icon: Icon, tone } = TIER_ICON[tier];
	return (
		<section
			data-tier={tier}
			aria-label={copy.label}
			className="border-t border-hairline first:border-t-0"
		>
			<h3
				className={cx(
					"flex flex-wrap items-center gap-1.5 text-label font-semibold tracking-[0.06em] text-ink-2 uppercase",
					compact ? "px-3 pt-2 pb-0.5" : "px-4 pt-4 pb-1",
				)}
			>
				<Icon aria-hidden className={cx("size-3.5", tone)} />
				<span>{copy.label}</span>
				<span className="font-mono text-xs font-medium tracking-normal text-muted-foreground tabular-nums">
					{count}
				</span>
				{compact ? null : (
					<span className="ml-1 text-xs font-normal tracking-normal text-muted-foreground normal-case">
						{copy.note}
					</span>
				)}
			</h3>
			<ul className="@container/attn flex flex-col">{children}</ul>
		</section>
	);
}

interface FilledTier {
	tier: Exclude<AttentionTierId, "done">;
	all: AttentionEntry[];
	shown: AttentionEntry[];
}

/** Fills tiers in severity order until `cap` rows are shown. */
export function fillTiers(
	items: readonly AttentionEntry[],
	cap = Number.POSITIVE_INFINITY,
): FilledTier[] {
	let left = cap;
	return SEVERITY_ORDER.flatMap((severity) => {
		const all = items.filter((item) => item.severity === severity);
		const shown = all.slice(0, Math.max(0, left));
		left -= shown.length;
		return shown.length ? [{ tier: TIER_OF[severity], all, shown }] : [];
	});
}

export interface AttentionListProps {
	items: readonly AttentionEntry[];
	/** Rows shown in total, filling tiers in severity order; the rest sit behind "Show all N". */
	cap?: number;
	compact?: boolean;
	/** Resolved items of this session ("Done in this session"). */
	done?: readonly AttentionDone[];
	/** The block's stamp, or "auto" for the most common item stamp (R5). */
	base?: StampSpec | "auto" | null;
	/** `false` hides the all-clear line. */
	emptyText?: ReactNode | false;
	onShowAll?: () => void;
	showAllHref?: string;
	/** Expanded past the cap: offers "Show fewer". */
	expanded?: boolean;
	onShowFewer?: () => void;
	className?: string;
}

interface AllClearProps {
	text?: ReactNode;
	compact: boolean;
}

function AllClear({ text, compact }: Readonly<AllClearProps>) {
	const { t } = useTranslation("devices");
	return (
		<p
			className={cx(
				"flex items-center gap-2 text-ink-2",
				compact ? "px-3 py-2.5" : "px-4 py-3.5",
			)}
		>
			<CircleCheck aria-hidden className="size-4 text-good" />
			<span>
				{text ?? t("view.attention.empty", "Nothing needs you right now.")}
			</span>
		</p>
	);
}

interface MoreRowProps {
	props: Readonly<AttentionListProps>;
	hidden: number;
	counted: number;
}

function MoreRow({ props, hidden, counted }: Readonly<MoreRowProps>) {
	const { t } = useTranslation("devices");
	const showAll = t("view.attention.showAll", "Show all {{count, number}}", {
		count: counted,
	});
	let control: ReactNode = null;
	if (hidden > 0 && props.showAllHref)
		control = (
			<DvButton size="sm" asChild>
				<a href={props.showAllHref}>{showAll}</a>
			</DvButton>
		);
	else if (hidden > 0 && props.onShowAll)
		control = (
			<DvButton size="sm" onClick={props.onShowAll}>
				{showAll}
			</DvButton>
		);
	else if (props.expanded && props.onShowFewer)
		control = (
			<DvButton size="sm" onClick={props.onShowFewer}>
				{t("view.attention.showFewer", "Show fewer")}
			</DvButton>
		);
	if (!control) return null;
	return (
		<div className="flex flex-wrap items-center gap-x-3 gap-y-2 border-t border-hairline px-4 py-2.5 text-xs text-muted-foreground">
			{control}
			<span>
				{t(
					"view.attention.order",
					"Ordered by severity, then services, devices, certificates, access, keys and setup.",
				)}
			</span>
		</div>
	);
}

/** SPEC §4.11: tiers by severity (Info never counted), capped, with done results kept until dismissed. */
export function AttentionList(props: Readonly<AttentionListProps>) {
	const { items, cap, done = [], className } = props;
	const compact = props.compact ?? false;
	const shared =
		props.base === "auto"
			? baseSource(items.map((item) => item.stamp))
			: (props.base ?? null);
	const counted = items.filter((item) => item.severity !== "info").length;
	const hidden = items.length - Math.min(items.length, cap ?? items.length);

	return (
		<div
			data-attention-list=""
			data-compact={compact ? "" : undefined}
			className={cx("flex min-w-0 flex-col", className)}
		>
			{counted === 0 && props.emptyText !== false ? (
				<AllClear text={props.emptyText} compact={compact} />
			) : null}
			{fillTiers(items, cap).map(({ tier, all, shown }) => (
				<AttentionTier
					key={tier}
					tier={tier}
					count={all.length}
					compact={compact}
				>
					{shown.map((item) => (
						<AttentionItem
							key={item.id}
							item={item}
							compact={compact}
							base={shared}
							inTier
						/>
					))}
				</AttentionTier>
			))}
			{done.length ? (
				<AttentionTier tier="done" count={done.length} compact={compact}>
					{done.map((item) => (
						<AttentionDoneItem key={item.id} item={item} compact={compact} />
					))}
				</AttentionTier>
			) : null}
			<MoreRow props={props} hidden={hidden} counted={counted} />
		</div>
	);
}
