"use client";

import { useTranslation } from "@flow-like/locales";
import {
	Cloud,
	Database,
	LayoutDashboard,
	LayoutGrid,
	LayoutTemplate,
	type LucideIcon,
	Monitor,
	Smartphone,
	Sparkles,
	Zap,
} from "lucide-react";
import { type ReactNode, useMemo } from "react";
import type { IApp } from "../../../lib/schema/app/app";
import type { IAppShowcase } from "../../../lib/schema/app/showcase";
import {
	type ShowcaseSectionKey,
	compact,
	eventKind,
	sumRows,
	visibleSections,
} from "./model";
import { scrollToSection, sectionDomId } from "./showcase-ui";

interface Capability {
	id: string;
	section: ShowcaseSectionKey;
	icon: LucideIcon;
	tone: string;
	content: ReactNode;
}

export function ShowcaseCapabilities({
	app,
	showcase,
}: Readonly<{ app: IApp; showcase: IAppShowcase }>) {
	const { t } = useTranslation("store");

	const caps = useMemo((): Capability[] => {
		const shown = new Set(visibleSections(showcase, app.app_type));
		const out: Capability[] = [];
		const { tables, events, pages, widgets, templates, hood } = showcase;

		if (tables.length > 0 && shown.has("data")) {
			const rows = sumRows(tables);
			out.push({
				id: "tables",
				section: "data",
				icon: Database,
				tone: "text-[var(--showcase-text)]",
				content: (
					<>
						{t("showcaseCapTables", {
							defaultValue_one: "{{count}} table",
							defaultValue_other: "{{count}} tables",
							count: tables.length,
						})}
						{rows !== null ? (
							<>
								{" · "}
								{t("showcaseCapRows", {
									defaultValue_one: "{{formatted}} row",
									defaultValue_other: "{{formatted}} rows",
									count: rows,
									formatted: compact(rows),
								})}
							</>
						) : null}
					</>
				),
			});
		}
		if (events.length > 0 && shown.has("triggers")) {
			const labels = [
				...new Set(
					events.map((e) => {
						const [key, fallback] = eventKind(e.event_type).short;
						return t(key as "showcaseEventOther", fallback);
					}),
				),
			];
			out.push({
				id: "triggers",
				section: "triggers",
				icon: Zap,
				tone: "text-primary",
				content: (
					<>
						{labels.slice(0, 2).join(", ")}
						{labels.length > 2
							? ` ${t("showcaseCapMore", "+{{count}}", { count: labels.length - 2 })}`
							: ""}
					</>
				),
			});
		}
		if (pages.length > 0 && shown.has("pages"))
			out.push({
				id: "pages",
				section: "pages",
				icon: LayoutDashboard,
				tone: "text-tertiary",
				content: t("showcaseCapPages", {
					defaultValue_one: "{{count}} page",
					defaultValue_other: "{{count}} pages",
					count: pages.length,
				}),
			});
		if (widgets.length > 0 && shown.has("widgets"))
			out.push({
				id: "widgets",
				section: "widgets",
				icon: LayoutGrid,
				tone: "text-tertiary",
				content: t("showcaseCapWidgets", {
					defaultValue_one: "{{count}} Home widget",
					defaultValue_other: "{{count}} Home widgets",
					count: widgets.length,
				}),
			});
		if (templates.length > 0 && shown.has("templates"))
			out.push({
				id: "templates",
				section: "templates",
				icon: LayoutTemplate,
				tone: "text-muted-foreground",
				content: t("showcaseCapTemplates", {
					defaultValue_one: "{{count}} template",
					defaultValue_other: "{{count}} templates",
					count: templates.length,
				}),
			});
		if (hood && shown.has("hood")) {
			if (hood.models && hood.models.length > 0)
				out.push({
					id: "models",
					section: "hood",
					icon: Sparkles,
					tone: "text-primary",
					content: t("showcaseCapModels", {
						defaultValue_one: "{{count}} AI model",
						defaultValue_other: "{{count}} AI models",
						count: hood.models.length,
					}),
				});
			if (hood.execution_mode) {
				const mode = hood.execution_mode;
				out.push({
					id: "runs",
					section: "hood",
					icon:
						mode === "Remote" ? Cloud : mode === "Local" ? Smartphone : Monitor,
					tone: "text-muted-foreground",
					content:
						mode === "Remote"
							? t("showcaseCapRunsCloud", "Runs in the cloud")
							: mode === "Local"
								? t("showcaseCapRunsDevice", "Runs on your device")
								: t("showcaseCapRunsAny", "Cloud or device"),
				});
			}
		}
		return out;
	}, [showcase, app.app_type, t]);

	if (caps.length === 0) return null;

	return (
		<div
			className="mt-4 flex flex-wrap gap-2"
			// biome-ignore lint/a11y/useSemanticElements: a fieldset would add form semantics
			role="group"
			aria-label={t("showcaseCapsLabel", "What's inside")}
		>
			{caps.map((c) => (
				<button
					key={c.id}
					type="button"
					onClick={() => scrollToSection(sectionDomId(c.section))}
					className="inline-flex items-center gap-2 rounded-full border border-border bg-card py-1 pr-3 pl-1.5 text-[12.5px] transition-colors hover:border-foreground/30 focus-visible:outline-none focus-visible:ring-2 focus-visible:ring-ring"
				>
					<span className="grid h-5.5 w-5.5 place-items-center rounded-full bg-muted">
						<c.icon className={`h-3.5 w-3.5 ${c.tone}`} aria-hidden="true" />
					</span>
					<span>{c.content}</span>
				</button>
			))}
		</div>
	);
}
