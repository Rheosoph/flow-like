import type {
	CreateMeeting,
	Meeting,
	MeetingClient,
	MeetingCredentials,
	MeetingPreview,
	MeetingRealtimeAccess,
	MeetingSession,
} from "./types";

/** Account requests use the platform session; guest credentials never enter that path. */
export type MeetingRequest = <T>(
	path: string,
	init: RequestInit,
	options: { authenticated: boolean },
) => Promise<T>;

export function createMeetingClient({
	request,
}: { request: MeetingRequest }): MeetingClient {
	const path = (id: string, suffix = "") =>
		`meetings/${encodeURIComponent(id)}${suffix}`;
	const call = <T>(
		route: string,
		body: unknown,
		authenticated: boolean,
		token?: string,
		method = "POST",
		extra: RequestInit = {},
	) =>
		request<T>(
			route,
			{
				...extra,
				method,
				cache: "no-store",
				referrerPolicy: "no-referrer",
				headers: {
					"Content-Type": "application/json",
					...(token ? { "x-flow-meeting-session": token } : {}),
				},
				...(method === "GET" ? {} : { body: JSON.stringify(body) }),
			},
			{ authenticated },
		);
	return {
		list: async (id) =>
			(
				await call<{ meetings: Meeting[] }>(
					`meetings?conversation_id=${encodeURIComponent(id)}`,
					undefined,
					true,
					undefined,
					"GET",
				)
			).meetings,
		create: (input: CreateMeeting) =>
			call<{ meeting: Meeting; invite_token: string }>("meetings", input, true),
		invite: (id) =>
			call<{ invite_token: string }>(
				path(id, "/invite"),
				undefined,
				true,
				undefined,
				"GET",
			),
		rotateInvite: (id) =>
			call<{ invite_token: string }>(path(id, "/invite"), {}, true),
		preview: (id, token) =>
			call<MeetingPreview>(
				path(id, "/preview"),
				token ? { invite_token: token } : {},
				!token,
			),
		join: (id) => call<MeetingCredentials>(path(id, "/join"), {}, true),
		joinGuest: (id, token, name) =>
			call<MeetingCredentials>(
				path(id, "/guest"),
				{ invite_token: token, display_name: name },
				false,
			),
		session: (id, token) =>
			call<MeetingSession>(
				path(id, "/session"),
				undefined,
				false,
				token,
				"GET",
			),
		realtime: (id, token) =>
			call<MeetingRealtimeAccess>(path(id, "/realtime"), {}, false, token),
		send: (id, token, body) =>
			call<void>(path(id, "/messages"), { body }, false, token),
		leave: (id, token) =>
			call<void>(path(id, "/leave"), {}, false, token, "POST", {
				keepalive: true,
			}),
		admit: (id, participant, token) =>
			call<void>(
				path(id, `/participants/${encodeURIComponent(participant)}/admit`),
				{},
				false,
				token,
			),
		remove: (id, participant, token) =>
			call<void>(
				path(id, `/participants/${encodeURIComponent(participant)}/remove`),
				{},
				false,
				token,
			),
		end: (id, token) => call<void>(path(id, "/end"), {}, !token, token),
	};
}
