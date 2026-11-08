import { act, createElement } from "react";
import { createRoot } from "react-dom/client";
// @vitest-environment happy-dom
import { afterEach, beforeEach, describe, expect, test, vi } from "vitest";
import type {
	IUserContextRequest,
	IUserContextResponse,
} from "../../state/backend-state/inbox-state";

const state = vi.hoisted(() => ({
	auth: { isAuthenticated: true, user: { profile: { sub: "person_1" } } },
	conversation: {
		kind: "group",
		encryption: "plain",
		members: [
			{ principal_id: "person_1" },
			{ principal_id: "person_2" },
			{ principal_id: "bot_1" },
		],
	},
	send: vi.fn(async (_request: unknown) => ({})),
	availability: vi.fn(async () => ({
		status: "busy",
		source: "calendar",
		until: null,
		preferences: { allow_mentions: true, allow_calls: true },
		calendar_links: [],
		free_busy: {
			window_start: 100,
			window_end: 500,
			busy: [{ start: 200, end: 300 }],
		},
	})),
	applyLocalStatus: vi.fn(),
	invalidateAvailability: vi.fn(async () => {}),
	location: vi.fn(async () => ({
		latitude: 52.521918,
		longitude: 13.410773,
		accuracy: 10,
	})),
}));

vi.mock("react-oidc-context", () => ({ useAuth: () => state.auth }));
vi.mock("../../hooks/use-inbox", () => ({
	useConversation: () => ({ data: state.conversation }),
	useSendInboxEntry: () => ({ mutateAsync: state.send }),
}));
vi.mock("../../hooks/use-user-lookup", () => ({
	useUserIdentity: (id: string) => ({ label: id }),
}));
vi.mock("../../hooks/use-user-availability", () => ({
	useUserAvailability: () => ({
		load: state.availability,
		applyLocalStatus: state.applyLocalStatus,
		invalidate: state.invalidateAvailability,
	}),
}));
vi.mock("@flow-like/locales", () => ({
	useTranslation: () => ({
		t: (_key: string, fallback: string, values?: Record<string, unknown>) =>
			fallback.replace(/\{\{(\w+)\}\}/g, (_, key) =>
				String(values?.[key] ?? key),
			),
	}),
}));
vi.mock("../../lib/device-bridge", async (original) => ({
	...(await original<typeof import("../../lib/device-bridge")>()),
	readLocationForUserContext: state.location,
}));

const { UserContextCard } = await import("./user-context-card");
const { clearUserContextSessions, hasUserContextGrant } = await import(
	"../../lib/bot-user-context"
);
const { useAuthStatusStore } = await import("../../state/backend-state");

let container: HTMLDivElement;
let root: ReturnType<typeof createRoot>;

function request(
	overrides: Partial<IUserContextRequest> = {},
): IUserContextRequest {
	return {
		id: "context_1",
		bot_id: "bot_1",
		subject_id: "person_1",
		conversation_id: "room_1",
		purpose: "Find a meeting place",
		fields: ["location"],
		precision: "approximate",
		audience: "group",
		audience_member_ids: ["person_1", "person_2", "bot_1"],
		created_at: Date.now() - 1000,
		expires_at: Date.now() + 300_000,
		...overrides,
	};
}

async function show(value = request()) {
	await act(async () => {
		root.render(
			createElement(UserContextCard, {
				key: value.id,
				context: { request: value },
			}),
		);
	});
}

async function settle() {
	await act(async () => {
		await new Promise((resolve) => setTimeout(resolve, 0));
	});
}

function button(text: string) {
	return [...container.querySelectorAll("button")].find(
		(item) => item.textContent?.trim() === text,
	);
}

async function click(text: string) {
	const target = button(text);
	expect(target).toBeDefined();
	await act(async () => {
		target?.click();
	});
	await settle();
}

function sentResponse(): IUserContextResponse {
	return (
		state.send.mock.calls[0][0] as {
			request: {
				envelope: { body: { user_context_response: IUserContextResponse } };
			};
		}
	).request.envelope.body.user_context_response;
}

beforeEach(() => {
	Object.assign(globalThis, { IS_REACT_ACT_ENVIRONMENT: true });
	Object.defineProperty(document, "visibilityState", {
		configurable: true,
		value: "visible",
	});
	state.auth.user.profile.sub = "person_1";
	state.auth.isAuthenticated = true;
	state.conversation.encryption = "plain";
	state.conversation.members = [
		{ principal_id: "person_1" },
		{ principal_id: "person_2" },
		{ principal_id: "bot_1" },
	];
	state.send.mockClear();
	state.location.mockClear();
	state.availability.mockClear();
	state.applyLocalStatus.mockClear();
	state.invalidateAvailability.mockClear();
	clearUserContextSessions();
	useAuthStatusStore.getState().setSignedIn(true);
	container = document.createElement("div");
	document.body.append(container);
	root = createRoot(container);
});

afterEach(async () => {
	await act(async () => root.unmount());
	container.remove();
	clearUserContextSessions();
});

describe("participant context cards", () => {
	test("an accepted local status change survives its card closing during send", async () => {
		Object.assign(state.conversation, { local: true });
		state.conversation.members = [
			{ principal_id: "local" },
			{ principal_id: "bot_1" },
		];
		let finish!: (value: object) => void;
		state.send.mockImplementationOnce(
			() =>
				new Promise((resolve) => {
					finish = resolve;
				}),
		);
		try {
			await show(
				request({
					subject_id: "local",
					fields: ["availability"],
					audience_member_ids: ["local", "bot_1"],
					status_change: { status: "focus", duration_minutes: 60 },
				}),
			);
			await click("Apply status change");
			expect(state.applyLocalStatus).not.toHaveBeenCalled();
			await act(async () => {
				root.unmount();
				finish({});
				await new Promise((resolve) => setTimeout(resolve, 0));
			});
			expect(state.applyLocalStatus).toHaveBeenCalledOnce();
			expect(state.applyLocalStatus.mock.calls[0][0]).toBe("focus");
		} finally {
			Reflect.deleteProperty(state.conversation, "local");
		}
	});
	test("status proposals require a single explicit confirmation with duration and notification impact", async () => {
		const value = request({
			fields: ["availability"],
			status_change: { status: "focus", duration_minutes: 60 },
		});
		await show(value);
		expect(container.textContent).toContain("60 minutes");
		expect(container.textContent).toContain(
			"Focus pauses routine notifications",
		);
		expect(button("Allow for this session")).toBeUndefined();
		expect(state.availability).not.toHaveBeenCalled();
		expect(state.send).not.toHaveBeenCalled();
		await click("Apply status change");
		const response = sentResponse();
		expect(response.values.availability).toEqual({
			status: "focus",
			source: "bot",
			until: response.captured_at + 60 * 60_000,
		});
		expect(response.decision).toBe("once");
		expect(hasUserContextGrant(value)).toBe(false);
		expect(state.invalidateAvailability).toHaveBeenCalledOnce();
	});
	test("declining a status change does not read availability or apply it", async () => {
		await show(
			request({
				fields: ["availability"],
				status_change: { status: "busy", duration_minutes: 15 },
			}),
		);
		await click("Decline");
		expect(state.availability).not.toHaveBeenCalled();
		expect(state.applyLocalStatus).not.toHaveBeenCalled();
		expect(state.invalidateAvailability).not.toHaveBeenCalled();
	});
	test("calendar data loads only when the person shares it", async () => {
		await show(request({ fields: ["free_busy"] }));
		expect(state.availability).not.toHaveBeenCalled();
		await click("Share once");
		expect(state.availability).toHaveBeenCalledOnce();
		const response = sentResponse();
		expect(response.values.free_busy?.busy).toEqual([{ start: 200, end: 300 }]);
	});
	test("shows purpose, device and group audience without reading a sensor", async () => {
		await show();
		expect(container.textContent).toContain("Find a meeting place");
		expect(container.textContent).toContain("everyone in this group");
		expect(container.textContent).toContain("Source: this device");
		expect(state.location).not.toHaveBeenCalled();
		expect(state.send).not.toHaveBeenCalled();
	});

	test("the owner cannot answer for another participant", async () => {
		state.auth.user.profile.sub = "bot_owner";
		await show();
		expect(button("Share once")).toBeUndefined();
		expect(button("Allow for this session")).toBeUndefined();
		expect(state.location).not.toHaveBeenCalled();
	});

	test("signed-out clients cannot use cached account claims to share", async () => {
		state.auth.isAuthenticated = false;
		await show();
		expect(button("Share once")).toBeUndefined();
		expect(state.location).not.toHaveBeenCalled();
	});

	test.each(["membership", "encryption"])(
		"%s changes while location is pending stop sharing",
		async (change) => {
			let finish: (value: {
				latitude: number;
				longitude: number;
				accuracy: number;
			}) => void = () => {};
			state.location.mockImplementationOnce(
				() =>
					new Promise((resolve) => {
						finish = resolve;
					}),
			);
			const value = request();
			await show(value);
			await click("Share once");
			expect(state.location).toHaveBeenCalledTimes(1);
			if (change === "membership")
				state.conversation.members.push({ principal_id: "new_person" });
			else state.conversation.encryption = "e2e";
			await show(value);
			await act(async () => {
				finish({ latitude: 1, longitude: 2, accuracy: 10 });
			});
			expect(state.send).not.toHaveBeenCalled();
		},
	);

	test.each(["membership", "encryption"])(
		"a changed %s prevents collection",
		async (change) => {
			if (change === "membership")
				state.conversation.members.push({ principal_id: "new_person" });
			else state.conversation.encryption = "e2e";
			await show();
			expect(button("Share once")?.disabled).toBe(true);
			await click("Share once");
			expect(state.location).not.toHaveBeenCalled();
			expect(state.send).not.toHaveBeenCalled();
		},
	);

	test("declining sends no context data and does not read a sensor", async () => {
		await show();
		await click("Decline");
		const sent = state.send.mock.calls[0]?.[0] as unknown as {
			request: {
				envelope: {
					body: { user_context_response: { decision: string; values: object } };
				};
			};
		};
		expect(sent.request.envelope.body.user_context_response).toMatchObject({
			decision: "deny",
			values: {},
		});
		expect(state.location).not.toHaveBeenCalled();
	});

	test("manual location shares the entered place with the requesting bot", async () => {
		await show();
		const select = container.querySelector("select") as HTMLSelectElement;
		await act(async () => {
			select.value = "manual";
			select.dispatchEvent(new Event("change", { bubbles: true }));
		});
		const input = container.querySelector("input") as HTMLInputElement;
		await act(async () => {
			Object.getOwnPropertyDescriptor(
				HTMLInputElement.prototype,
				"value",
			)?.set?.call(input, "Hamburg");
			input.dispatchEvent(new Event("input", { bubbles: true }));
		});
		await click("Share once");
		const sent = state.send.mock.calls[0]?.[0] as unknown as {
			request: {
				message_id: string;
				envelope: {
					body: {
						mentions: string[];
						user_context_response: { values: object };
					};
				};
			};
		};
		expect(sent.request.message_id).toBe("uctx_reply_context_1");
		expect(sent.request.envelope.body.mentions).toEqual(["bot_1"]);
		expect(sent.request.envelope.body.user_context_response.values).toEqual({
			location: { place: "Hamburg", precision: "approximate" },
		});
		expect(state.location).not.toHaveBeenCalled();
	});

	test("expired cards have no sharing action", async () => {
		await show(request({ expires_at: Date.now() - 1 }));
		expect(container.textContent).toContain("expired");
		expect(button("Share once")).toBeUndefined();
		expect(state.location).not.toHaveBeenCalled();
	});

	test("session consent can be revoked and logout clears it while unmounted", async () => {
		const value = request({ fields: ["timezone"] });
		await show(value);
		await click("Allow for this session");
		expect(hasUserContextGrant(value)).toBe(true);
		await click("Revoke session access");
		expect(hasUserContextGrant(value)).toBe(false);
		await show({ ...value, id: "context_2" });
		await click("Allow for this session");
		expect(hasUserContextGrant(value)).toBe(true);
		await act(async () => root.render(null));
		useAuthStatusStore.getState().setSignedIn(false);
		expect(hasUserContextGrant(value)).toBe(false);
	});

	test("logout during the send cannot recreate session consent", async () => {
		let finish: (value: object) => void = () => {};
		state.send.mockImplementationOnce(
			() =>
				new Promise((resolve) => {
					finish = resolve;
				}),
		);
		const value = request({ fields: ["timezone"] });
		await show(value);
		await click("Allow for this session");
		expect(state.send).toHaveBeenCalledTimes(1);
		await act(async () => {
			useAuthStatusStore.getState().setSignedIn(false);
			finish({});
		});
		expect(hasUserContextGrant(value)).toBe(false);
	});

	test("session sharing waits for a background card to become visible", async () => {
		const value = request({ fields: ["timezone"] });
		await show(value);
		await click("Allow for this session");
		state.send.mockClear();
		Object.defineProperty(document, "visibilityState", {
			configurable: true,
			value: "hidden",
		});
		await show({ ...value, id: "context_2" });
		expect(state.send).not.toHaveBeenCalled();
		await act(async () => {
			Object.defineProperty(document, "visibilityState", {
				configurable: true,
				value: "visible",
			});
			document.dispatchEvent(new Event("visibilitychange"));
		});
		await settle();
		expect(state.send).toHaveBeenCalledTimes(1);
		expect(state.send.mock.calls[0]?.[0]).toMatchObject({
			request: {
				envelope: { body: { user_context_response: { decision: "session" } } },
			},
		});
	});
});
