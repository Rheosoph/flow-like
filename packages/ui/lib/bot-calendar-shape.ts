import {
	DEFAULT_CALENDAR_WAKE,
	type ICalendarFeed,
	type ICalendarRange,
	type ICalendarWake,
} from "../state/backend-state/bot-state";
import { ApiResponseError } from "./api-error";
import { asArray, isRecord } from "./response-shape";

/** A calendar listing as a host answered it; anything but an object is an error naming `operation`. */
export function expectCalendarRange(
	value: unknown,
	operation: string,
): ICalendarRange {
	if (!isRecord(value)) {
		throw new Error(`Could not ${operation}: the answer was not a calendar`);
	}
	return {
		from: typeof value.from === "string" ? value.from : "",
		to: typeof value.to === "string" ? value.to : "",
		entries: asArray(value.entries as ICalendarRange["entries"]).filter(
			(entry) =>
				isRecord(entry) &&
				typeof entry.id === "string" &&
				typeof entry.starts_at === "string",
		),
		sources: asArray(value.sources as ICalendarRange["sources"]).filter(
			isRecord,
		),
	};
}

/** Wake-up settings with the defaults filled in for what an older host left out. */
export function expectCalendarWake(
	value: unknown,
	operation: string,
): ICalendarWake {
	if (!isRecord(value)) {
		throw new Error(
			`Could not ${operation}: the answer had no wake-up settings`,
		);
	}
	return { ...DEFAULT_CALENDAR_WAKE, ...(value as Partial<ICalendarWake>) };
}

/** A calendar feed as a host answered it; anything without a link is an error naming `operation`. */
export function expectCalendarFeed(
	value: unknown,
	operation: string,
): ICalendarFeed {
	if (
		!isRecord(value) ||
		typeof value.url !== "string" ||
		typeof value.webcal_url !== "string"
	) {
		throw new Error(`Could not ${operation}: the answer had no calendar link`);
	}
	const flag = (key: string, fallback: boolean) =>
		typeof value[key] === "boolean" ? (value[key] as boolean) : fallback;
	return {
		url: value.url,
		webcal_url: value.webcal_url,
		include_events: flag("include_events", true),
		include_tasks: flag("include_tasks", true),
		include_memories: flag("include_memories", false),
		include_notes: flag("include_notes", false),
		created_at: typeof value.created_at === "string" ? value.created_at : "",
		last_fetched_at:
			typeof value.last_fetched_at === "string" ? value.last_fetched_at : null,
	};
}

/** The 404 a bot answers while it has no calendar link yet. */
export function isNoCalendarFeed(error: unknown): boolean {
	return (
		error instanceof ApiResponseError &&
		error.status === 404 &&
		error.code === "BOT_CALENDAR_FEED_NONE"
	);
}
