// @vitest-environment happy-dom
import { act, createElement } from "react";
import { createRoot } from "react-dom/client";
import { afterEach, beforeEach, expect, test, vi } from "vitest";

const mocks = vi.hoisted(() => ({
	auth: { isAuthenticated: true, user: { profile: { sub: "person" } } },
	providers: { google: {}, microsoft: {} } as Record<string, object>,
	links: [] as { id: string; provider_id: string; label: string }[],
	link: vi.fn(),
	unlink: vi.fn(),
	refresh: vi.fn(),
	invalidate: vi.fn(),
	oauth: vi.fn(),
}));
vi.mock("@flow-like/locales", () => ({
	useTranslation: () => ({
		t: (_key: string, fallback: string, values?: Record<string, string>) =>
			fallback.replace(/\{\{(\w+)\}\}/g, (_, key) => values?.[key] ?? key),
	}),
}));
vi.mock("react-oidc-context", () => ({ useAuth: () => mocks.auth }));
vi.mock("../../hooks/use-hub", () => ({
	useHub: () => ({ hub: { oauth_providers: mocks.providers } }),
}));
vi.mock("../../hooks/use-user-availability", () => ({
	useUserAvailability: () => ({
		data: { calendar_links: mocks.links },
		invalidate: mocks.invalidate,
	}),
}));
vi.mock("../../state/backend-state", () => {
	const userState = {
		linkCalendar: mocks.link,
		unlinkCalendar: mocks.unlink,
		refreshCalendars: mocks.refresh,
	};
	return { useBackend: () => ({ userState }) };
});
vi.mock("../oauth/oauth-execution-provider", () => ({
	useOAuthExecutionContext: () => ({ withOAuthCheckFromPrerun: mocks.oauth }),
}));
import { UserCalendarLinks } from "./user-calendar-links";

let container: HTMLDivElement;
let root: ReturnType<typeof createRoot>;
beforeEach(() => {
	vi.clearAllMocks();
	vi.stubGlobal("IS_REACT_ACT_ENVIRONMENT", true);
	mocks.auth.isAuthenticated = true;
	mocks.auth.user.profile.sub = "person";
	mocks.providers = { google: {}, microsoft: {} };
	mocks.links = [];
	mocks.oauth.mockImplementation(async (_requirements, execute) =>
		execute({ google: { refresh_token: "fl_oauth_account" } }),
	);
	container = document.createElement("div");
	root = createRoot(container);
});
afterEach(async () => {
	await act(async () => root.unmount());
	vi.unstubAllGlobals();
});
async function show() {
	await act(async () => root.render(createElement(UserCalendarLinks)));
}
function button(label: string) {
	const value = [...container.querySelectorAll("button")].find(
		(item) => item.textContent?.trim() === label,
	);
	if (!value) throw new Error(`Missing button ${label}`);
	return value;
}

test("links the person's calendar with a hub handle and free/busy scope", async () => {
	await show();
	await act(async () => button("Link Google Calendar").click());
	expect(mocks.oauth.mock.calls[0][0]).toEqual([
		{
			provider_id: "google",
			scopes: ["https://www.googleapis.com/auth/calendar.freebusy"],
		},
	]);
	expect(mocks.link).toHaveBeenCalledWith({
		provider_id: "google",
		connection_handle: "fl_oauth_account",
		label: "Google Calendar",
	});
	expect(mocks.refresh).not.toHaveBeenCalled();
	expect(mocks.invalidate).toHaveBeenCalledOnce();
});

test("never submits raw provider refresh credentials as a calendar link", async () => {
	mocks.oauth.mockImplementation(async (_requirements, execute) =>
		execute({ google: { refresh_token: "raw-provider-token" } }),
	);
	await show();
	await act(async () => button("Link Google Calendar").click());
	expect(mocks.link).not.toHaveBeenCalled();
	expect(container.querySelector('[role="alert"]')?.textContent).toContain(
		"Reconnect",
	);
});

test("an account change cancels linking after an unfinished OAuth check", async () => {
	let execute!: (tokens: unknown) => Promise<unknown>;
	let finish!: () => void;
	mocks.oauth.mockImplementation((_requirements, callback) => {
		execute = callback;
		return new Promise<void>((resolve) => {
			finish = resolve;
		});
	});
	await show();
	await act(async () => button("Link Google Calendar").click());
	mocks.auth.user.profile.sub = "different-person";
	await show();
	await act(async () => {
		await execute({ google: { refresh_token: "fl_oauth_old" } });
		finish();
	});
	expect(mocks.link).not.toHaveBeenCalled();
});

test("unlink removes the selected availability link", async () => {
	mocks.links = [{ id: "calendar_1", provider_id: "google", label: "Work" }];
	await show();
	await act(async () => button("Unlink").click());
	expect(mocks.unlink).toHaveBeenCalledWith("calendar_1");
	expect(mocks.invalidate).toHaveBeenCalledOnce();
});

test("starting OAuth explains the remaining link step", async () => {
	mocks.oauth.mockRejectedValue(new Error("OAuth authorization required"));
	await show();
	await act(async () => button("Link Google Calendar").click());
	expect(container.querySelector('[role="status"]')?.textContent).toContain(
		"select Link again",
	);
	expect(mocks.link).not.toHaveBeenCalled();
});
