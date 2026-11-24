"use client";

import { useTranslation } from "@flow-like/locales";
import { useEffect, useMemo, useState } from "react";
import type { IApp } from "../../../lib/schema/app/app";
import type { IAppShowcase } from "../../../lib/schema/app/showcase";
import { cn } from "../../../lib/utils";
import { type ShowcaseSectionKey, visibleSections } from "./model";
import {
	ABOUT_DOM_ID,
	REVIEWS_DOM_ID,
	scrollToSection,
	sectionDomId,
} from "./showcase-ui";

export function ShowcaseNav({
	app,
	showcase,
}: Readonly<{ app: IApp; showcase: IAppShowcase }>) {
	const { t } = useTranslation("store");
	const sections = useMemo(
		() => visibleSections(showcase, app.app_type),
		[showcase, app.app_type],
	);
	const labels = useMemo(
		(): Record<ShowcaseSectionKey, string> => ({
			data: t("showcaseNavData", "Data"),
			map: t("showcaseNavMap", "How it works"),
			triggers: t("showcaseNavTriggers", "Ways to use"),
			pages: t("showcaseNavPages", "Pages"),
			widgets: t("showcaseNavWidgets", "Widgets"),
			templates: t("showcaseNavTemplates", "Templates"),
			hood: t("showcaseNavHood", "Under the hood"),
			activity: t("showcaseNavActivity", "Activity"),
		}),
		[t],
	);
	const items = useMemo(
		() => [
			{ id: ABOUT_DOM_ID, label: t("showcaseNavAbout", "About") },
			...sections.map((k) => ({ id: sectionDomId(k), label: labels[k] })),
			{ id: REVIEWS_DOM_ID, label: t("showcaseNavReviews", "Reviews") },
		],
		[sections, labels, t],
	);
	const [current, setCurrent] = useState(ABOUT_DOM_ID);

	useEffect(() => {
		if (typeof IntersectionObserver === "undefined") return;
		const seen = new Map<string, boolean>();
		const observer = new IntersectionObserver(
			(entries) => {
				for (const en of entries) seen.set(en.target.id, en.isIntersecting);
				const first = items.find((i) => seen.get(i.id));
				if (first) setCurrent(first.id);
			},
			{ rootMargin: "-72px 0px -60% 0px" },
		);
		for (const i of items) {
			const el = document.getElementById(i.id);
			if (el) observer.observe(el);
		}
		return () => observer.disconnect();
	}, [items]);

	if (sections.length === 0) return null;

	return (
		<nav
			aria-label={t("showcaseNavLabel", "Sections")}
			className="sticky top-0 z-30 mt-6 border-b border-border bg-background/90 backdrop-blur-md"
		>
			<div className="mx-auto flex max-w-5xl gap-0.5 overflow-x-auto px-6 py-1.5 no-scrollbar md:px-10">
				{items.map((i) => (
					<button
						key={i.id}
						type="button"
						aria-current={current === i.id ? "true" : undefined}
						onClick={() => scrollToSection(i.id)}
						className={cn(
							"whitespace-nowrap rounded-md px-2.5 py-1.5 text-[13px] font-medium text-muted-foreground transition-colors hover:bg-muted hover:text-foreground focus-visible:outline-none focus-visible:ring-2 focus-visible:ring-ring",
							current === i.id && "bg-muted text-foreground",
						)}
					>
						{i.label}
					</button>
				))}
			</div>
		</nav>
	);
}
