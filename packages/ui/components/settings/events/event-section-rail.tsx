"use client";

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
import type { ComponentType } from "react";
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
	return (
		<nav
			aria-label="Event settings sections"
			className="flex flex-col gap-0.5 lg:sticky lg:top-4 lg:self-start"
		>
			{sections.map((section, index) => {
				const Icon = ICONS[section.icon] ?? CogIcon;
				const sectionIssues = issues.filter((i) => i.section === section.id);
				const blocking = sectionIssues.some((i) => i.severity === "blocking");
				const isActive = active === section.id;
				return (
					<div key={section.id}>
						{index === 1 && (
							<p className="px-2.5 pb-1.5 pt-3 text-[10px] font-semibold uppercase tracking-[0.13em] text-muted-foreground">
								Every event
							</p>
						)}
						<button
							type="button"
							onClick={() => onSelect(section.id)}
							aria-current={isActive}
							className={cn(
								"group relative flex w-full items-center gap-2.5 rounded-md px-2.5 py-2 text-left transition-colors",
								isActive
									? "bg-card shadow-sm ring-1 ring-border"
									: "hover:bg-muted/60",
							)}
						>
							{isActive && (
								<span className="absolute -left-2 top-1/2 h-4 w-0.5 -translate-y-1/2 rounded-r bg-primary" />
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
