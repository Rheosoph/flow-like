import { afterAll, beforeAll, describe, expect, mock, test } from "bun:test";
import { act, createElement } from "react";
import type {
	IOrgApp,
	IOrgView,
	IPinnableApp,
	TalkReach,
} from "../../../state/backend-state/bot-team-state";
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
let pinnable: IPinnableApp[] = [];
const created: unknown[] = [];
const calls: string[] = [];
const record = (name: string) => ({
	...{ isPending: false },
	mutate: (args: unknown) => calls.push(`${name}:${JSON.stringify(args)}`),
});

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
		acceptLead: record("acceptLead"),
		declineLead: record("declineLead"),
		resignLead: record("resignLead"),
		openThread: record("openThread"),
		requestChat: record("requestChat"),
		setClassification: record("setClassification"),
		pinApp: record("pinApp"),
		approveAppPin: record("approveAppPin"),
		declineAppPin: record("declineAppPin"),
		unpinApp: record("unpinApp"),
	}),
	useSetTeamIcon: () => record("setTeamIcon"),
	usePinnableApps: () => ({ isLoading: false, isError: false, data: pinnable }),
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
async function show(me = "mara") {
	container = window.document.createElement("div") as unknown as HTMLElement;
	window.document.body.appendChild(container as never);
	root = createRoot(container);
	await act(async () =>
		root.render(
			createElement(OrgChartView, {
				me,
				bots: [],
				onOpenChat: (id: string) => opened.push(`chat:${id}`),
				onOpenConversation: (id: string) => opened.push(`room:${id}`),
				onOpenApp: (id: string, where: string) =>
					opened.push(`app:${where}:${id}`),
			}),
		),
	);
}
function button(label: string) {
	return [...container.querySelectorAll("button")].find((b) =>
		b.textContent?.includes(label),
	) as HTMLElement;
}

function labelled(label: string) {
	return container.querySelector(`[aria-label="${label}"]`) as HTMLElement;
}
function chartViewport() {
	return container.querySelector('[role="presentation"]') as HTMLElement;
}
function zoomLabel() {
	return container.querySelector('[aria-live="polite"]')?.textContent;
}
function wheel({
	ctrlKey = false,
	...deltas
}: { deltaX?: number; deltaY: number; ctrlKey?: boolean }) {
	const event = new window.WheelEvent("wheel", {
		bubbles: true,
		cancelable: true,
		...deltas,
	});
	// happy-dom's WheelEvent extends UIEvent, so it has no mouse fields.
	for (const [key, value] of Object.entries({
		ctrlKey,
		metaKey: false,
		clientX: 0,
		clientY: 0,
	})) {
		Object.defineProperty(event, key, { value });
	}
	return event as unknown as Event;
}
function pointer(type: string, clientX: number, clientY: number) {
	return new window.PointerEvent(type, {
		bubbles: true,
		pointerId: 1,
		pointerType: "mouse",
		button: 0,
		buttons: type === "pointerup" ? 0 : 1,
		clientX,
		clientY,
	}) as unknown as Event;
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

	test("the wheel pans the chart and ctrl-wheel or the buttons zoom it", async () => {
		await show();
		const viewport = chartViewport();
		const content = viewport.firstElementChild as HTMLElement;
		await act(async () => {
			viewport.dispatchEvent(wheel({ deltaX: 30, deltaY: 100 }));
		});
		expect(content.style.transform).toBe("translate(-30px, -100px) scale(1)");
		expect(zoomLabel()).toBe("100%");

		await act(async () => {
			viewport.dispatchEvent(wheel({ deltaY: -40, ctrlKey: true }));
		});
		expect(zoomLabel()).toBe("135%");

		await act(async () => labelled("Zoom out").click());
		expect(zoomLabel()).toBe("112%");
		await act(async () => root.unmount());
	});

	test("dragging empty space pans without deselecting", async () => {
		await show();
		await act(async () => button("Scout").click());
		const viewport = chartViewport();
		const content = viewport.firstElementChild as HTMLElement;
		await act(async () => {
			viewport.dispatchEvent(pointer("pointerdown", 100, 100));
			window.document.dispatchEvent(pointer("pointermove", 102, 101) as never);
			window.document.dispatchEvent(pointer("pointermove", 160, 140) as never);
			window.document.dispatchEvent(pointer("pointerup", 160, 140) as never);
			viewport.dispatchEvent(
				new window.MouseEvent("click", { bubbles: true }) as never,
			);
		});
		expect(content.style.transform).toBe("translate(60px, 40px) scale(1)");
		expect(container.textContent).toContain("Reporting line");
		await act(async () => root.unmount());
	});

	test("the + under a team creates a team inside it", async () => {
		created.length = 0;
		await show();
		await act(async () => labelled("New team inside Kestrel").click());
		const name = window.document.querySelector(
			"#org-new-team-name",
		) as unknown as HTMLInputElement;
		const setter = Object.getOwnPropertyDescriptor(
			window.HTMLInputElement.prototype,
			"value",
		)?.set;
		await act(async () => {
			setter?.call(name, "Pricing");
			name.dispatchEvent(new window.Event("input", { bubbles: true }) as never);
		});
		const create = [...window.document.querySelectorAll("button")].find((b) =>
			b.textContent?.includes("Create team"),
		) as unknown as HTMLElement;
		await act(async () => create.click());
		expect(created).toEqual([
			{ name: "Pricing", purpose: null, parent_id: "kestrel" },
		]);
		await act(async () => root.unmount());
	});

	test("only a team's admins get the + under it", async () => {
		await show("ines");
		expect(labelled("New team inside Kestrel")).toBeNull();
		await act(async () => root.unmount());
	});

	test("ways in and out sit beside the team, calendars mark the bots, and the switch folds the rails", async () => {
		const plain = org;
		org = {
			...plain,
			bots: plain.bots.map((bot) =>
				bot.id === "scout"
					? {
							...bot,
							inputs: {
								mail: { address: "scout@mail.test", received: 3, sent: 2 },
								webhooks: [{ name: "Stripe payouts", last_called_at: null }],
								own_calendar: true,
								connected_calendars: ["google"],
							},
						}
					: bot,
			),
		};
		await show();
		const team = () => labelled("Kestrel");
		expect(team().textContent).toContain("scout@mail.test");
		expect(team().textContent).toContain("Stripe payouts");
		expect(team().textContent).toContain("2 sent");
		expect(labelled("Own calendar: its events wake it")).toBeTruthy();
		expect(labelled("Connected calendars: Google Calendar")).toBeTruthy();

		await act(async () => button("Scout").click());
		expect(container.textContent).toContain("Ways in and out");
		expect(container.textContent).toContain("3 received · 2 sent");

		await act(async () =>
			(container.querySelector("#org-ins-outs") as HTMLElement).click(),
		);
		expect(team().textContent).not.toContain("Stripe payouts");
		expect(labelled("Own calendar: its events wake it")).toBeTruthy();
		await act(async () => root.unmount());
		org = plain;
	});

	test("a team shows its room's picture, and its people change it from the inspector", async () => {
		const plain = org;
		const picture = "https://img.test/kestrel.png";
		org = {
			...plain,
			teams: plain.teams.map((team) => ({ ...team, image_url: picture })),
		};
		await show();
		expect(container.querySelector(`img[src="${picture}"]`)).toBeTruthy();

		await act(async () => button("Kestrel").click());
		const change = labelled("Change team icon");
		expect(change).toBeTruthy();
		await act(async () => {
			change.dispatchEvent(
				new window.KeyboardEvent("keydown", {
					key: "Enter",
					bubbles: true,
				}) as never,
			);
		});
		const reset = [
			...window.document.querySelectorAll('[role="menuitem"]'),
		].find((item) => item.textContent?.includes("Use default icon"));
		await act(async () => (reset as unknown as HTMLElement).click());
		expect(calls).toContain(
			`setTeamIcon:${JSON.stringify({ teamId: "kestrel", roomId: "room-1", image: null })}`,
		);
		await act(async () => root.unmount());
		org = plain;
	});

	test("people outside a team see its icon but can't change it", async () => {
		await show("ines");
		await act(async () => button("Kestrel").click());
		expect(container.textContent).toContain("Top-level team");
		expect(labelled("Change team icon")).toBeNull();
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

	describe("apps", () => {
		const closeApp: IOrgApp = {
			id: "close",
			name: "Month-end close",
			app_type: "DataPipeline",
			can_open: true,
			can_request: false,
			reader_admin: true,
			owner_id: "mara",
			updated_at: "2026-11-20T10:00:00Z",
			stats: {
				runs: [1, 1, 1, 1, 1, 1, 1, 1, 1, 1, 1, 1, 1, 1],
				failed: [0, 0, 0, 0, 0, 0, 0, 0, 0, 1, 0, 0, 0, 0],
				last_run_at: null,
				triggers: ["cron"],
				schedule: "0 2 * * *",
				pages: 0,
			},
		};
		const deskApp: IOrgApp = {
			id: "desk",
			name: "Triage desk",
			description: "Where open cases live",
			app_type: "CustomInterface",
			can_open: false,
			can_request: true,
			reader_admin: false,
			owner_id: "jonas",
			updated_at: "2026-11-20T10:00:00Z",
			stats: null,
		};
		const appsOrg = (status: "active" | "pending" = "active"): IOrgView => ({
			...org,
			teams: org.teams.map((team) => ({
				...team,
				people: [
					{ user_id: "mara", role: "admin", status: "active" },
					{ user_id: "ines", role: "member", status: "active" },
					{ user_id: "olu", role: "member", status: "pending" },
				],
				apps: [
					{
						app_id: "close",
						status,
						pinned_by: "ines",
						pinned_at: "2026-11-20T10:00:00Z",
					},
				],
			})),
			bots: org.bots.map((bot) =>
				bot.id === "scout" ? { ...bot, app_ids: ["close", "desk"] } : bot,
			),
			apps: [closeApp, deskApp],
		});
		const exact = (label: string) =>
			[...window.document.querySelectorAll("button")].find(
				(b) => b.textContent?.trim() === label,
			) as unknown as HTMLElement;
		const withApps = async (
			run: () => Promise<void>,
			status?: "active" | "pending",
		) => {
			const plain = org;
			org = appsOrg(status);
			try {
				await run();
			} finally {
				await act(async () => root.unmount());
				org = plain;
			}
		};

		test("a team's apps hang under it, drawn by their type", () =>
			withApps(async () => {
				await show();
				const close = button("Month-end close");
				expect(close.textContent).toContain("Automation");
				expect(close.textContent).toContain("Daily at 02:00");
				expect(close.textContent).toContain("7 runs this week");
				const desk = button("Triage desk");
				expect(desk.textContent).toContain("via Scout");
				expect(desk.textContent).toContain("Request access");
				expect(labelled("Uses Month-end close, Triage desk")).toBeTruthy();
			}));

		test("selecting an app lights its bots and opens it from the inspector", () =>
			withApps(async () => {
				opened.length = 0;
				await show();
				await act(async () => button("Month-end close").click());
				expect(container.textContent).toContain("Runs, last 14 days");
				expect(container.textContent).toContain("13 ok, 1 failed");
				const scoutRow = [
					...labelled("Kestrel").querySelectorAll("button"),
				].find((b) => b.textContent?.startsWith("Scout"));
				expect(scoutRow?.className).toContain("bg-primary/10");
				await act(async () => exact("Open app").click());
				expect(opened).toContain("app:app:close");
			}));

		test("an app the reader can't open shows what it is and offers access", () =>
			withApps(async () => {
				opened.length = 0;
				await show();
				await act(async () => button("Triage desk").click());
				expect(container.textContent).toContain("Where open cases live");
				expect(container.textContent).not.toContain("Activity, last 14 days");
				await act(async () => exact("Request access").click());
				expect(opened).toContain("app:store:desk");
			}));

		test("the Apps switch and the filter hide the cards", () =>
			withApps(async () => {
				await show();
				await act(async () => exact("Agents").click());
				expect(button("Month-end close")).toBeUndefined();
				await act(async () => exact("Automations").click());
				expect(button("Month-end close")).toBeTruthy();
				await act(async () =>
					(container.querySelector("#org-apps") as HTMLElement).click(),
				);
				expect(button("Month-end close")).toBeUndefined();
			}));

		test("Pin app lists the reader's apps and an admin pins at once", () =>
			withApps(async () => {
				calls.length = 0;
				pinnable = [
					{
						id: "ledger",
						name: "Ledger DB",
						app_type: "DataFocus",
						reader_admin: true,
						pinned: null,
						pinned_elsewhere: [],
					},
					{
						id: "expenses",
						name: "Expenses",
						app_type: "Form",
						reader_admin: false,
						owner_id: "priya",
						pinned: null,
						pinned_elsewhere: [],
					},
				];
				await show();
				await act(async () => exact("Pin app").click());
				const body = window.document.body.textContent ?? "";
				expect(body).toContain("Pin an app to Kestrel");
				expect(body).toContain("Apps you administer");
				expect(body).toContain("Only its admins can pin it");
				await act(async () => exact("Pin").click());
				expect(calls).toContain(
					`pinApp:${JSON.stringify({ teamId: "kestrel", appId: "ledger" })}`,
				);
				pinnable = [];
			}));

		test("a member's pin waits and a team admin approves it", () =>
			withApps(async () => {
				calls.length = 0;
				await show("mara");
				const close = button("Month-end close");
				expect(close.textContent).toContain("Waiting for approval");
				await act(async () => close.click());
				await act(async () => exact("Approve for Kestrel").click());
				expect(calls).toContain(
					`approveAppPin:${JSON.stringify({ teamId: "kestrel", appId: "close" })}`,
				);
			}, "pending"));

		test("the team header stacks its people and lists them on tap", () =>
			withApps(async () => {
				await show("mara");
				const stack = labelled("People in Kestrel: 2");
				expect(stack).toBeTruthy();
				await act(async () => stack.click());
				const body = window.document.body.textContent ?? "";
				expect(body).toContain("2 people");
				expect(body).toContain("1 invited, not joined yet");
				expect(exact("Add people")).toBeTruthy();
			}));
	});

	describe("reach", () => {
		const reachOrg = (reach: TalkReach): IOrgView => {
			return {
				teams: [
					{
						id: "kestrel",
						name: "Kestrel",
						direct_asks: false,
						lead_bot_id: "boss",
						gated_bots: ["scout"],
						people: [
							{ user_id: "mara", role: "admin", status: "active" },
							{ user_id: "ines", role: "member", status: "active" },
						],
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
						classification: "confidential",
					},
				],
				invitations: [],
				reach: [
					{ bot_id: "boss", reach: "lead", conversation_id: "thread-1" },
					{
						bot_id: "scout",
						reach,
						conversation_id: reach === "chat" ? "conv-1" : null,
						retry_at: "2030-01-05T00:00:00Z",
					},
				],
			};
		};
		const cases: [TalkReach, string][] = [
			["team_room", "Team room"],
			["chat", "Chat"],
			["request", "Ask to chat"],
			["requested", "Waiting"],
			["declined", "Declined"],
			["classified", "Classified"],
			["unavailable", "Joining"],
		];

		for (const [reach, label] of cases) {
			test(`a ${reach} bot carries a "${label}" pill`, async () => {
				const full = org;
				org = reachOrg(reach);
				await show("ines");
				const row = labelled("Kestrel");
				expect(row.textContent).toContain(label);
				expect(row.textContent).toContain("Talk");
				await act(async () => root.unmount());
				org = full;
			});
		}

		test("the lead's Talk pill and the team's Talk button open the thread", async () => {
			const full = org;
			org = reachOrg("team_room");
			opened.length = 0;
			await show("ines");
			await act(async () => button("Talk to Kestrel").click());
			expect(calls.some((call) => call.startsWith("openThread:"))).toBe(true);
			await act(async () =>
				(
					container.querySelector(
						'button[aria-label^="Talk to Boss"]',
					) as HTMLElement
				).click(),
			);
			expect(opened).toContain("room:thread-1");
			await act(async () => root.unmount());
			org = full;
		});

		test("the gated bot shows the muted marker, the own bot no pill", async () => {
			const full = org;
			org = reachOrg("team_room");
			await show("ines");
			expect(
				container.querySelector(
					'[aria-label^="Doesn\'t answer in the team room"]',
				),
			).toBeTruthy();
			await act(async () => root.unmount());
			org = reachOrg("own");
			await show("mara");
			expect(labelled("Kestrel").textContent).not.toContain("Team room");
			await act(async () => root.unmount());
			org = full;
		});

		test("asking to chat sends the note", async () => {
			const full = org;
			org = reachOrg("request");
			calls.length = 0;
			await show("ines");
			await act(async () => button("Ask to chat").click());
			const note = window.document.querySelector(
				"textarea",
			) as unknown as HTMLTextAreaElement;
			expect(window.document.body.textContent).toContain("0/280");
			const setter = Object.getOwnPropertyDescriptor(
				window.HTMLTextAreaElement.prototype,
				"value",
			)?.set;
			await act(async () => {
				setter?.call(note, "Need a hand");
				note.dispatchEvent(
					new window.Event("input", { bubbles: true }) as never,
				);
			});
			const send = [...window.document.querySelectorAll("button")].find((b) =>
				b.textContent?.includes("Send request"),
			) as unknown as HTMLElement;
			await act(async () => send.click());
			expect(calls).toContain(
				'requestChat:{"botId":"scout","note":"Need a hand"}',
			);
			await act(async () => root.unmount());
			org = full;
		});

		test("the owner answers a lead offer and can step down", async () => {
			const full = org;
			const base = reachOrg("team_room");
			org = {
				...base,
				teams: [{ ...base.teams[0], lead_offer_bot_id: "scout" }],
			};
			calls.length = 0;
			await show("mara");
			await act(async () => button("Scout").click());
			expect(container.textContent).toContain(
				"Kestrel asks Scout to lead it. Everyone in Kestrel can then talk to it",
			);
			await act(async () => button("Accept").click());
			expect(calls).toContain(
				'acceptLead:{"teamId":"kestrel","botId":"scout"}',
			);
			await act(async () => button("Decline").click());
			expect(calls).toContain(
				'declineLead:{"teamId":"kestrel","botId":"scout"}',
			);
			await act(async () => button("Boss").click());
			expect(button("Step down as lead")).toBeTruthy();
			expect(container.textContent).not.toContain("asks Boss to lead");
			await act(async () => root.unmount());
			org = full;
		});

		test("an admin sees who a lead offer waits for, and can withdraw it", async () => {
			const full = org;
			const base = reachOrg("team_room");
			org = {
				...base,
				teams: [{ ...base.teams[0], lead_offer_bot_id: "scout" }],
			};
			await show("mara");
			await act(async () => button("Kestrel").click());
			expect(container.textContent).toContain("Waiting for mara to accept");
			await act(async () => root.unmount());
			org = full;
		});

		test("only admins of a top-level team classify people", async () => {
			const full = org;
			org = reachOrg("team_room");
			await show("mara");
			await act(async () => button("Kestrel").click());
			expect(
				container.querySelectorAll('[aria-label="Classification"]').length,
			).toBe(2);
			expect(container.textContent).toContain("Internal (default)");
			expect(container.textContent).toContain(
				"Your classification in Kestrel: internal",
			);
			await act(async () => root.unmount());

			await show("ines");
			await act(async () => button("Kestrel").click());
			expect(
				container.querySelector('[aria-label="Classification"]'),
			).toBeNull();
			expect(container.textContent).toContain(
				"Your classification in Kestrel: internal",
			);
			await act(async () => root.unmount());
			org = full;
		});
	});
});
