"use client";

import { TriangleAlertIcon } from "lucide-react";
import type {
	EventSectionId,
	IEventSection,
} from "../../../lib/event-sections";
import { cn } from "../../../lib/utils";
import { Button } from "../../ui/button";
import type { IEventIssue } from "./use-event-issues";

/**
 * Everything blocking or worth checking, named and addressable. Sits above the
 * section content so a broken event is visible without opening every section.
 */
export function EventAttentionStrip({
	issues,
	sections,
	onNavigate,
}: Readonly<{
	issues: IEventIssue[];
	sections: IEventSection[];
	onNavigate: (section: EventSectionId) => void;
}>) {
	if (issues.length === 0) return null;
	const hasBlocking = issues.some((i) => i.severity === "blocking");

	return (
		<div
			className={cn(
				"mb-5 overflow-hidden rounded-md border",
				hasBlocking
					? "border-destructive/40 bg-destructive/5"
					: "border-amber-500/40 bg-amber-500/5",
			)}
		>
			{issues.map((issue) => {
				const target = sections.find((s) => s.id === issue.section);
				return (
					<div
						key={issue.id}
						className="flex flex-wrap items-center gap-x-3 gap-y-1 border-b border-foreground/5 px-3.5 py-2 text-[12.5px] last:border-b-0"
					>
						<span
							className={cn(
								"inline-flex shrink-0 items-center gap-1 rounded-full px-2 py-0.5 text-[11px] font-semibold",
								issue.severity === "blocking"
									? "bg-destructive/15 text-destructive"
									: "bg-amber-500/15 text-amber-700 dark:text-amber-400",
							)}
						>
							<TriangleAlertIcon className="h-3 w-3" />
							{issue.severity === "blocking" ? "Blocking" : "Check"}
						</span>
						<span className="min-w-0 flex-1">
							<span className="font-semibold">{issue.title}</span>
							<span className="text-muted-foreground"> — {issue.detail}</span>
						</span>
						{target && (
							<Button
								variant="link"
								size="sm"
								className="h-auto shrink-0 p-0 text-[12px]"
								onClick={() => onNavigate(issue.section)}
							>
								Go to {target.label}
							</Button>
						)}
					</div>
				);
			})}
		</div>
	);
}
