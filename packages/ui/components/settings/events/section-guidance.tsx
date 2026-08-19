"use client";

import { useTranslation } from "@flow-like/locales";
import { InfoIcon, XIcon } from "lucide-react";
import { useCallback, useState } from "react";
import type { EventSectionId } from "../../../lib/event-sections";
import {
	getEventSections,
	getSectionGuidance,
} from "../../../lib/event-sections";
import type { IEvent } from "../../../lib/schema/flow/event";
import { Button } from "../../ui/button";

const STORAGE_KEY = "flow-like:event-guidance-dismissed";

function readDismissed(): Record<string, boolean> {
	if (typeof window === "undefined") return {};
	try {
		return JSON.parse(window.localStorage.getItem(STORAGE_KEY) ?? "{}");
	} catch {
		return {};
	}
}

/**
 * What a section is for, and the mistake people actually make in it.
 * Dismissible per event type + section, restorable from the pill it leaves behind.
 */
export function SectionGuidance({
	event,
	section,
}: Readonly<{ event: IEvent; section: EventSectionId }>) {
	const { t } = useTranslation("settings");
	const [dismissed, setDismissed] = useState<Record<string, boolean>>(() =>
		readDismissed(),
	);
	const key = `${event.event_type}.${section}`;

	const persist = useCallback((next: Record<string, boolean>) => {
		setDismissed(next);
		if (typeof window === "undefined") return;
		try {
			window.localStorage.setItem(STORAGE_KEY, JSON.stringify(next));
		} catch {
			// Storage being unavailable must not break the screen.
		}
	}, []);

	const guidance = getSectionGuidance(event, section);
	if (!guidance) return null;

	// The section header already states what the section is for. Repeating it
	// here is noise, so only show `what` when it says something the header didn't.
	const blurb = getEventSections(event).find((s) => s.id === section)?.blurb;
	const showWhat = !!guidance.what && guidance.what !== blurb;

	if (dismissed[key]) {
		return (
			<Button
				variant="ghost"
				size="sm"
				className="mb-4 h-7 gap-1.5 rounded-full border border-dashed px-3 text-[11.5px] text-muted-foreground"
				onClick={() => persist({ ...dismissed, [key]: false })}
			>
				<InfoIcon className="h-3 w-3" />
				{t('showGuidanceForThisSection', 'Show guidance for this section')}
			</Button>
		);
	}

	return (
		<div className="mb-5 flex items-start gap-3 rounded-md border border-primary/25 bg-primary/5 px-4 py-3">
			<InfoIcon className="mt-0.5 h-4 w-4 shrink-0 text-primary" />
			<div className="min-w-0 flex-1">
				{showWhat && (
					<p className="text-[12.5px] font-medium">{guidance.what}</p>
				)}
				<p
					className={`text-[12.5px] leading-relaxed text-muted-foreground ${showWhat ? "mt-1" : ""}`}
				>
					<span className="font-semibold text-foreground">
						{t('mostCommonMistake', 'Most common mistake:')}
					</span>{" "}
					{guidance.mistake}
				</p>
			</div>
			<button
				type="button"
				aria-label={t('hideGuidance', 'Hide guidance')}
				onClick={() => persist({ ...dismissed, [key]: true })}
				className="shrink-0 rounded p-0.5 text-muted-foreground hover:text-foreground"
			>
				<XIcon className="h-3.5 w-3.5" />
			</button>
		</div>
	);
}
