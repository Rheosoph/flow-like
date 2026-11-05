import { afterAll, beforeAll, describe, expect, mock, test } from "bun:test";
import { QueryClient, QueryClientProvider } from "@tanstack/react-query";
import { act, createElement } from "react";
import {
	fakeI18n,
	installDomGlobals,
	restoreDomGlobals,
	settle,
	translate,
	domWindow as window,
} from "./testing/dom-harness";

const actualLocales = { ...(await import("@flow-like/locales")) };
mock.module("@flow-like/locales", () => ({
	...actualLocales,
	useTranslation: () => ({ t: translate, i18n: fakeI18n }),
}));

const invitable = [
	{
		principal_id: "bot_flux",
		kind: "bot",
		name: null,
		description: null,
		owner_id: "me",
		mode: "add_now",
		state: null,
	},
	{
		principal_id: "user_nameless",
		kind: "user",
		name: null,
		description: null,
		owner_id: null,
		mode: "invite",
		state: null,
	},
];

const actualInbox = { ...(await import("../../hooks/use-inbox")) };
mock.module("../../hooks/use-inbox", () => ({
	...actualInbox,
	useInvitable: () => ({
		data: invitable,
		isFetching: false,
		isError: false,
	}),
}));

let directoryResults: { user: Record<string, unknown> }[] = [];

const actualSearch = {
	...(await import("../../hooks/use-project-user-search")),
};
mock.module("../../hooks/use-project-user-search", () => ({
	...actualSearch,
	useProjectUserSearch: () => ({
		results: directoryResults,
		isSearchingDirectory: false,
		canSearchDirectory: false,
		directoryError: null,
	}),
}));

const actualBackend = { ...(await import("../../state/backend-state")) };
const userState = {
	lookupUser: async () => null,
	lookupUsers: async (ids: string[]) =>
		ids.map((id) => ({
			id,
			email: "mara.albers@example.com",
			created_at: "",
		})),
};
mock.module("../../state/backend-state", () => ({
	...actualBackend,
	useBackend: () => ({ userState }),
}));

const { createRoot } = await import("react-dom/client");
const { PrincipalPicker } = await import("./principal-picker");

beforeAll(() => installDomGlobals());
afterAll(() => {
	mock.module("@flow-like/locales", () => actualLocales);
	mock.module("../../hooks/use-inbox", () => actualInbox);
	mock.module("../../hooks/use-project-user-search", () => actualSearch);
	mock.module("../../state/backend-state", () => actualBackend);
	restoreDomGlobals();
});

describe("the principal picker names", () => {
	test("a built-in bot by its default and a nameless person by their email", async () => {
		const container = window.document.createElement("div");
		window.document.body.appendChild(container);
		const root = createRoot(container as unknown as Element);
		const picked: { label: string }[] = [];
		const source = {
			me: "me",
			bots: [
				{ id: "bot_flux", system: "flux", name: "", owner_id: "me" },
			] as never,
			recentPeopleIds: [],
			directory: (id: string) => ({ id, name: id }) as never,
			people: true,
		};
		await act(async () =>
			root.render(
				createElement(
					QueryClientProvider,
					{ client: new QueryClient() },
					createElement(PrincipalPicker, {
						source,
						selected: new Set<string>(),
						multiple: true,
						serverRule: {},
						onPick: (principal) => picked.push(principal),
					}),
				),
			),
		);
		await settle(60);
		const text = window.document.body.textContent ?? "";
		expect(text).toContain("Flux");
		expect(text).toContain("Mara Albers");
		expect(text).not.toContain("bot_flux");
		expect(text).not.toContain("user_nameless");
		const rows = [...window.document.body.querySelectorAll("li button")];
		await act(async () => (rows[1] as unknown as HTMLElement).click());
		expect(picked[0]?.label).toBe("Mara Albers");
		await act(async () => root.unmount());
	});
});

describe("the principal picker in ruled mode", () => {
	test("also offers directory people who share no app, minus members, and marks invited ones", async () => {
		directoryResults = [
			{ user: { id: "user_stranger", name: "Sam Stranger", created_at: "" } },
			{ user: { id: "user_member", name: "Mia Member", created_at: "" } },
			{ user: { id: "user_pending", name: "Pat Pending", created_at: "" } },
			{ user: { id: "user_nameless", name: "Dupe Invitable", created_at: "" } },
		];
		const container = window.document.createElement("div");
		window.document.body.appendChild(container);
		const root = createRoot(container as unknown as Element);
		const picked: { id: string; mode?: string }[] = [];
		const source = {
			me: "me",
			bots: [] as never,
			recentPeopleIds: [],
			directory: (id: string) => ({ id, name: id }) as never,
			people: true,
		};
		await act(async () =>
			root.render(
				createElement(
					QueryClientProvider,
					{ client: new QueryClient() },
					createElement(PrincipalPicker, {
						source,
						selected: new Set<string>(),
						multiple: true,
						serverRule: {
							conversationId: "c1",
							memberIds: ["user_member"],
							invitedIds: ["user_pending"],
						},
						onPick: (principal) => picked.push(principal),
					}),
				),
			),
		);
		await settle(60);
		const text = window.document.body.textContent ?? "";
		expect(text).toContain("Sam Stranger");
		expect(text).toContain("Pat Pending");
		expect(text).toContain("Already invited");
		expect(text).not.toContain("Mia Member");
		expect(text).not.toContain("Dupe Invitable");
		const stranger = [
			...window.document.body.querySelectorAll("li button"),
		].find((row) => row.textContent?.includes("Sam Stranger"));
		await act(async () => (stranger as unknown as HTMLElement).click());
		expect(picked[0]).toMatchObject({ id: "user_stranger", mode: "invite" });
		await act(async () => root.unmount());
		directoryResults = [];
	});
});
