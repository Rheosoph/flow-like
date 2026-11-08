// @vitest-environment happy-dom
import { beforeEach, describe, expect, test, vi } from "vitest";
import type { AssistantSurfaceState } from "../state/assistant-surface";
import type { IUserContextRequest } from "../state/backend-state/inbox-state";
import {
	approximateUserLocation,
	clearUserContextSessions,
	collectUserContext,
	currentUserContextActivity,
	defaultUserContextInput,
	deniedUserContext,
	grantUserContextSession,
	hasUserContextGrant,
	revokeUserContextSession,
	setUserContextAccount,
} from "./bot-user-context";
import { DeviceError } from "./device-bridge";

function request(
	overrides: Partial<IUserContextRequest> = {},
): IUserContextRequest {
	return {
		id: "context_1",
		bot_id: "bot_1",
		subject_id: "person_1",
		conversation_id: "room_1",
		purpose: "Find nearby meeting places",
		fields: ["location"],
		precision: "approximate",
		audience: "group",
		audience_member_ids: ["bot_1", "person_1", "person_2"],
		created_at: Date.now() - 1000,
		expires_at: Date.now() + 300_000,
		...overrides,
	};
}

beforeEach(() => {
	clearUserContextSessions();
	Object.defineProperty(document, "visibilityState", {
		configurable: true,
		value: "visible",
	});
});

describe("context collection", () => {
	test("a status proposal cannot reuse session consent or exceed four hours", async () => {
		const value = request({
			fields: ["availability"],
			status_change: { status: "focus", duration_minutes: 60 },
		});
		grantUserContextSession(value, defaultUserContextInput());
		expect(hasUserContextGrant(value)).toBe(false);
		await expect(
			collectUserContext(value, defaultUserContextInput(), {
				subjectId: "person_1",
				decision: "session",
				activity: {},
			}),
		).rejects.toThrow();
		await expect(
			collectUserContext(
				{ ...value, status_change: { status: "focus", duration_minutes: 241 } },
				defaultUserContextInput(),
				{ subjectId: "person_1", decision: "once", activity: {} },
			),
		).rejects.toThrow();
		const load = vi.fn();
		const response = await collectUserContext(
			value,
			defaultUserContextInput(),
			{
				subjectId: "person_1",
				decision: "once",
				activity: {},
				availability: load,
			},
		);
		expect(load).not.toHaveBeenCalled();
		expect(response.values.availability?.until).toBe(
			response.captured_at + 60 * 60_000,
		);
	});
	test("account snapshots share only status and free/busy ranges, with one lazy read", async () => {
		const load = vi.fn(async () => ({
			status: "busy" as const,
			source: "calendar" as const,
			until: 2000,
			preferences: { allow_mentions: true, allow_calls: true },
			calendar_links: [
				{ id: "private-id", label: "Private calendar", provider_id: "google" },
			],
			free_busy: {
				window_start: 1000,
				window_end: 3000,
				busy: [{ start: 1000, end: 2000, title: "Private event" }],
			},
		}));
		const response = await collectUserContext(
			request({ fields: ["availability", "free_busy"] }),
			defaultUserContextInput(),
			{
				subjectId: "person_1",
				decision: "once",
				activity: {},
				availability: load,
			},
		);
		expect(load).toHaveBeenCalledOnce();
		expect(response.values).toEqual({
			availability: { status: "busy", source: "calendar", until: 2000 },
			free_busy: {
				window_start: 1000,
				window_end: 3000,
				busy: [{ start: 1000, end: 2000 }],
			},
		});
		expect(JSON.stringify(response)).not.toContain("Private");
	});
	test("declined snapshots identify the native client without collecting values", () => {
		const previous = Object.getOwnPropertyDescriptor(
			window,
			"__TAURI_INTERNALS__",
		);
		Object.defineProperty(window, "__TAURI_INTERNALS__", {
			configurable: true,
			value: {},
		});
		try {
			const result = deniedUserContext(request());
			expect(result.source.platform).toBe("native");
			expect(result.values).toEqual({});
		} finally {
			if (previous)
				Object.defineProperty(window, "__TAURI_INTERNALS__", previous);
			else Reflect.deleteProperty(window, "__TAURI_INTERNALS__");
		}
	});
	test("manual sharing sends the entered place without reading the device", async () => {
		const location = vi.fn();
		const result = await collectUserContext(
			request(),
			{
				...defaultUserContextInput(),
				locationMode: "manual",
				place: " Berlin ",
			},
			{ subjectId: "person_1", decision: "once", activity: {}, location },
		);
		expect(location).not.toHaveBeenCalled();
		expect(result.values).toEqual({
			location: { place: "Berlin", precision: "approximate" },
		});
		expect(result.source.session_id).toBeTruthy();
		expect(result.expires_at - result.captured_at).toBe(300_000);
	});

	test("the bot receives rounded coordinates and uncertainty, without other sensor values", async () => {
		const location = vi.fn(
			async () =>
				({
					latitude: 52.521918,
					longitude: 13.410773,
					accuracy: 10,
					altitude: 65,
					speed: 2,
					heading: 40,
				}) as never,
		);
		const result = await collectUserContext(
			request(),
			defaultUserContextInput(),
			{ subjectId: "person_1", decision: "once", activity: {}, location },
		);
		expect(result.values.location).toEqual({
			latitude: 52.52,
			longitude: 13.41,
			accuracy_m: 1500,
			precision: "approximate",
		});
		expect(result.values.location).not.toHaveProperty("altitude");
		expect(
			approximateUserLocation(0.0049, -0.0049, 3_000).accuracy_m,
		).toBeGreaterThan(3_000);
	});

	test("wrong subjects, background clients and expired requests never read location", async () => {
		const location = vi.fn();
		const options = {
			subjectId: "person_1",
			decision: "once" as const,
			activity: {},
			location,
		};
		await expect(
			collectUserContext(request(), defaultUserContextInput(), {
				...options,
				subjectId: "other",
			}),
		).rejects.toThrow("Only the requested person");
		await expect(
			collectUserContext(
				request({ expires_at: Date.now() - 1 }),
				defaultUserContextInput(),
				options,
			),
		).rejects.toThrow("expired");
		Object.defineProperty(document, "visibilityState", {
			configurable: true,
			value: "hidden",
		});
		await expect(
			collectUserContext(request(), defaultUserContextInput(), options),
		).rejects.toThrow("Keep this app open");
		expect(location).not.toHaveBeenCalled();
	});

	test("an OS refusal returns unavailable without fabricated coordinates", async () => {
		const location = vi.fn(async () => {
			throw new DeviceError("permission_denied", "No permission");
		});
		const result = await collectUserContext(
			request(),
			defaultUserContextInput(),
			{ subjectId: "person_1", decision: "once", activity: {}, location },
		);
		expect(result.values).toEqual({});
		expect(result.unavailable).toEqual({ location: "permission_denied" });
	});

	test("only the requested fields are collected and current context contains no document contents", async () => {
		const activity = currentUserContextActivity({
			boardSurface: {
				appId: "app",
				boardId: "board",
				selectedNodeIds: ["node"],
				board: { secret: "never share" },
			},
		} as unknown as AssistantSurfaceState);
		const location = vi.fn();
		const result = await collectUserContext(
			request({ fields: ["locale", "current_context"] }),
			{ ...defaultUserContextInput(), language: "de-DE", units: "metric" },
			{ subjectId: "person_1", decision: "once", activity, location },
		);
		expect(result.values).toEqual({
			locale: { language: "de-DE", units: "metric" },
			current_context: {
				kind: "board",
				app_id: "app",
				board_id: "board",
				selected_ids: ["node"],
			},
		});
		expect(location).not.toHaveBeenCalled();
	});

	test("cancelled collection cannot return a sensor result", async () => {
		const controller = new AbortController();
		const location = vi.fn(async () => {
			controller.abort();
			return { latitude: 1, longitude: 2, accuracy: 10 } as never;
		});
		await expect(
			collectUserContext(request(), defaultUserContextInput(), {
				subjectId: "person_1",
				decision: "once",
				activity: {},
				signal: controller.signal,
				location,
			}),
		).rejects.toThrow("cancelled");
	});

	test.each([
		{ compact: true, touch: false, fine: true, viewport: "compact" },
		{ compact: false, touch: true, fine: false, viewport: "wide" },
	])(
		"device context reports observed hints for $viewport views",
		async ({ compact, touch, fine, viewport }) => {
			const matchMedia = vi.spyOn(window, "matchMedia").mockImplementation(
				(query) =>
					({
						matches: query === "(max-width: 767px)" ? compact : fine,
					}) as MediaQueryList,
			);
			const previousTouch = Object.getOwnPropertyDescriptor(
				navigator,
				"maxTouchPoints",
			);
			Object.defineProperty(navigator, "maxTouchPoints", {
				configurable: true,
				value: touch ? 5 : 0,
			});
			try {
				const result = await collectUserContext(
					request({ fields: ["device"] }),
					defaultUserContextInput(),
					{ subjectId: "person_1", decision: "once", activity: {} },
				);
				expect(result.values.device).toEqual({
					viewport,
					touch,
					fine_pointer: fine,
				});
				expect(result.values.device).not.toHaveProperty("keyboard");
				expect(result.values.device).not.toHaveProperty("form_factor");
			} finally {
				matchMedia.mockRestore();
				if (previousTouch)
					Object.defineProperty(navigator, "maxTouchPoints", previousTouch);
				else Reflect.deleteProperty(navigator, "maxTouchPoints");
			}
		},
	);
});

describe("session consent", () => {
	test("grants bind bot, subject, audience, fields, precision and purpose", () => {
		const original = request();
		grantUserContextSession(original, defaultUserContextInput(), 1_000);
		expect(hasUserContextGrant(original, 2_000)).toBe(true);
		for (const change of [
			{ bot_id: "other" },
			{ subject_id: "other" },
			{ conversation_id: "other" },
			{ audience: "direct" as const },
			{ audience_member_ids: ["bot_1", "person_1"] },
			{ purpose: "Different purpose" },
			{ fields: ["timezone" as const] },
			{ precision: "precise" as const },
		])
			expect(hasUserContextGrant({ ...original, ...change }, 2_000)).toBe(
				false,
			);
		expect(hasUserContextGrant(original, 1_801_000)).toBe(false);
	});

	test("revocation and account switching clear earlier grants", () => {
		const original = request();
		setUserContextAccount("person_1");
		grantUserContextSession(original, defaultUserContextInput());
		revokeUserContextSession(original);
		expect(hasUserContextGrant(original)).toBe(false);
		grantUserContextSession(original, defaultUserContextInput());
		setUserContextAccount(null);
		setUserContextAccount("person_1");
		expect(hasUserContextGrant(original)).toBe(false);
	});
});
