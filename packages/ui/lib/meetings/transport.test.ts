import { describe, expect, mock, test } from "bun:test";
import { createMeetingTransport, meetingPageUrl } from "./transport";

describe("meeting transport", () => {
	test("keeps guest session credentials out of account authentication", async () => {
		const accountRequest = mock(async () => ({}));
		const send = mock(async () => new Response('{"ok":true}'));
		const request = createMeetingTransport({
			accountRequest: accountRequest as never,
			apiUrl: (path) => `https://hub.test/api/v1/${path}`,
			fetch: send as never,
		});
		expect(
			await request<{ ok: boolean }>(
				"meetings/m/session",
				{
					method: "GET",
					headers: {
						Authorization: "Bearer account",
						"x-flow-meeting-session": "guest",
					},
				},
				{ authenticated: false },
			),
		).toEqual({ ok: true });
		expect(accountRequest).not.toHaveBeenCalled();
		const [url, init] = send.mock.calls[0] as unknown as [string, RequestInit];
		expect(url).toBe("https://hub.test/api/v1/meetings/m/session");
		expect(new Headers(init.headers).get("Authorization")).toBeNull();
		expect(new Headers(init.headers).get("x-flow-meeting-session")).toBe(
			"guest",
		);
		expect(init.credentials).toBe("omit");
		expect(init.referrerPolicy).toBe("no-referrer");
		expect(init.redirect).toBe("error");
	});

	test("account operations keep platform session renewal", async () => {
		const accountRequest = mock(async () => ({ meetings: [] }));
		const send = mock(async () => new Response());
		const request = createMeetingTransport({
			accountRequest: accountRequest as never,
			apiUrl: (path) => path,
			fetch: send as never,
		});
		const init = { method: "GET" };
		await request("meetings?conversation_id=room", init, {
			authenticated: true,
		});
		expect(accountRequest).toHaveBeenCalledWith(
			"meetings?conversation_id=room",
			init,
		);
		expect(send).not.toHaveBeenCalled();
	});

	test("keeps expired-session errors inspectable without an account sign-in redirect", async () => {
		const request = createMeetingTransport({
			accountRequest: mock() as never,
			apiUrl: (path) => path,
			fetch: (async () =>
				new Response('{"message":"Meeting ended"}', { status: 403 })) as never,
		});
		await expect(
			request("meetings/m/session", {}, { authenticated: false }),
		).rejects.toMatchObject({ status: 403 });
	});
});

test("meeting links use the configured web app and preserve a deployment path", () => {
	expect(
		meetingPageUrl("https://example.test/work/", "https://other.test"),
	).toBe("https://example.test/work/meeting");
	expect(meetingPageUrl(undefined, "https://example.test")).toBe(
		"https://example.test/meeting",
	);
	expect(meetingPageUrl(undefined, "tauri://localhost")).toBeUndefined();
	expect(meetingPageUrl("https://user:password@example.test")).toBeUndefined();
	expect(meetingPageUrl("javascript:alert(1)")).toBeUndefined();
});
