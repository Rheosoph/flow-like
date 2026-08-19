"use client";

import { useTranslation } from "@flow-like/locales";
import {
	ClipboardListIcon,
	ClockIcon,
	CodeIcon,
	CogIcon,
	FileTextIcon,
	FormInputIcon,
	GitBranchIcon,
	GlobeIcon,
	HashIcon,
	LayersIcon,
	LayoutIcon,
	LinkIcon,
	MailIcon,
	MessageSquareIcon,
	PlugIcon,
	SendIcon,
	ServerIcon,
	ZapIcon,
} from "lucide-react";
import { type ComponentType, useEffect, useRef } from "react";
import type {
	EventSectionId,
	IEventSection,
} from "../../../lib/event-sections";
import { cn } from "../../../lib/utils";
import type { IEventIssue } from "./use-event-issues";

const ICONS: Record<string, ComponentType<{ className?: string }>> = {
	clock: ClockIcon,
	globe: GlobeIcon,
	server: ServerIcon,
	plug: PlugIcon,
	"message-square": MessageSquareIcon,
	hash: HashIcon,
	send: SendIcon,
	mail: MailIcon,
	link: LinkIcon,
	zap: ZapIcon,
	"clipboard-list": ClipboardListIcon,
	layout: LayoutIcon,
	cog: CogIcon,
	layers: LayersIcon,
	"form-input": FormInputIcon,
	code: CodeIcon,
	"git-branch": GitBranchIcon,
	"file-text": FileTextIcon,
};

export function EventSectionRail({
	sections,
	active,
	onSelect,
	issues,
}: Readonly<{
	sections: IEventSection[];
	active: EventSectionId;
	onSelect: (id: EventSectionId) => void;
	issues: IEventIssue[];
}>) {
	const { t } = useTranslation("settings");
	const navRef = useRef<HTMLElement | null>(null);

	// In chip-strip mode the section jumped to from the attention strip can sit
	// off-screen, so the rail looks like it ignored the click.
	useEffect(() => {
		const nav = navRef.current;
		if (!nav || nav.scrollWidth <= nav.clientWidth) return;
		nav
			.querySelector(`[data-section="${active}"]`)
			?.scrollIntoView({ block: "nearest", inline: "center" });
	}, [active]);

	return (
		<nav
			ref={navRef}
			aria-label={t('eventSettingsSections', 'Event settings sections')}
			// Below lg the rail would stack ten full-width rows on top of the form,
			// pushing the actual fields a screen and a half down. There it becomes a
			// horizontally scrollable chip strip instead.
			className="-mx-1 flex gap-1.5 overflow-x-auto px-1 pb-1 [scrollbar-width:none] [&::-webkit-scrollbar]:hidden lg:mx-0 lg:flex-col lg:gap-0.5 lg:overflow-visible lg:px-0 lg:pb-0 lg:sticky lg:top-4 lg:self-start"
		>
			{sections.map((section, index) => {
				const Icon = ICONS[section.icon] ?? CogIcon;
				const sectionIssues = issues.filter((i) => i.section === section.id);
				const blocking = sectionIssues.some((i) => i.severity === "blocking");
				const isActive = active === section.id;
				return (
					<div
						key={section.id}
						data-section={section.id}
						className="shrink-0 lg:shrink lg:w-full"
					>
						{index === 1 && (
							<p className="hidden px-2.5 pb-1.5 pt-3 text-[10px] font-semibold uppercase tracking-[0.13em] text-muted-foreground lg:block">
								{t('everyEvent', 'Every event')}
							</p>
						)}
						<button
							type="button"
							onClick={() => onSelect(section.id)}
							aria-current={isActive}
							className={cn(
								"group relative flex w-full items-center gap-2 whitespace-nowrap rounded-full border px-3 py-2 text-left transition-colors lg:gap-2.5 lg:rounded-md lg:border-0 lg:px-2.5",
								isActive
									? "border-border bg-card shadow-sm lg:ring-1 lg:ring-border"
									: "border-transparent bg-muted/50 hover:bg-muted/60 lg:bg-transparent",
							)}
						>
							{isActive && (
								<span className="absolute -left-2 top-1/2 hidden h-4 w-0.5 -translate-y-1/2 rounded-r bg-primary lg:block" />
							)}
							<Icon
								className={cn(
									"h-4 w-4 shrink-0",
									isActive ? "text-primary" : "text-muted-foreground",
								)}
							/>
							<span
								className={cn(
									"min-w-0 flex-1 truncate text-[13px]",
									isActive ? "font-semibold" : "font-medium",
								)}
							>
								{section.label}
							</span>
							{sectionIssues.length > 0 && (
								<span
									className={cn(
										"grid h-4 min-w-4 shrink-0 place-items-center rounded-full px-1 text-[9.5px] font-bold",
										blocking
											? "bg-destructive text-destructive-foreground"
											: "bg-amber-500 text-black",
									)}
								>
									{sectionIssues.length}
								</span>
							)}
						</button>
					</div>
				);
			})}
		</nav>
	);
}
