"use client";

import { useTranslation } from "@flow-like/locales";
import {
	BoxIcon,
	LayersIcon,
	PlusIcon,
	VariableIcon,
	WorkflowIcon,
} from "lucide-react";
import { memo } from "react";
import {
	formatAbsoluteDateTime,
	formatRelativeTime,
} from "../../../../lib/date";
import { cn } from "../../../../lib/utils";
import { Badge } from "../../../ui/badge";
import { Button } from "../../../ui/button";
import { Skeleton } from "../../../ui/skeleton";
import { TemplateThumbnail } from "./template-thumbnail";
import { type ITemplateItem, versionLabel } from "./templates-gallery-model";

/** Dotted canvas behind every template preview. */
export const PREVIEW_SURFACE =
	"bg-muted/30 bg-[radial-gradient(var(--border)_1px,transparent_1.2px)] bg-[length:14px_14px]";

export function TemplateStats({ item }: Readonly<{ item: ITemplateItem }>) {
	const { t } = useTranslation("common");
	const digest = item.digest;
	if (!digest) {
		return item.boardState === "loading" ? (
			<Skeleton aria-hidden className="h-3.5 w-44" />
		) : (
			<span>{t("templateStructureUnavailable", "Structure unavailable")}</span>
		);
	}
	return (
		<>
			<span className="inline-flex items-center gap-1.5">
				<BoxIcon aria-hidden className="size-3.5" />
				{t("templateNodeCount", "{{count}} nodes", {
					count: digest.nodeCount,
					defaultValue_one: "{{count}} node",
				})}
			</span>
			<span className="inline-flex items-center gap-1.5">
				<LayersIcon aria-hidden className="size-3.5" />
				{t("templateLayerCount", "{{count}} layers", {
					count: digest.layerCount,
					defaultValue_one: "{{count}} layer",
				})}
			</span>
			<span className="inline-flex items-center gap-1.5">
				<VariableIcon aria-hidden className="size-3.5" />
				{t("templateVariableCount", "{{count}} variables", {
					count: digest.variableCount,
					defaultValue_one: "{{count}} variable",
				})}
			</span>
		</>
	);
}

export function TemplateSource({ item }: Readonly<{ item: ITemplateItem }>) {
	const { i18n } = useTranslation("common");
	const relative = item.updatedAtMs
		? formatRelativeTime(item.updatedAtMs, "long", "", {
				locale: i18n.language,
			})
		: "";
	return (
		<p className="flex min-w-0 items-center gap-1.5 text-xs text-muted-foreground">
			<WorkflowIcon aria-hidden className="size-3.5 shrink-0" />
			{item.sourceName ? (
				<span className="truncate font-medium text-foreground">
					{item.sourceName}
				</span>
			) : item.boardState === "loading" ? (
				<Skeleton aria-hidden className="h-3 w-24" />
			) : null}
			{relative ? (
				<>
					{item.sourceName || item.boardState === "loading" ? (
						<span aria-hidden>·</span>
					) : null}
					<time
						dateTime={new Date(item.updatedAtMs).toISOString()}
						title={formatAbsoluteDateTime(item.updatedAtMs)}
						className="shrink-0"
					>
						{relative}
					</time>
				</>
			) : null}
		</p>
	);
}

function PreviewArea({ item }: Readonly<{ item: ITemplateItem }>) {
	const { t } = useTranslation("common");
	if (
		item.board &&
		item.digest &&
		item.digest.nodeCount + item.digest.layerCount > 0
	) {
		return <TemplateThumbnail board={item.board} className="p-3" />;
	}
	if (item.boardState === "loading") {
		return (
			<Skeleton aria-hidden className="m-3 h-[calc(100%-1.5rem)] rounded-lg" />
		);
	}
	return (
		<div className="flex size-full items-center justify-center text-xs text-muted-foreground">
			{item.boardState === "error"
				? t("previewNotAvailable", "Preview not available")
				: t("templateHasNoNodes", "This template has no nodes")}
		</div>
	);
}

export const TemplateCard = memo(function TemplateCard({
	item,
	canCreateFlow,
	onOpen,
	onUse,
}: Readonly<{
	item: ITemplateItem;
	canCreateFlow: boolean;
	onOpen: (item: ITemplateItem) => void;
	onUse: (item: ITemplateItem) => void;
}>) {
	const { t } = useTranslation("common");
	const version = versionLabel(item.digest?.version);

	return (
		<article className="group relative flex min-w-0 flex-col overflow-hidden rounded-xl border border-border/60 bg-card transition-[border-color,box-shadow,translate] duration-150 hover:-translate-y-px hover:border-border hover:shadow-lg hover:shadow-foreground/5 focus-within:border-border">
			<div
				className={cn(
					"relative h-36 overflow-hidden border-b border-border/60",
					PREVIEW_SURFACE,
				)}
			>
				<PreviewArea item={item} />
				{version ? (
					<span className="absolute left-2.5 top-2.5 rounded-md border border-border/60 bg-card/90 px-1.5 py-0.5 font-mono text-[11px] text-muted-foreground">
						{version}
					</span>
				) : null}
				{canCreateFlow && item.board ? (
					<div className="absolute inset-x-0 bottom-3 z-10 flex justify-center opacity-0 transition-opacity duration-150 group-hover:opacity-100 group-focus-within:opacity-100">
						<Button
							size="sm"
							className="h-8 shadow-md"
							onClick={() => onUse(item)}
						>
							<PlusIcon aria-hidden className="size-3.5" />
							{t("newFlowFromThis", "New flow from this")}
						</Button>
					</div>
				) : null}
			</div>
			<div className="flex flex-1 flex-col gap-2 p-4">
				<h3 className="text-[15px] font-semibold leading-snug">
					<button
						type="button"
						onClick={() => onOpen(item)}
						title={item.name}
						className="text-left outline-none after:absolute after:inset-0 after:z-1 after:rounded-xl focus-visible:after:ring-2 focus-visible:after:ring-ring"
					>
						<span className="line-clamp-2 wrap-break-word">{item.name}</span>
					</button>
				</h3>
				{item.description ? (
					<p className="line-clamp-2 text-[13px] text-muted-foreground">
						{item.description}
					</p>
				) : null}
				<TemplateSource item={item} />
				{item.tags.length > 0 ? (
					<div className="flex flex-wrap gap-1">
						{item.tags.map((tag) => (
							<Badge key={tag} variant="secondary" className="text-[11px]">
								{tag}
							</Badge>
						))}
					</div>
				) : null}
				<div className="mt-auto flex flex-wrap gap-x-3.5 gap-y-1 border-t border-border/60 pt-3 text-xs text-muted-foreground">
					<TemplateStats item={item} />
				</div>
			</div>
		</article>
	);
});

export function TemplateCardSkeleton() {
	return (
		<div
			aria-hidden
			className="flex flex-col overflow-hidden rounded-xl border border-border/60 bg-card"
		>
			<Skeleton className="h-36 rounded-none" />
			<div className="flex flex-col gap-2.5 p-4">
				<Skeleton className="h-4 w-3/4" />
				<Skeleton className="h-3 w-1/2" />
				<Skeleton className="mt-3 h-3.5 w-44" />
			</div>
		</div>
	);
}
