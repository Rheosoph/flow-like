import { describe, expect, test } from "bun:test";
import {
	localDateTime,
	meetingCalendar,
	meetingInviteFromHash,
	meetingJoinUrl,
	meetingPhase,
	scheduleRange,
} from "./helpers";

const meeting = {
	id: "meeting-1",
	title: "Design review",
	starts_at: "2026-11-04T10:00:00Z",
	ends_at: "2026-11-04T11:00:00Z",
	ended_at: null,
	guests_allowed: true,
};

describe("meeting schedule and sharing", () => {
	test("opens fifteen minutes early and closes exactly at the end", () => {
		expect(meetingPhase(meeting, Date.parse("2026-11-04T09:44:59Z"))).toBe(
			"upcoming",
		);
		expect(meetingPhase(meeting, Date.parse("2026-11-04T09:45:00Z"))).toBe(
			"open",
		);
		expect(meetingPhase(meeting, Date.parse(meeting.ends_at))).toBe("ended");
		expect(
			meetingPhase(
				{ ...meeting, ended_at: meeting.starts_at },
				Date.parse(meeting.starts_at),
			),
		).toBe("ended");
	});
	test("keeps the guest secret in the URL fragment and replaces old invitation data", () => {
		const url = new URL(
			meetingJoinUrl(
				"https://example.test/meeting#invite=old",
				"meeting / 1",
				"secret+/=",
			),
		);
		expect(url.searchParams.get("id")).toBe("meeting / 1");
		expect(url.search).not.toContain("secret");
		expect(meetingInviteFromHash(url.hash)).toBe("secret+/=");
		expect(new URL(meetingJoinUrl(url.toString(), "next")).hash).toBe("");
	});
	test("converts local wall time to UTC and rejects reversed or invalid ranges", () => {
		const start = new Date(2026, 10, 4, 10, 0);
		const end = new Date(2026, 10, 4, 11, 0);
		expect(scheduleRange(localDateTime(start), localDateTime(end))).toEqual({
			starts_at: start.toISOString(),
			ends_at: end.toISOString(),
		});
		expect(() =>
			scheduleRange(localDateTime(end), localDateTime(start)),
		).toThrow("after");
		expect(() => scheduleRange("invalid", "invalid")).toThrow("valid");
	});
	test("calendar exports escape injected lines and fold long unicode titles by bytes", () => {
		const calendar = meetingCalendar(
			{ ...meeting, title: `Design, review;\nBEGIN:VTODO ${"👋".repeat(80)}` },
			"https://example.test/meeting?id=1#invite=secret",
			0,
		);
		expect(calendar).toContain("DTSTART:20261104T100000Z\r\n");
		expect(calendar).toContain("DTEND:20261104T110000Z\r\n");
		expect(calendar).toContain("SUMMARY:Design\\, review\\;\\nBEGIN:VTODO");
		expect(calendar).not.toContain("\r\nBEGIN:VTODO");
		for (const line of calendar.split("\r\n"))
			expect(new TextEncoder().encode(line).length).toBeLessThanOrEqual(75);
	});
});
