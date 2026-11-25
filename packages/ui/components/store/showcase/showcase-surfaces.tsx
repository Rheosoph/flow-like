"use client";

import { useTranslation } from "@flow-like/locales";
import {
	ChevronRight,
	LayoutDashboard,
	LayoutGrid,
	LayoutTemplate,
} from "lucide-react";
import type {
	IShowcasePage,
	IShowcaseTemplate,
	IShowcaseWidget,
} from "../../../lib/schema/app/showcase";
import {
	ShowcaseSection,
	cardClass,
	interactiveCardClass,
} from "./showcase-ui";
import { PageFrame, Wireframe } from "./wireframe";

export function ShowcasePages({
	pages,
	onOpen,
}: Readonly<{ pages: IShowcasePage[]; onOpen: (id: string) => void }>) {
	const { t } = useTranslation("store");
	const previews = pages.some((p) => p.outline);
	return (
		<ShowcaseSection
			id="pages"
			icon={LayoutDashboard}
			eyebrow={t("showcasePagesEyebrow", "Pages")}
			title={t("showcasePagesTitle", "What you get to open")}
			subtitle={t("showcasePagesSubtitle", {
				defaultValue_one: "{{count}} page people open in the app.",
				defaultValue_other: "{{count}} pages people open in the app.",
				count: pages.length,
			})}
		>
			{previews ? (
				<div className="grid grid-cols-[repeat(auto-fill,minmax(min(100%,240px),1fr))] gap-x-4 gap-y-6">
					{pages.map((p) => (
						<button
							key={p.id}
							type="button"
							className="group flex min-w-0 flex-col gap-2.5 text-left focus-visible:outline-none"
							onClick={() => onOpen(p.id)}
						>
							<PageFrame
								route={p.route}
								className="w-full transition-[border-color,transform] group-hover:border-foreground/25 group-focus-visible:ring-2 group-focus-visible:ring-ring motion-safe:group-hover:-translate-y-0.5"
							>
								{p.outline ? (
									<Wireframe
										outline={p.outline}
										label={t(
											"showcasePagePreview",
											"Layout preview of {{name}}",
											{ name: p.name },
										)}
									/>
								) : (
									<span className="grid aspect-16/10 w-full place-items-center bg-background/60 text-muted-foreground/60">
										<LayoutDashboard className="h-5 w-5" aria-hidden="true" />
									</span>
								)}
							</PageFrame>
							<span className="flex min-w-0 flex-col gap-0.5 px-0.5">
								<span className="flex items-baseline justify-between gap-2">
									<b className="truncate text-sm font-medium">{p.name}</b>
									{typeof p.component_count === "number" &&
									p.component_count > 0 ? (
										<span className="shrink-0 text-xs tabular-nums text-muted-foreground">
											{t("showcaseComponentCount", {
												defaultValue_one: "{{count}} component",
												defaultValue_other: "{{count}} components",
												count: p.component_count,
											})}
										</span>
									) : null}
								</span>
								{p.description ? (
									<span className="line-clamp-2 text-xs text-muted-foreground">
										{p.description}
									</span>
								) : null}
							</span>
						</button>
					))}
				</div>
			) : (
				<div className="flex flex-col rounded-xl border border-border bg-card">
					{pages.map((p) => (
						<button
							key={p.id}
							type="button"
							className="flex w-full items-center gap-2.5 border-b border-border px-3.5 py-3 text-left first:rounded-t-xl last:rounded-b-xl last:border-b-0 hover:bg-muted/40 focus-visible:outline-none focus-visible:ring-2 focus-visible:ring-ring"
							onClick={() => onOpen(p.id)}
						>
							<LayoutDashboard
								className="h-4 w-4 text-muted-foreground"
								aria-hidden="true"
							/>
							<b className="font-medium">{p.name}</b>
							<span className="flex-1 font-mono text-xs text-muted-foreground">
								{p.route}
							</span>
							<ChevronRight
								className="h-4 w-4 text-muted-foreground"
								aria-hidden="true"
							/>
						</button>
					))}
				</div>
			)}
		</ShowcaseSection>
	);
}

export function ShowcaseWidgets({
	widgets,
	onOpen,
}: Readonly<{ widgets: IShowcaseWidget[]; onOpen: (id: string) => void }>) {
	const { t } = useTranslation("store");
	return (
		<ShowcaseSection
			id="widgets"
			icon={LayoutGrid}
			eyebrow={t("showcaseWidgetsEyebrow", "Home widgets")}
			title={t("showcaseWidgetsTitle", "Bring it to your Home")}
			subtitle={t(
				"showcaseWidgetsSubtitle",
				"Members can pin these to their Home screen.",
			)}
		>
			<div className="grid grid-cols-[repeat(auto-fill,minmax(min(100%,260px),1fr))] gap-3">
				{widgets.map((w) => (
					<button
						key={w.id}
						type="button"
						className={`${interactiveCardClass} flex flex-col gap-3`}
						onClick={() => onOpen(w.id)}
					>
						{w.thumbnail ? (
							<img
								src={w.thumbnail}
								alt=""
								loading="lazy"
								decoding="async"
								className="aspect-video w-full rounded-lg border border-border object-cover"
							/>
						) : null}
						<span className="flex flex-col gap-0.5">
							<b className="text-sm font-medium">{w.name}</b>
							{w.description ? (
								<span className="line-clamp-3 text-[13px] text-muted-foreground">
									{w.description}
								</span>
							) : null}
						</span>
					</button>
				))}
			</div>
		</ShowcaseSection>
	);
}

export function ShowcaseTemplates({
	templates,
}: Readonly<{ templates: IShowcaseTemplate[] }>) {
	const { t } = useTranslation("store");
	return (
		<ShowcaseSection
			id="templates"
			icon={LayoutTemplate}
			eyebrow={t("showcaseTemplatesEyebrow", "Templates")}
			title={t("showcaseTemplatesTitle", "Start your own from these")}
			subtitle={t(
				"showcaseTemplatesSubtitle",
				"Boards from this app you can copy into your own projects.",
			)}
		>
			<ul className={`${cardClass} flex flex-col p-0`}>
				{templates.map((tpl) => (
					<li
						key={tpl.id}
						className="flex items-center gap-3 border-b border-border px-3.5 py-3 last:border-b-0"
					>
						<span className="grid h-8 w-8 shrink-0 place-items-center rounded-lg bg-muted text-muted-foreground">
							<LayoutTemplate className="h-4 w-4" aria-hidden="true" />
						</span>
						<span className="flex min-w-0 flex-1 flex-col">
							<b className="font-medium">{tpl.name}</b>
							{tpl.description ? (
								<small className="text-[12.5px] text-muted-foreground">
									{tpl.description}
								</small>
							) : null}
						</span>
					</li>
				))}
			</ul>
		</ShowcaseSection>
	);
}
