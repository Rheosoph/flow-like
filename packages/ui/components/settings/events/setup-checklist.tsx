"use client";

import { useTranslation } from "@flow-like/locales";
import { CheckIcon, ExternalLinkIcon } from "lucide-react";
import { useCallback, useMemo, useState } from "react";
import type {
	EventSectionId,
	IEventGuideStep,
} from "../../../lib/event-sections";
import { getEventGuide } from "../../../lib/event-sections";
import type { IEvent } from "../../../lib/schema/flow/event";
import { cn } from "../../../lib/utils";
import { Button } from "../../ui/button";

const STORAGE_PREFIX = "flow-like:event-setup:";

function readConfirmed(eventId: string): Record<string, boolean> {
	if (typeof window === "undefined") return {};
	try {
		return JSON.parse(
			window.localStorage.getItem(`${STORAGE_PREFIX}${eventId}`) ?? "{}",
		);
	} catch {
		return {};
	}
}

function writeConfirmed(eventId: string, value: Record<string, boolean>) {
	if (typeof window === "undefined") return;
	try {
		window.localStorage.setItem(
			`${STORAGE_PREFIX}${eventId}`,
			JSON.stringify(value),
		);
	} catch {
		// A full or blocked storage quota must not break the screen.
	}
}

/**
 * Ordered setup steps for the event, mixing what happens on this screen with
 * what happens in someone else's product. Steps that can be derived from the
 * config tick themselves; the rest are user-confirmed and remembered locally.
 */
export function SetupChecklist({
	event,
	config,
	onNavigate,
}: Readonly<{
	event: IEvent;
	config: Record<string, unknown>;
	onNavigate: (section: EventSectionId) => void;
}>) {
	const { t } = useTranslation("settings");
	const [confirmed, setConfirmed] = useState<Record<string, boolean>>(() =>
		readConfirmed(event.id),
	);
	const [expanded, setExpanded] = useState(false);

	const steps = useMemo(() => getEventGuide(event), [event]);

	const isDone = useCallback(
		(step: IEventGuideStep) =>
			step.auto ? step.auto(config, event) : !!confirmed[step.id],
		[config, event, confirmed],
	);

	const doneCount = steps.filter(isDone).length;
	const complete = doneCount === steps.length;

	const toggle = useCallback(
		(id: string) => {
			setConfirmed((previous) => {
				const next = { ...previous, [id]: !previous[id] };
				writeConfirmed(event.id, next);
				return next;
			});
		},
		[event.id],
	);

	const showSteps = !complete || expanded;

	return (
		<div
			className={cn(
				"overflow-hidden rounded-lg border bg-card",
				complete ? "border-green-600/40" : "border-primary/35",
			)}
		>
			<div className="flex items-center justify-between gap-2 border-b px-3 py-2">
				<span className="text-[10px] font-semibold uppercase tracking-[0.1em] text-muted-foreground">
					{t("setThisUp", "Set this up")}
				</span>
				<span
					className={cn(
						"rounded-full px-2 py-0.5 text-[11px] font-semibold",
						complete
							? "bg-green-600/15 text-green-700 dark:text-green-400"
							: "bg-primary/10 text-primary",
					)}
				>
					{t("donecountOfLength", "{{doneCount}} of {{length}}", {
						doneCount,
						length: steps.length,
					})}
				</span>
			</div>

			<div className="h-[3px] bg-muted">
				<div
					className={cn(
						"h-full transition-[width] duration-300",
						complete ? "bg-green-600" : "bg-primary",
					)}
					style={{ width: `${(doneCount / Math.max(1, steps.length)) * 100}%` }}
				/>
			</div>

			{complete && !expanded && (
				<div className="flex flex-wrap items-center gap-2 px-3 py-2.5 text-xs text-muted-foreground">
					<span className="rounded-full bg-green-600/15 px-2 py-0.5 text-[11px] font-semibold text-green-700 dark:text-green-400">
						{t("allDone", "All done")}
					</span>
					<span className="flex-1">
						{t(
							"nothingLeftBeforeThisCarriesTraffic",
							"Nothing left before this carries traffic.",
						)}
					</span>
					<Button
						variant="outline"
						size="sm"
						className="h-6 px-2 text-[11px]"
						onClick={() => setExpanded(true)}
					>
						{t("review", "Review")}
					</Button>
				</div>
			)}

			{showSteps && (
				<div className="flex flex-col">
					{complete && (
						<div className="flex items-center justify-end px-3 pt-2">
							<Button
								variant="ghost"
								size="sm"
								className="h-6 px-2 text-[11px]"
								onClick={() => setExpanded(false)}
							>
								{t("collapse", "Collapse")}
							</Button>
						</div>
					)}
					{steps.map((step) => {
						const done = isDone(step);
						const derived = !!step.auto;
						return (
							<div
								key={step.id}
								className={cn(
									"flex gap-2.5 border-b px-3 py-2.5 last:border-b-0",
									done && "opacity-60",
								)}
							>
								<button
									type="button"
									disabled={derived}
									onClick={() => toggle(step.id)}
									aria-label={
										done
											? t("doneTitle", "Done: {{title}}", { title: step.title })
											: t("markDoneTitle", "Mark done: {{title}}", {
													title: step.title,
												})
									}
									className={cn(
										"mt-0.5 grid h-4 w-4 shrink-0 place-items-center rounded border",
										derived ? "border-dashed" : "cursor-pointer",
										done
											? "border-green-600 bg-green-600 text-white"
											: "border-border bg-background hover:border-primary",
									)}
								>
									{done && <CheckIcon className="h-2.5 w-2.5" />}
								</button>
								<div className="min-w-0">
									<div className="flex flex-wrap items-center gap-1.5">
										<span
											className={cn(
												"text-[12.5px] font-medium leading-snug",
												done && "line-through",
											)}
										>
											{step.title}
										</span>
										{step.external && (
											<span className="rounded-sm bg-amber-500/15 px-1.5 py-px text-[9px] font-bold uppercase tracking-wide text-amber-700 dark:text-amber-400">
												{t("outsideFlowlike", "outside Flow-Like")}
											</span>
										)}
									</div>
									<p className="mt-0.5 text-[11.5px] leading-relaxed text-muted-foreground">
										{step.why}
									</p>
									{step.where && (
										<p className="mt-1 flex items-center gap-1 font-mono text-[10.5px] text-muted-foreground">
											<ExternalLinkIcon className="h-3 w-3" />
											{step.where}
										</p>
									)}
									{step.section && !done && (
										<Button
											variant="outline"
											size="sm"
											className="mt-1.5 h-6 px-2 text-[11px]"
											onClick={() => onNavigate(step.section as EventSectionId)}
										>
											{t("takeMeThere", "Take me there")}
										</Button>
									)}
								</div>
							</div>
						);
					})}
				</div>
			)}

			<p className="border-t px-3 py-2 text-[11px] leading-relaxed text-muted-foreground">
				{`Dashed boxes tick themselves from your settings. Solid ones are yours to confirm — they are the steps that happen outside this screen.`}
			</p>
		</div>
	);
}
