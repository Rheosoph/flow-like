import { afterAll, beforeAll, describe, expect, mock, test } from "bun:test";
import { act, createElement } from "react";
import type { IOrgView } from "../../../state/backend-state/bot-team-state";
import {
	fakeI18n,
	installDomGlobals,
	restoreDomGlobals,
	translate,
	domWindow as window,
} from "../testing/dom-harness";

const actualLocales = { ...(await import("@flow-like/locales")) };
mock.module("@flow-like/locales", () => ({
	...actualLocales,
	useTranslation: () => ({ t: translate, i18n: fakeI18n }),
}));

const appearance = {} as never;
let org: IOrgView = {
	teams: [
		{
			id: "kestrel",
			name: "Kestrel",
			direct_asks: false,
			lead_bot_id: "boss",
			conversation_id: "room-1",
			people: [{ user_id: "mara", role: "admin", status: "active" }],
			bots: [
				{
					bot_id: "boss",
					owner_id: "mara",
					status: "active",
					direct_agreed: false,
				},
				{
					bot_id: "scout",
					owner_id: "mara",
					status: "active",
					direct_agreed: false,
				},
			],
			created_at: "",
			updated_at: "",
		},
	],
	bots: [
		{
			id: "boss",
			owner_id: "mara",
			name: "Boss",
			appearance,
			location: "server",
		},
		{
			id: "scout",
			owner_id: "mara",
			name: "Scout",
			appearance,
			location: "server",
		},
	],
	invitations: [],
};
const created: unknown[] = [];

const actualTeams = { ...(await import("../../../hooks/use-bot-teams")) };
const mutation = { mutate: () => {}, isPending: false };
mock.module("../../../hooks/use-bot-teams", () => ({
	...actualTeams,
	useOrg: () => ({ org, available: true }),
	useTeamBriefing: () => ({
		isLoading: false,
		data: { bot_id: "scout", text: "## YOUR TEAM\n- Boss leads" },
	}),
	useTeamActions: () => ({
		createTeam: {
			...mutation,
			mutate: (request: unknown) => created.push(request),
		},
		updateTeam: mutation,
		deleteTeam: mutation,
		invitePeople: mutation,
		removePerson: mutation,
		setPersonRole: mutation,
		addBot: mutation,
		removeBot: mutation,
		setDirectAgreement: mutation,
	}),
}));

const actualLookup = { ...(await import("../../../hooks/use-user-lookup")) };
mock.module("../../../hooks/use-user-lookup", () => ({
	...actualLookup,
	useUserIdentity: (id: string) => ({
		label: id,
		initials: "X",
		isPending: false,
	}),
}));
const actualBots = { ...(await import("../../../hooks/use-bots")) };
mock.module("../../../hooks/use-bots", () => ({
	...actualBots,
	useBotLabels: () => (bot: { name?: string | null }) => ({
		name: bot.name ?? "",
		description: "",
	}),
}));
const actualAvatar = { ...(await import("../../bots/bot-avatar")) };
mock.module("../../bots/bot-avatar", () => ({
	BotAvatar: () => createElement("span"),
}));
const actualUserAvatar = { ...(await import("../../ui/user-avatar")) };
mock.module("../../ui/user-avatar", () => ({
	UserAvatar: () => createElement("span"),
}));

const { createRoot } = await import("react-dom/client");
const { OrgChartView } = await import("./org-chart-view");

let container: HTMLElement;
let root: ReturnType<typeof createRoot>;

beforeAll(() => installDomGlobals());
afterAll(() => {
	mock.module("@flow-like/locales", () => actualLocales);
	mock.module("../../../hooks/use-bot-teams", () => actualTeams);
	mock.module("../../../hooks/use-user-lookup", () => actualLookup);
	mock.module("../../../hooks/use-bots", () => actualBots);
	mock.module("../../bots/bot-avatar", () => actualAvatar);
	mock.module("../../ui/user-avatar", () => actualUserAvatar);
	restoreDomGlobals();
});

const opened: string[] = [];
async function show() {
	container = window.document.createElement("div") as unknown as HTMLElement;
	window.document.body.appendChild(container as never);
	root = createRoot(container);
	await act(async () =>
		root.render(
			createElement(OrgChartView, {
				me: "mara",
				bots: [],
				onOpenChat: (id: string) => opened.push(`chat:${id}`),
				onOpenConversation: (id: string) => opened.push(`room:${id}`),
			}),
		),
	);
}
function button(label: string) {
	return [...container.querySelectorAll("button")].find((b) =>
		b.textContent?.includes(label),
	) as HTMLElement;
}

describe("org chart", () => {
	test("shows the team, its lead and members", async () => {
		await show();
		expect(container.textContent).toContain("Kestrel");
		expect(container.textContent).toContain("Boss");
		expect(container.textContent).toContain("Scout");
		await act(async () => root.unmount());
	});

	test("selecting a bot opens its inspector with the briefing", async () => {
		await show();
		await act(async () => button("Scout").click());
		expect(container.textContent).toContain("Reporting line");
		expect(container.textContent).toContain("YOUR TEAM");
		await act(async () => button("Open chat").click());
		expect(opened).toContain("chat:scout");
		await act(async () => root.unmount());
	});

	test("an empty org offers the first team", async () => {
		const full = org;
		org = { teams: [], bots: [], invitations: [] };
		await show();
		expect(button("Create your first team")).toBeTruthy();
		await act(async () => root.unmount());
		org = full;
	});
});
