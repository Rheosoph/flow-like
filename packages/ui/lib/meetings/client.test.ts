import { describe, expect, test } from "bun:test";
import { type MeetingRequest, createMeetingClient } from "./client";

describe("meeting credential routing", () => {
	test("public and session requests never use account authentication", async () => {
		const calls: { path: string; init: RequestInit; authenticated: boolean }[] =
			[];
		const request: MeetingRequest = async <T>(
			path: string,
			init: RequestInit,
			options: { authenticated: boolean },
		) => {
			calls.push({ path, init, authenticated: options.authenticated });
			return { meetings: [] } as T;
		};
		const client = createMeetingClient({ request });
		await client.preview("id", "invitation");
		await client.joinGuest("id", "invitation", "Guest");
		await client.session("id", "session-secret");
		await client.realtime("id", "session-secret");
		await client.send("id", "session-secret", "hello");
		await client.admit("id", "person", "session-secret");
		await client.end("id", "session-secret");
		await client.leave("id", "session-secret");
		for (const call of calls) {
			expect(call.authenticated).toBe(false);
			expect(call.path).not.toMatch(/secret|invitation/);
			expect(call.init.referrerPolicy).toBe("no-referrer");
			expect(new Headers(call.init.headers).has("Authorization")).toBe(false);
		}
		expect(JSON.parse(calls[1].init.body as string)).toEqual({
			invite_token: "invitation",
			display_name: "Guest",
		});
		expect(
			new Headers(calls[2].init.headers).get("x-flow-meeting-session"),
		).toBe("session-secret");
		expect(calls.at(-1)?.init.keepalive).toBe(true);
	});
	test("sharing reads the current invite while replacing it is an explicit mutation", async () => {
		const calls: { path: string; init: RequestInit; authenticated: boolean }[] =
			[];
		const client = createMeetingClient({
			request: async <T>(
				path: string,
				init: RequestInit,
				options: { authenticated: boolean },
			) => {
				calls.push({ path, init, authenticated: options.authenticated });
				return { meetings: [] } as T;
			},
		});
		await client.list("room & other");
		await client.invite("id");
		await client.rotateInvite("id");
		await client.join("id");
		await client.preview("id");
		expect(calls.every((call) => call.authenticated)).toBe(true);
		expect(calls[0].path).toBe("meetings?conversation_id=room%20%26%20other");
		expect(calls[1].init.method).toBe("GET");
		expect(calls[2].init.method).toBe("POST");
	});
});
