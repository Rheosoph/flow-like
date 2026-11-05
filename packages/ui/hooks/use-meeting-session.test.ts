import { describe, expect, test } from "bun:test";
import type { MeetingSession } from "../lib/meetings/types";
import {
	MEETING_SESSION_LEASE_MS,
	meetingSessionActive,
} from "./use-meeting-session";

const session: MeetingSession = {
	meeting: {
		id: "id",
		conversation_id: "private-room",
		title: "Review",
		host_id: "host",
		starts_at: "2026-11-04T10:00:00Z",
		ends_at: "2026-11-04T11:00:00Z",
		ended_at: null,
		guests_allowed: true,
	},
	me: {
		id: "guest-session",
		display_name: "Guest",
		guest: true,
		status: "admitted",
		is_host: false,
	},
	participants: [],
	messages: [],
	room_epoch: 0,
	server_time: "2026-11-04T10:30:00Z",
};

describe("meeting access lease", () => {
	test("uses server time even when the device clock is wrong", () => {
		expect(meetingSessionActive(session, 1000, 2000)).toBe(true);
		expect(
			meetingSessionActive(
				session,
				Date.parse("2040-01-01"),
				Date.parse("2040-01-01") + 1000,
			),
		).toBe(true);
	});
	test("stops media on stale authorization, host end, removal, or waiting status", () => {
		expect(
			meetingSessionActive(session, 1000, 1000 + MEETING_SESSION_LEASE_MS),
		).toBe(false);
		expect(
			meetingSessionActive(
				{
					...session,
					meeting: { ...session.meeting, ended_at: session.server_time },
				},
				1000,
				1001,
			),
		).toBe(false);
		for (const status of ["waiting", "removed", "left"] as const)
			expect(
				meetingSessionActive(
					{ ...session, me: { ...session.me, status } },
					1000,
					1001,
				),
			).toBe(false);
	});
	test("stops at scheduled end without waiting for the next poll", () => {
		const ending = { ...session, server_time: "2026-11-04T10:59:59Z" };
		expect(meetingSessionActive(ending, 1000, 1999)).toBe(true);
		expect(meetingSessionActive(ending, 1000, 2000)).toBe(false);
	});
	test("requires fresh authorization after a backward clock adjustment", () => {
		expect(meetingSessionActive(session, 2000, 1000)).toBe(false);
		expect(
			meetingSessionActive({ ...session, server_time: "invalid" }, 1000, 2000),
		).toBe(false);
	});
});
