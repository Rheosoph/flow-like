import type { MeetingPreview } from "./types";

export const MEETING_EARLY_JOIN_MS = 15 * 60 * 1000;

export function meetingPhase(
	meeting: MeetingPreview,
	now = Date.now(),
): "upcoming" | "open" | "ended" {
	if (meeting.ended_at || now >= Date.parse(meeting.ends_at)) return "ended";
	return now < Date.parse(meeting.starts_at) - MEETING_EARLY_JOIN_MS
		? "upcoming"
		: "open";
}

/** Keep the invitation secret out of HTTP request URLs and referrers. */
export function meetingJoinUrl(
	baseUrl: string,
	id: string,
	token?: string,
): string {
	const url = new URL(baseUrl);
	url.searchParams.set("id", id);
	url.hash = token ? new URLSearchParams({ invite: token }).toString() : "";
	return url.toString();
}

export function meetingInviteFromHash(hash: string): string | undefined {
	return new URLSearchParams(hash.replace(/^#/, "")).get("invite") || undefined;
}

export function localDateTime(date: Date): string {
	const two = (value: number) => String(value).padStart(2, "0");
	return `${date.getFullYear()}-${two(date.getMonth() + 1)}-${two(date.getDate())}T${two(date.getHours())}:${two(date.getMinutes())}`;
}

export function scheduleRange(
	start: string,
	end: string,
): { starts_at: string; ends_at: string } {
	const starts = new Date(start);
	const ends = new Date(end);
	if (!Number.isFinite(starts.getTime()) || !Number.isFinite(ends.getTime()))
		throw new Error("Choose a valid start and end time.");
	if (localDateTime(starts) !== start || localDateTime(ends) !== end)
		throw new Error(
			"That time does not exist in your time zone. Choose another time.",
		);
	if (ends <= starts)
		throw new Error("The end time must be after the start time.");
	return { starts_at: starts.toISOString(), ends_at: ends.toISOString() };
}

function calendarText(value: string): string {
	return value
		.replace(/\\/g, "\\\\")
		.replace(/\r?\n/g, "\\n")
		.replace(/,/g, "\\,")
		.replace(/;/g, "\\;")
		.replace(/\r/g, "");
}

function calendarDate(value: string | number): string {
	return new Date(value)
		.toISOString()
		.replace(/[-:]/g, "")
		.replace(/\.\d{3}/, "");
}

/** Fold on UTF-8 byte boundaries so long titles stay valid in calendar clients. */
function foldCalendarLine(line: string): string {
	const encoder = new TextEncoder();
	let result = "";
	let bytes = 0;
	for (const char of line) {
		const size = encoder.encode(char).length;
		if (bytes + size > 75) {
			result += "\r\n ";
			bytes = 1;
		}
		result += char;
		bytes += size;
	}
	return result;
}

export function meetingCalendar(
	meeting: MeetingPreview,
	joinUrl: string,
	now = Date.now(),
): string {
	return [
		"BEGIN:VCALENDAR",
		"VERSION:2.0",
		"PRODID:-//Flow-Like//Meetings//EN",
		"CALSCALE:GREGORIAN",
		"BEGIN:VEVENT",
		`UID:${calendarText(meeting.id)}@flow-like`,
		`DTSTAMP:${calendarDate(now)}`,
		`DTSTART:${calendarDate(meeting.starts_at)}`,
		`DTEND:${calendarDate(meeting.ends_at)}`,
		`SUMMARY:${calendarText(meeting.title)}`,
		`DESCRIPTION:${calendarText(`Join the meeting: ${joinUrl}`)}`,
		`URL:${joinUrl.replace(/[\r\n]/g, "")}`,
		"END:VEVENT",
		"END:VCALENDAR",
		"",
	]
		.map(foldCalendarLine)
		.join("\r\n");
}

export function downloadMeetingCalendar(
	meeting: MeetingPreview,
	joinUrl: string,
): void {
	const url = URL.createObjectURL(
		new Blob([meetingCalendar(meeting, joinUrl)], {
			type: "text/calendar;charset=utf-8",
		}),
	);
	const link = document.createElement("a");
	link.href = url;
	link.download = `meeting-${meeting.id.replace(/[^a-zA-Z0-9_-]/g, "")}.ics`;
	link.click();
	setTimeout(() => URL.revokeObjectURL(url), 1000);
}

export function meetingError(error: unknown): string {
	return error instanceof Error
		? error.message
		: "The meeting request failed. Try again.";
}
