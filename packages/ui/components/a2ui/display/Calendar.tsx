"use client";

import { useCallback, useMemo } from "react";
import {
	useComponentEventTrigger,
	useIsComponentTriggering,
} from "../ActionHandler";
import type { ComponentProps } from "../ComponentRegistry";
import { useData } from "../DataContext";
import { resolveInlineStyle, resolveStyle } from "../StyleResolver";
import {
	type WeekStartsOn,
	normalizeCalendarEvents,
	parseTimeToMinutes,
} from "../planning-utils";
import type {
	BoundValue,
	CalendarComponent,
	CalendarEvent,
	CalendarView,
} from "../types";
import {
	PLANNING_CALENDAR_VIEWS,
	PlanningCalendar,
	type PlanningCalendarChange,
} from "./planning-calendar";

function useResolved<T>(boundValue: BoundValue | undefined): T | undefined {
	const { resolve } = useData();
	if (!boundValue) return undefined;
	return resolve(boundValue) as T;
}

/** The workflow payload of each change, as A2UI calendars have always fired it. */
function changePayload(
	change: PlanningCalendarChange,
): Record<string, unknown> {
	const { event } = change;
	switch (change.type) {
		case "open":
		case "delete":
			return { id: event.id, metadata: event.metadata };
		case "create":
			return {
				id: event.id,
				start: event.start,
				end: event.end,
				allDay: event.allDay ?? false,
				event,
				...(change.sourceId === undefined ? {} : { sourceId: change.sourceId }),
			};
		case "update":
			return { id: event.id, event, previous: change.previous };
		case "move":
		case "resize":
			return {
				id: event.id,
				start: event.start,
				end: event.end,
				oldStart: change.previous.start,
				oldEnd: change.previous.end,
				metadata: event.metadata,
			};
	}
}

export function A2UICalendar({
	elementRef,
	component,
	componentId,
	style,
}: ComponentProps<CalendarComponent>) {
	const triggerEvent = useComponentEventTrigger(componentId);
	const isTriggering = useIsComponentTriggering(componentId);

	const rawEvents = useResolved<unknown>(component.events);
	const rawView = useResolved<string>(component.view);
	// Unknown view values fall back to month instead of a blank body.
	const view: CalendarView = PLANNING_CALENDAR_VIEWS.includes(
		rawView as CalendarView,
	)
		? (rawView as CalendarView)
		: "month";
	const date = useResolved<string>(component.date);
	const title = useResolved<string>(component.title) || undefined;
	const density = useResolved<string>(component.density);
	const editable = useResolved<boolean>(component.editable) ?? true;
	const selectable = useResolved<boolean>(component.selectable) ?? true;
	const firstDayOfWeek = (useResolved<number>(component.firstDayOfWeek) ??
		1) as WeekStartsOn;
	const minTime = parseTimeToMinutes(useResolved<string>(component.minTime), 0);
	const maxTime = parseTimeToMinutes(
		useResolved<string>(component.maxTime),
		24 * 60,
	);
	const slotDuration = useResolved<number>(component.slotDuration) ?? 30;
	const showWeekends = useResolved<boolean>(component.showWeekends) ?? true;
	const showNowIndicator =
		useResolved<boolean>(component.showNowIndicator) ?? true;
	const showAllDay = useResolved<boolean>(component.showAllDay) ?? true;
	const showViewSwitcher =
		useResolved<boolean>(component.showViewSwitcher) ?? true;
	const locale = useResolved<string>(component.locale) || undefined;
	const height = useResolved<string>(component.height);
	const responsive = useResolved<boolean>(component.responsive) ?? true;
	const compactBreakpoint = useResolved<number>(component.compactBreakpoint);

	// `useResolved` re-parses `literalJson` into a fresh array every render, so
	// key the memo on the serialized content — otherwise the calendar's sync
	// effect would fire on every render and wipe local edits/creates instantly.
	const eventsKey = JSON.stringify(rawEvents ?? null);
	// biome-ignore lint/correctness/useExhaustiveDependencies: eventsKey is the stable identity of rawEvents
	const events = useMemo<CalendarEvent[]>(
		() => normalizeCalendarEvents(rawEvents),
		[eventsKey],
	);

	const onChange = useCallback(
		(change: PlanningCalendarChange) => {
			const interaction = change.type;
			void triggerEvent(interaction, component, {
				interaction,
				...changePayload(change),
			});
		},
		[triggerEvent, component],
	);

	return (
		<PlanningCalendar
			events={events}
			view={view}
			date={date}
			title={title}
			density={density}
			editable={editable}
			selectable={selectable}
			firstDayOfWeek={firstDayOfWeek}
			minTime={minTime}
			maxTime={maxTime}
			slotDuration={slotDuration}
			showWeekends={showWeekends}
			showNowIndicator={showNowIndicator}
			showAllDay={showAllDay}
			showViewSwitcher={showViewSwitcher}
			locale={locale}
			height={height}
			responsive={responsive}
			compactBreakpoint={compactBreakpoint ?? undefined}
			busy={isTriggering}
			className={resolveStyle(style)}
			style={resolveInlineStyle(style)}
			elementRef={elementRef}
			onChange={onChange}
		/>
	);
}
