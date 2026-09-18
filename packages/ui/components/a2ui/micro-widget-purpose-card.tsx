"use client";

import { useTranslation } from "@flow-like/locales";
import {
	Building2Icon,
	GlobeIcon,
	InfoIcon,
	type LucideIcon,
	RadioIcon,
	TriangleAlertIcon,
} from "lucide-react";
import { type ReactNode, useMemo, useState } from "react";
import { cn } from "../../lib/utils";
import { Badge } from "../ui/badge";
import { Button } from "../ui/button";
import {
	FLOW_MARK_BODY,
	FLOW_MARK_INSET,
	FLOW_MARK_VIEW_BOX,
} from "../ui/flow-mark-paths";
import {
	type WidgetConsentChip,
	type WidgetPurposeView,
	partitionWidgetChips,
} from "./micro-widget-consent-view";
import {
	type Translate,
	WIDGET_DIRECTIVE_LABELS,
	WIDGET_SOURCE_LEVEL_LABELS,
	widgetSourceAbout,
	widgetSourceCardRisk,
	widgetSourceCopyParams,
	widgetSourceRisk,
} from "./micro-widget-network-copy";
import type { WidgetSourceLevel } from "./micro-widget-policy";

interface LevelStyle {
	icon: LucideIcon;
	variant: "outline" | "secondary" | "destructive";
	className?: string;
}

/** Only `broad` is destructive (§14.9); the other levels stay neutral. */
const LEVEL_STYLES: Readonly<Record<WidgetSourceLevel, LevelStyle>> = {
	known: {
		icon: Building2Icon,
		variant: "outline",
		className: "text-muted-foreground",
	},
	external: { icon: GlobeIcon, variant: "secondary" },
	shared: { icon: InfoIcon, variant: "secondary" },
	broad: { icon: TriangleAlertIcon, variant: "destructive" },
};

export function WidgetSourceLevelIcon({
	level,
	className,
}: {
	level: WidgetSourceLevel;
	className?: string;
}) {
	const Icon = LEVEL_STYLES[level].icon;
	return <Icon aria-hidden="true" className={className} />;
}

export function WidgetSourceLevelBadge({
	level,
	className,
}: {
	level: WidgetSourceLevel;
	className?: string;
}) {
	const { t } = useTranslation("common");
	const style = LEVEL_STYLES[level];
	return (
		<Badge
			variant={style.variant}
			className={cn(style.className, className)}
			data-widget-level-badge={level}
		>
			<WidgetSourceLevelIcon level={level} />
			{WIDGET_SOURCE_LEVEL_LABELS[level](t)}
		</Badge>
	);
}

function FlowLikeMark({ className }: { className?: string }) {
	return (
		<svg
			viewBox={FLOW_MARK_VIEW_BOX}
			aria-hidden="true"
			focusable="false"
			className={cn("size-3 shrink-0 fill-current", className)}
		>
			<path d={FLOW_MARK_BODY} />
			<path d={FLOW_MARK_INSET} className="opacity-60" />
		</svg>
	);
}

/** Flow-Like's own voice: the mark, then a screen-reader label. */
export function WidgetHostCopy({ children }: { children: ReactNode }) {
	const { t } = useTranslation("common");
	return (
		<p className="flex items-start gap-1.5 text-xs" data-widget-host-copy>
			<FlowLikeMark className="mt-0.5 text-muted-foreground" />
			<span className="min-w-0">
				<span className="sr-only">
					{t("widgetSourceHostNote", "Flow-Like:")}{" "}
				</span>
				{children}
			</span>
		</p>
	);
}

/** The publisher's reason: labelled, isolated, never translated or emphasized. */
export function WidgetPublisherNote({ reason }: { reason: string }) {
	const { t } = useTranslation("common");
	return (
		<p
			className="line-clamp-2 border-l-2 pl-2 text-xs wrap-break-word"
			data-widget-publisher-note
		>
			<span className="text-muted-foreground">
				{t("widgetNetworkPublisherLabel", "Publisher:")}
			</span>{" "}
			<bdi>{reason}</bdi>
		</p>
	);
}

function splitEmphasis(chip: WidgetConsentChip): {
	prefix: string;
	emphasis: string;
} {
	if (chip.host === chip.emphasis) return { prefix: "", emphasis: chip.host };
	return chip.host.endsWith(`.${chip.emphasis}`)
		? {
				prefix: chip.host.slice(0, chip.host.length - chip.emphasis.length),
				emphasis: chip.emphasis,
			}
		: { prefix: "", emphasis: chip.host };
}

/** A host as text: punycode, never a link, never focusable. */
export function WidgetSourceChip({ chip }: { chip: WidgetConsentChip }) {
	const { t } = useTranslation("common");
	const { prefix, emphasis } = splitEmphasis(chip);
	return (
		<span
			className={cn(
				"inline-flex max-w-full items-center gap-1 rounded-md border px-1.5 py-0.5",
				chip.runtime && "border-dashed",
				chip.level === "broad" && "border-destructive/60",
				!chip.included && "opacity-50",
			)}
			data-widget-source-chip={chip.key}
			data-widget-source-runtime={chip.runtime ? "" : undefined}
			data-widget-source-included={chip.included ? "true" : "false"}
		>
			{chip.runtime && (
				<>
					<RadioIcon
						aria-hidden="true"
						className="size-3 shrink-0 text-muted-foreground"
					/>
					<span className="sr-only">
						{t("widgetRuntimeSourcesLine", "Provided while the app runs")}
					</span>
				</>
			)}
			<bdi
				dir="ltr"
				translate="no"
				className="min-w-0 font-mono text-xs break-all"
			>
				{chip.wildcard && (
					<>
						<span aria-hidden="true" className="text-muted-foreground">
							*.
						</span>
						<span className="sr-only">
							{t("widgetSourceWildcardSr", "any address under")}{" "}
						</span>
					</>
				)}
				{prefix && <span className="text-muted-foreground">{prefix}</span>}
				<span className="font-medium whitespace-nowrap">{emphasis}</span>
			</bdi>
			{chip.platformStorage && (
				<span className="text-xs text-muted-foreground">
					{t("widgetSourceAppFiles", "this app's files")}
				</span>
			)}
		</span>
	);
}

function distinct(values: readonly string[]): string[] {
	return [...new Set(values)];
}

function cardCopy(
	card: WidgetPurposeView,
	density: WidgetPurposeCardDensity,
	t: Translate,
): string[] {
	if (density === "store") {
		return distinct(
			card.sources.flatMap((source) =>
				[widgetSourceAbout(source, t), widgetSourceRisk(source, t)].filter(
					(line): line is string => line !== null,
				),
			),
		);
	}
	return distinct(
		card.notes.map((note) => {
			switch (note.kind) {
				case "runtime":
					return t(
						"widgetRuntimeCardExplanation",
						"Not among the widget's declared addresses. It reached the widget while the app ran, possibly from the app's pages, its workflows, or nodes from this widget's publisher.",
					);
				case "service":
					return t(
						"widgetSourceRiskService",
						"Data goes to {{provider}}.",
						widgetSourceCopyParams(note.source),
					);
				case "risk":
					return widgetSourceCardRisk(note.source, t);
			}
		}),
	);
}

function ChipRow({
	chips,
	collapsible,
	trailing,
}: {
	chips: readonly WidgetConsentChip[];
	collapsible: boolean;
	trailing?: ReactNode;
}) {
	const { t } = useTranslation("common");
	const [expanded, setExpanded] = useState(false);
	const { visible, hidden } = useMemo(
		() =>
			collapsible
				? partitionWidgetChips(chips)
				: { visible: [...chips], hidden: [] },
		[chips, collapsible],
	);
	const shown = expanded ? chips : visible;
	return (
		<div className="flex flex-wrap items-center gap-1.5">
			{shown.map((chip) => (
				<WidgetSourceChip key={chip.key} chip={chip} />
			))}
			{hidden.length > 0 && (
				<Button
					type="button"
					variant="link"
					size="sm"
					className="h-auto min-h-6 px-0 text-xs"
					aria-expanded={expanded}
					onClick={() => setExpanded((open) => !open)}
				>
					{expanded
						? t("showLess", "Show less")
						: t("widgetNetworkMoreSources", "+{{count}} more", {
								count: hidden.length,
							})}
				</Button>
			)}
			{trailing}
		</div>
	);
}

export type WidgetPurposeCardDensity = "dialog" | "store";

/**
 * One purpose group (§14.5.2): Flow-Like's level and hosts first, the
 * publisher's reason last. `store` density collapses nothing and always
 * shows About and Risk.
 */
export function WidgetPurposeCard({
	card,
	density = "dialog",
}: {
	card: WidgetPurposeView;
	density?: WidgetPurposeCardDensity;
}) {
	const { t } = useTranslation("common");
	const copy = useMemo(() => cardCopy(card, density, t), [card, density, t]);
	const providers =
		card.providers.length > 0 ? (
			<span translate="no" className="text-xs text-muted-foreground">
				{card.providers.join(", ")}
			</span>
		) : null;
	return (
		<section
			className="flex flex-col gap-1.5 rounded-md border p-2.5"
			data-widget-purpose={card.index}
			data-widget-purpose-level={card.level}
		>
			<div className="flex flex-wrap items-center gap-x-2 gap-y-1">
				<WidgetSourceLevelBadge level={card.level} />
				<span className="text-xs text-muted-foreground">
					{card.directives
						.map((key) => WIDGET_DIRECTIVE_LABELS[key](t))
						.join(" · ")}
				</span>
				{card.raised ? (
					<Badge variant="outline" data-widget-source-raised>
						{t(
							"widgetSourceLevelRaised",
							"Risk increased since you allowed it",
						)}
					</Badge>
				) : card.isNew ? (
					<Badge variant="outline" data-widget-source-new>
						{t("widgetSourceNew", "New")}
					</Badge>
				) : null}
			</div>
			{card.declared.length > 0 && (
				<ChipRow
					chips={card.declared}
					collapsible={density === "dialog"}
					trailing={providers}
				/>
			)}
			{card.runtime.length > 0 && (
				<div className="flex flex-col gap-1" data-widget-runtime-sources>
					<p
						className="flex flex-wrap items-center gap-x-1 text-xs text-muted-foreground"
						data-widget-runtime-heading
					>
						<RadioIcon aria-hidden="true" className="size-3 shrink-0" />
						<span>
							{t("widgetRuntimeSourcesLine", "Provided while the app runs")}
						</span>
						{card.inputs.length > 0 && (
							<>
								<span aria-hidden="true">·</span>
								<span>
									{t("widgetRuntimeInputLabel", "Inputs", {
										count: card.inputs.length,
									})}
								</span>
								{card.inputs.map((path) => (
									<code
										key={path}
										className="rounded bg-muted px-1 font-mono text-foreground"
									>
										{path}
									</code>
								))}
							</>
						)}
					</p>
					<ChipRow
						chips={card.runtime}
						collapsible={false}
						trailing={card.declared.length === 0 ? providers : null}
					/>
				</div>
			)}
			{copy.map((line) => (
				<WidgetHostCopy key={line}>{line}</WidgetHostCopy>
			))}
			{card.reason && <WidgetPublisherNote reason={card.reason} />}
		</section>
	);
}
