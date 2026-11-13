import {
	afterAll,
	beforeAll,
	beforeEach,
	describe,
	expect,
	mock,
	test,
} from "bun:test";
import { act, createElement } from "react";
import type {
	BotApprovalDecision,
	IBot,
	IBotApprovalItemDecision,
	IBotToolGrant,
} from "../../state/backend-state/bot-state";
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

const { createRoot } = await import("react-dom/client");
const { QueryClient, QueryClientProvider } = await import(
	"@tanstack/react-query"
);
const { useBackendStore } = await import("../../state/backend-state");
const { ApprovalCard, ChatApprovalCard } = await import("./approval-card");
const { transcriptOf } = await import("../../state/global-chat/bot-history");
const { BotToolGrants } = await import("./bot-tool-grants");
const { NeedsYouSection } = await import("./needs-you-section");

const answers: Array<[string, string, string, BotApprovalDecision]> = [];
const answerItems: Array<IBotApprovalItemDecision[] | undefined> = [];
let refusal: { code: string; message: string } | undefined;
const events: string[] = [];
const secrets: Array<[string, string, { value: string; description: string }]> =
	[];
let failSecret = false;
let grants: IBotToolGrant[] = [];
const revoked: Array<[string, string, string]> = [];
let failAnswer = false;

const botState = {
	getBot: async () => ({ id: "bot_a", name: "Scout", system: null }),
	answerApproval: async (
		botId: string,
		turnId: string,
		approvalId: string,
		decision: BotApprovalDecision,
		items?: IBotApprovalItemDecision[],
	) => {
		if (failAnswer) throw new Error("offline");
		if (refusal) throw Object.assign(new Error(refusal.message), refusal);
		events.push("answer");
		answers.push([botId, turnId, approvalId, decision]);
		answerItems.push(items);
	},
	upsertBotSecret: async (
		botId: string,
		name: string,
		request: { value: string; description: string },
	) => {
		if (failSecret) throw new Error("store offline");
		events.push("store");
		secrets.push([botId, name, request]);
		return { name };
	},
	listToolGrants: async () => grants,
	deleteToolGrant: async (botId: string, tool: string, scope: string) => {
		revoked.push([botId, tool, scope]);
		grants = grants.filter((g) => !(g.tool === tool && g.scope === scope));
	},
};

const people: Record<string, string> = { "user-anna": "Anna Schmidt" };
const lookup = (id: string) => ({
	id,
	name: people[id],
	created_at: "2026-01-01T00:00:00Z",
});
const userState = {
	lookupUser: async (id: string) => lookup(id),
	lookupUsers: async (ids: string[]) => ids.map(lookup),
};

const previousBackend = useBackendStore.getState().backend;
let client: InstanceType<typeof QueryClient>;
let container: HTMLElement;
let root: ReturnType<typeof createRoot>;

const approval = {
	id: "ap-1",
	turnId: "turn_1",
	tool: "shell_run",
	summary: "Run a program called git",
	scopes: ["command:git"],
};

async function show(view: ReturnType<typeof createElement>) {
	await act(async () =>
		root.render(createElement(QueryClientProvider, { client }, view)),
	);
	await settle();
}

function button(label: string): HTMLButtonElement | undefined {
	return [...container.querySelectorAll("button")].find(
		(candidate) =>
			candidate.textContent?.trim() === label ||
			candidate.getAttribute("aria-label") === label,
	) as HTMLButtonElement | undefined;
}

async function click(element: HTMLButtonElement | undefined) {
	expect(element).toBeDefined();
	await act(async () => {
		element?.click();
	});
	await settle();
}

beforeAll(() => {
	installDomGlobals();
	useBackendStore.getState().setBackend({ botState, userState } as never);
	container = window.document.createElement("div") as unknown as HTMLElement;
	window.document.body.appendChild(container as never);
	root = createRoot(container);
});

afterAll(async () => {
	await act(async () => root.unmount());
	mock.module("@flow-like/locales", () => actualLocales);
	useBackendStore.setState({ backend: previousBackend });
	restoreDomGlobals();
});

beforeEach(async () => {
	await act(async () => root.render(null));
	answers.length = 0;
	answerItems.length = 0;
	refusal = undefined;
	events.length = 0;
	secrets.length = 0;
	failSecret = false;
	revoked.length = 0;
	failAnswer = false;
	grants = [];
	client = new QueryClient({ defaultOptions: { queries: { retry: false } } });
});

async function type(input: HTMLInputElement, value: string) {
	const setter = Object.getOwnPropertyDescriptor(
		Object.getPrototypeOf(input),
		"value",
	)?.set;
	await act(async () => {
		setter?.call(input, value);
		input.dispatchEvent(new window.Event("input", { bubbles: true }) as never);
	});
	await settle();
}

describe("ApprovalCard room invitation", () => {
	const inviteCard = (id = "ap-i") =>
		createElement(ApprovalCard, {
			key: id,
			approval: {
				id,
				turnId: "turn_i",
				tool: "room_invite",
				summary: "The bot wants to invite ana@example.org to this group chat.",
				scopes: [],
				room: "Launch crew",
				invite: {
					person: "ana@example.org",
					reason: "She owns the budget sheet we are discussing.",
				},
			} as never,
			botId: "bot_a",
			botName: "Scout",
			readOnly: false,
		});

	test("names the person, the room and why, and offers Invite or Deny only", async () => {
		await show(inviteCard());
		const text = container.textContent ?? "";
		expect(text).toContain("Scout wants to invite ana@example.org");
		expect(text).toContain("She owns the budget sheet we are discussing.");
		expect(text).toContain("Launch crew");
		expect(button("Invite")).toBeDefined();
		expect(button("Deny")).toBeDefined();
		expect(button("Always allow")).toBeUndefined();
		expect(button("Allow once")).toBeUndefined();
	});

	test("Invite answers once and Deny answers deny", async () => {
		await show(inviteCard());
		await click(button("Invite"));
		await show(inviteCard("ap-i2"));
		await click(button("Deny"));
		expect(answers).toEqual([
			["bot_a", "turn_i", "ap-i", "once"],
			["bot_a", "turn_i", "ap-i2", "deny"],
		]);
	});
});

describe("ApprovalCard secret request", () => {
	const secretCard = (replace?: boolean) =>
		createElement(ApprovalCard, {
			approval: {
				id: "ap-s",
				turnId: "turn_s",
				tool: "secret_request",
				summary: "Ask for a secret",
				scopes: [],
				secret: {
					name: "GITHUB_TOKEN",
					description: "Lets me open pull requests",
					...(replace ? { replace } : {}),
				},
			} as never,
			botId: "bot_a",
			botName: "Scout",
			readOnly: false,
		});
	const input = () =>
		container.querySelector("input") as unknown as HTMLInputElement;

	test("shows the name, the reason and a masked input instead of the allow buttons", async () => {
		await show(secretCard());
		const text = container.textContent ?? "";
		expect(text).toContain("Scout needs a secret");
		expect(text).toContain("GITHUB_TOKEN");
		expect(text).toContain("Lets me open pull requests");
		expect(input().type).toBe("password");
		expect(input().getAttribute("autocomplete")).toBe("new-password");
		expect(button("Allow once")).toBeUndefined();
		expect(button("Always allow")).toBeUndefined();
		expect(button("Save secret")).toBeDefined();
		expect(button("Decline")).toBeDefined();
		expect(text).not.toContain("rejected");
	});

	test("a replacement says the stored value was rejected", async () => {
		await show(secretCard(true));
		expect(container.textContent).toContain("The stored value was rejected");
	});

	test("Save is disabled until a value is typed", async () => {
		await show(secretCard());
		expect(button("Save secret")?.disabled).toBe(true);
		await type(input(), "s3cret");
		expect(button("Save secret")?.disabled).toBe(false);
	});

	test("stores the value first, then answers once, and never shows the value", async () => {
		await show(secretCard());
		await type(input(), "s3cret-value");
		await click(button("Save secret"));
		expect(secrets).toEqual([
			[
				"bot_a",
				"GITHUB_TOKEN",
				{ value: "s3cret-value", description: "Lets me open pull requests" },
			],
		]);
		expect(answers).toEqual([["bot_a", "turn_s", "ap-s", "once"]]);
		expect(events).toEqual(["store", "answer"]);
		expect(container.textContent).toContain("Secret saved");
		expect(container.textContent).not.toContain("s3cret-value");
		expect(container.querySelector("input")).toBeNull();
	});

	test("a failed store shows the error and does not answer", async () => {
		failSecret = true;
		await show(secretCard());
		await type(input(), "s3cret-value");
		await click(button("Save secret"));
		expect(answers).toEqual([]);
		expect(container.querySelector('[role="alert"]')).not.toBeNull();
		expect(button("Save secret")).toBeDefined();
	});

	test("Decline answers deny without storing anything", async () => {
		await show(secretCard());
		await type(input(), "typed-then-declined");
		await click(button("Decline"));
		expect(secrets).toEqual([]);
		expect(answers).toEqual([["bot_a", "turn_s", "ap-s", "deny"]]);
		expect(container.textContent).toContain("Declined");
	});

	test("a value over the byte limit cannot be saved", async () => {
		await show(secretCard());
		await type(input(), "é".repeat(8 * 1024 + 1));
		expect(button("Save secret")?.disabled).toBe(true);
		expect(container.textContent).toContain("Too large");
	});
});

describe("ApprovalCard", () => {
	const card = (
		overrides: Partial<typeof approval> & { answer?: "once" } = {},
	) =>
		createElement(ApprovalCard, {
			approval: { ...approval, ...overrides } as never,
			botId: "bot_a",
			botName: "Scout",
			readOnly: false,
		});

	test("configuration cards show every proposed value and cannot offer Always allow", async () => {
		const instructions = `${"A complete instruction.\n".repeat(100)}Final instruction.`;
		await show(
			card({
				tool: "bot_update_configuration",
				configurationChanges: {
					name: "Atlas",
					instructions,
					capabilities: ["internet"],
				},
				scopes: ["*"],
			} as never),
		);
		expect(
			container.querySelector("[data-slot='bot-configuration-changes']")
				?.textContent,
		).toContain(instructions);
		expect(container.textContent).toContain("Atlas");
		expect(container.textContent).toContain("Internet");
		expect(container.querySelector("header")?.textContent).toContain(
			"Scout wants to update its settings",
		);
		expect(container.querySelector("header")?.textContent).not.toContain(
			"bot_update_configuration",
		);
		expect(button("Always allow")).toBeUndefined();
		expect(button("Allow once")).toBeUndefined();
		expect(button("Reject changes")).toBeDefined();
		await click(button("Approve changes"));
		expect(answers).toEqual([["bot_a", approval.turnId, approval.id, "once"]]);
		expect(
			container.querySelector("[data-slot='approval-answer']")?.textContent,
		).toBe("Changes approved");
		expect(button("Approve changes")).toBeUndefined();
	});

	test("says what the bot wants, with the tool and the scope in plain words", async () => {
		await show(card());
		const text = container.textContent ?? "";
		expect(text).toContain("Scout wants to run a program called git");
		expect(text).toContain("shell_run");
		expect(text).toContain("Run `git` on the command line");
		expect(button("Allow once")).toBeDefined();
		expect(button("Always allow")).toBeDefined();
		expect(button("Deny")).toBeDefined();
	});

	test("a message to a person names them, and the core's 'The bot wants to' is not repeated", async () => {
		await show(
			card({
				tool: "message_person",
				summary:
					'The bot wants to message a person you chat with: "Does Friday work?"',
				scopes: ["person:user-anna"],
			}),
		);
		await settle(60);
		const text = container.textContent ?? "";
		expect(text).toContain(
			'Scout wants to message a person you chat with: "Does Friday work?"',
		);
		expect(text).not.toContain("the bot wants to");
		expect(text).toContain("Message Anna Schmidt");
	});

	test("names who triggered the turn and the room", async () => {
		await show(card({ requesterName: "Mia", room: "Launch crew" } as never));
		expect(container.textContent).toContain("Triggered by Mia in Launch crew");
		await show(card({ requesterName: "Mia" } as never));
		expect(container.textContent).toContain("Triggered by Mia");
		expect(container.textContent).not.toContain("Launch crew");
		await show(card());
		expect(container.textContent).not.toContain("Triggered by");
	});

	test.each([
		["Allow once", "once", "Allowed once"],
		["Always allow", "always", "Always allowed"],
		["Deny", "deny", "Denied"],
	] as const)(
		"%s answers the parked turn and shows the decision",
		async (label, decision, shown) => {
			await show(card());
			await click(button(label));
			expect(answers).toEqual([["bot_a", "turn_1", "ap-1", decision]]);
			expect(container.textContent).toContain(shown);
			expect(button("Allow once")).toBeUndefined();
		},
	);

	test("an answered card shows the decision and offers no buttons", async () => {
		await show(card({ answer: "once" }));
		expect(
			container.querySelector("[data-slot='approval-answer']")?.textContent,
		).toContain("Allowed once");
		expect(container.querySelectorAll("button")).toHaveLength(0);
	});

	test("a call that is never remembered offers no Always allow", async () => {
		await show(card({ scopes: [] }));
		expect(button("Always allow")).toBeUndefined();
		expect(container.textContent).toContain("Always asks first");
	});

	test("a failed answer keeps the buttons and says so", async () => {
		failAnswer = true;
		await show(card());
		await click(button("Allow once"));
		expect(container.querySelector("[role='alert']")).not.toBeNull();
		expect(button("Allow once")).toBeDefined();
	});
});

const DAY = 24 * 60 * 60 * 1000;

function radioGroup(label: string): HTMLElement | undefined {
	return [...container.querySelectorAll("[role='radiogroup']")].find(
		(group) => group.getAttribute("aria-label") === label,
	) as HTMLElement | undefined;
}

function radio(group: string, value: BotApprovalDecision) {
	return radioGroup(group)?.querySelector(`[role='radio'][value='${value}']`) as
		| HTMLButtonElement
		| undefined;
}

const footerButtons = () =>
	[...(container.querySelector("footer")?.querySelectorAll("button") ?? [])]
		.map((element) => element.textContent?.trim())
		.filter(Boolean);

/** The eyebrow: an `output`, so a screen reader hears the answer. */
const status = () =>
	container.querySelector("header output[aria-live='polite']");
const statusText = () => status()?.textContent ?? undefined;

describe("ApprovalCard for one call", () => {
	const card = (overrides: Record<string, unknown> = {}) =>
		createElement(ApprovalCard, {
			approval: { ...approval, postedAt: Date.now(), ...overrides } as never,
			botId: "bot_a",
			botName: "Scout",
			readOnly: false,
		});

	test("looks as before batches: no list, the three answers", async () => {
		await show(card());
		expect(
			container.querySelector("[data-slot='approval-batch-list']"),
		).toBeNull();
		expect(container.querySelector("section")?.getAttribute("aria-label")).toBe(
			"Approval request",
		);
		expect(footerButtons()).toEqual(["Deny", "Allow once", "Always allow"]);
		expect(container.textContent).not.toContain("Asked again");
	});

	test("asked again after web content: says why and offers no Always allow", async () => {
		await show(
			card({
				tool: "web_request",
				summary: "The bot wants to send a GET request to cisa.gov/alerts.",
				scopes: ["web:cisa.gov:read"],
				afterWebContent: true,
			}),
		);
		const text = container.textContent ?? "";
		expect(text).toContain("Read from cisa.gov");
		expect(text).toContain("Asked again");
		expect(text).toContain("Asked again because this turn read web pages.");
		expect(footerButtons()).toEqual(["Deny", "Allow once"]);
	});

	test("unanswered for more than seven days it reads as expired, without buttons", async () => {
		await show(card({ postedAt: Date.now() - 8 * DAY }));
		expect(
			container.querySelector("[data-slot='approval-closure']")?.textContent,
		).toBe("Expired");
		expect(container.textContent).toContain(
			"Nobody answered within 7 days, so Scout stopped.",
		);
		expect(container.querySelectorAll("button")).toHaveLength(0);
	});

	test("a refusal as expired turns it into the expired card", async () => {
		refusal = { code: "BOT_APPROVAL_EXPIRED", message: "expired" };
		await show(card());
		await click(button("Allow once"));
		expect(
			container.querySelector("[data-slot='approval-closure']")?.textContent,
		).toBe("Expired");
		expect(container.querySelector("[role='alert']")).toBeNull();
	});

	test("answered on another device: says so and keeps the buttons disabled", async () => {
		refusal = {
			code: "BOT_TURN_NOT_WAITING",
			message: "[BOT_TURN_NOT_WAITING] Turn is queued",
		};
		await show(card());
		await click(button("Allow once"));
		expect(container.textContent).toContain("Answered on another device");
		expect(container.querySelector("[role='alert']")).toBeNull();
		expect(button("Allow once")?.disabled).toBe(true);
		expect(button("Deny")?.disabled).toBe(true);
	});

	test("a cancelled task closes the card", async () => {
		await show(card({ cancelled: true }));
		expect(
			container.querySelector("[data-slot='approval-closure']")?.textContent,
		).toBe("Task cancelled");
		expect(container.querySelectorAll("button")).toHaveLength(0);
	});
});

describe("ApprovalCard with a batch of calls", () => {
	const read = (
		hash: string,
		page: number,
		extra: Record<string, unknown> = {},
	) => ({
		argsHash: hash,
		tool: "web_request",
		summary: `The bot wants to send a GET request to cisa.gov/alerts?p=${page}.`,
		scopes: ["web:cisa.gov:read"],
		calls: 1,
		afterWebContent: false,
		...extra,
	});
	const mail = {
		argsHash: "m1",
		tool: "email_send",
		summary: "The bot wants to send the weekly digest to team@acme.com.",
		scopes: ["email:acme.com"],
		calls: 1,
		afterWebContent: false,
	};
	const secretItem = {
		argsHash: "s1",
		tool: "secret_request",
		summary: "Ask for API_TOKEN",
		scopes: [],
		calls: 1,
		afterWebContent: false,
		secret: { name: "API_TOKEN", description: "For the weather API" },
	};
	const batch = (items: unknown[], extra: Record<string, unknown> = {}) => ({
		id: "ap-b",
		turnId: "turn_b",
		tool: "approval_batch",
		summary: "The bot wants to make some calls",
		scopes: [],
		items,
		postedAt: Date.now(),
		...extra,
	});
	const three = (extra: Record<string, unknown> = {}) =>
		batch([read("h1", 1), read("h2", 2), mail], extra);
	const thirty = () =>
		batch([
			...Array.from({ length: 12 }, (_, page) => read(`c${page}`, page)),
			...Array.from({ length: 3 }, (_, page) =>
				read(`n${page}`, page, {
					summary: `The bot wants to send a GET request to nvd.nist.gov/vuln?p=${page}.`,
					scopes: ["web:nvd.nist.gov:read"],
					calls: 3,
				}),
			),
			read("g1", 0, {
				summary:
					"The bot wants to send a POST request to api.github.com/gists.",
				scopes: ["web:api.github.com:write"],
				afterWebContent: true,
			}),
			{
				argsHash: "x1",
				tool: "shell_run",
				summary: "Run curl",
				scopes: ["command:curl"],
				calls: 4,
				afterWebContent: false,
			},
			mail,
			{
				...mail,
				argsHash: "m2",
				summary: "Send the alert digest to ops@acme.com",
			},
			{
				argsHash: "b1",
				tool: "browser_open",
				summary: "Open news.ycombinator.com",
				scopes: ["browser:news.ycombinator.com"],
				calls: 1,
				afterWebContent: false,
			},
			read("f1", 0, {
				tool: "file_save",
				summary: "Save example.org/report.pdf",
				scopes: ["web:example.org:read"],
			}),
		]);
	const card = (value: unknown) =>
		createElement(ApprovalCard, {
			approval: value as never,
			botId: "bot_a",
			botName: "Scout",
			readOnly: false,
		});
	const groups = () =>
		[...container.querySelectorAll("[data-slot='approval-group']")].map(
			(group) => group.textContent ?? "",
		);
	const callRows = () =>
		container.querySelectorAll("[data-slot='approval-group'] li");
	const cisa = "Answer for 2 calls to Read from cisa.gov";
	const acme = "Answer for 1 call to Send email to acme.com";
	const second = "Answer for Send a GET request to cisa.gov/alerts?p=2";

	test("three calls: one card, grouped by tool and site, every call on its own line", async () => {
		await show(card(three()));
		expect(container.querySelector("section")?.getAttribute("aria-label")).toBe(
			"Approval request, 3 calls",
		);
		expect(statusText()).toBe("Approval request · 3 calls");
		expect(container.querySelector("header h3")?.textContent).toBe(
			"Scout wants to make 3 calls",
		);
		const [web, email] = groups();
		expect(groups()).toHaveLength(2);
		expect(web).toContain("web_request");
		expect(web).toContain("Read from cisa.gov");
		expect(web).toContain("2 calls");
		expect(web).toContain("GET /alerts?p=1");
		expect(web).not.toContain("The bot wants to");
		expect(email).toContain("Send email to acme.com");
		expect(email).toContain("1 call");
		expect(email).toContain("Send the weekly digest to team@acme.com");
		expect(callRows()).toHaveLength(3);
		expect(footerButtons()).toEqual([
			"Choose per call",
			"Deny all",
			"Allow all once",
			"Always allow all",
		]);
		expect(button("Show all 3 calls")).toBeUndefined();
	});

	test("a group's calls read by what tells them apart; the whole line stays their name", async () => {
		await show(card(thirty()));
		await click(button("Show all 30 calls in 7 groups"));
		const rows = [
			...container.querySelectorAll("[data-slot='approval-group'] li button"),
		].slice(0, 12);
		expect(rows.map((row) => row.textContent)).toEqual(
			Array.from({ length: 12 }, (_, page) => `GET /alerts?p=${page}`),
		);
		expect(rows[3]?.getAttribute("aria-label")).toBe(
			"Send a GET request to cisa.gov/alerts?p=3",
		);
		expect(rows[3]?.getAttribute("title")).toBe(
			"Send a GET request to cisa.gov/alerts?p=3",
		);
		const lastPart = rows[3]?.lastElementChild;
		expect(lastPart?.textContent).toBe("?p=3");
		expect(lastPart?.className).not.toContain("truncate");
	});

	test("a call's line opens its full summary", async () => {
		await show(card(three()));
		const line = button("Send a GET request to cisa.gov/alerts?p=1");
		expect(line?.textContent).toBe("GET /alerts?p=1");
		expect(line?.getAttribute("aria-expanded")).toBe("false");
		await click(line);
		expect(line?.getAttribute("aria-expanded")).toBe("true");
		expect(container.textContent).toContain(
			"The bot wants to send a GET request to cisa.gov/alerts?p=1.",
		);
	});

	test.each([
		["Always allow all", "always", "Always allowed · 3 calls"],
		["Allow all once", "once", "Allowed once · 3 calls"],
		["Deny all", "deny", "Denied · 3 calls"],
	] as const)(
		"%s answers every call with one decision",
		async (label, decision, shown) => {
			await show(card(three()));
			await click(button(label));
			expect(answers).toEqual([["bot_a", "turn_b", "ap-b", decision]]);
			expect(answerItems).toEqual([undefined]);
			expect(statusText()).toBe(shown);
			expect(window.document.activeElement).toBe(status() as never);
			expect(button("Allow all once")).toBeUndefined();
		},
	);

	test("thirty calls show four groups until asked, then every call in a capped list", async () => {
		await show(card(thirty()));
		expect(statusText()).toBe("Approval request · 30 calls");
		expect(groups()).toHaveLength(4);
		expect(callRows()).toHaveLength(0);
		expect(groups()[0]).toContain("12 calls");
		expect(groups()[1]).toContain("9 calls");
		const toggle = button("Show all 30 calls in 7 groups");
		expect(toggle?.getAttribute("aria-expanded")).toBe("false");
		const frame = () =>
			container.querySelector("[data-slot='approval-batch-frame']");
		expect(toggle?.getAttribute("aria-controls")).toBe(frame()?.id ?? "");
		expect(frame()?.className).not.toContain("overflow-y-auto");
		await click(toggle);
		expect(groups()).toHaveLength(7);
		expect(callRows()).toHaveLength(21);
		expect(button("Show fewer")?.getAttribute("aria-expanded")).toBe("true");
		expect(frame()?.className).toContain("overflow-y-auto");
		await click(button("Show fewer"));
		expect(groups()).toHaveLength(4);
	});

	test("more than five calls in at most four groups: Show all opens their calls", async () => {
		await show(
			card(
				batch([
					...Array.from({ length: 6 }, (_, page) => read(`c${page}`, page)),
				]),
			),
		);
		expect(groups()).toHaveLength(1);
		expect(callRows()).toHaveLength(0);
		await click(button("Show all 6 calls"));
		expect(callRows()).toHaveLength(6);
	});

	test("per call: one choice per group and per call, a tally, and only the differing calls are sent", async () => {
		await show(card(three()));
		await click(button("Choose per call"));
		expect(container.querySelectorAll("[role='radiogroup']")).toHaveLength(4);
		expect(window.document.activeElement?.getAttribute("role")).toBe("radio");
		expect(container.textContent).toContain("3 allowed · 0 denied");
		await click(radio(second, "deny"));
		expect(radio(cisa, "once")?.getAttribute("aria-checked")).toBe("false");
		expect(radio(cisa, "deny")?.getAttribute("aria-checked")).toBe("false");
		expect(groups()[0]).toContain("Mixed");
		await click(radio(acme, "always"));
		expect(container.textContent).toContain("2 allowed · 1 denied");
		await click(button("Send answers"));
		expect(answers).toEqual([["bot_a", "turn_b", "ap-b", "once"]]);
		expect(answerItems).toEqual([
			[
				{ args_hash: "h2", decision: "deny" },
				{ args_hash: "m1", decision: "always" },
			],
		]);
		expect(statusText()).toBe("2 allowed, 1 denied");
	});

	test("a group's choice sets all its calls, and Back returns to the shortcuts", async () => {
		await show(card(three()));
		await click(button("Choose per call"));
		await click(radio(cisa, "deny"));
		expect(radio(second, "deny")?.getAttribute("aria-checked")).toBe("true");
		expect(groups()[0]).not.toContain("Mixed");
		expect(container.textContent).toContain("1 allowed · 2 denied");
		await click(button("Back"));
		expect(container.querySelectorAll("[role='radiogroup']")).toHaveLength(0);
		expect(window.document.activeElement).toBe(
			button("Choose per call") as never,
		);
	});

	test("a call asked again after web content: notice, badge, and no Always anywhere", async () => {
		await show(
			card(
				batch([
					read("h1", 1, { afterWebContent: true }),
					read("h2", 2, { afterWebContent: true }),
					mail,
				]),
			),
		);
		expect(container.textContent).toContain(
			"Asked again because this turn read web pages.",
		);
		expect(groups()[0]).toContain("Asked again");
		expect(groups()[1]).not.toContain("Asked again");
		expect(footerButtons()).toEqual([
			"Choose per call",
			"Deny all",
			"Allow all once",
		]);
		await click(button("Choose per call"));
		const always = radio(cisa, "always");
		expect(always?.disabled).toBe(true);
		const reason = window.document.getElementById(
			always?.getAttribute("aria-describedby") ?? "",
		);
		expect(reason?.textContent).toBe(
			"Can't be remembered after the turn read web pages",
		);
		expect(radio(acme, "always")?.disabled).toBe(false);
	});

	test("a secret row holds the shortcuts until it is saved or declined", async () => {
		await show(card(batch([read("h1", 1), secretItem])));
		expect(container.textContent).toContain("API_TOKEN");
		expect(container.textContent).toContain("Needs a secret");
		expect(container.textContent).toContain("Save or decline the secret first");
		expect(button("Allow all once")?.disabled).toBe(true);
		await type(
			container.querySelector("input[type='password']") as never,
			"s3cret",
		);
		await click(button("Save secret"));
		expect(secrets).toEqual([
			[
				"bot_a",
				"API_TOKEN",
				{ value: "s3cret", description: "For the weather API" },
			],
		]);
		expect(answers).toEqual([]);
		expect(container.textContent).toContain("Secret saved");
		expect(button("Allow all once")?.disabled).toBe(false);
		await click(button("Allow all once"));
		expect(answers).toEqual([["bot_a", "turn_b", "ap-b", "once"]]);
		expect(answerItems).toEqual([undefined]);
	});

	test("Deny all needs no secret first: it denies every call, the secret's too", async () => {
		await show(card(batch([read("h1", 1), secretItem])));
		expect(button("Allow all once")?.disabled).toBe(true);
		expect(button("Deny all")?.disabled).toBe(false);
		await click(button("Deny all"));
		expect(secrets).toEqual([]);
		expect(answers).toEqual([["bot_a", "turn_b", "ap-b", "deny"]]);
		expect(answerItems).toEqual([undefined]);
		expect(statusText()).toBe("Denied · 2 calls");
	});

	test("Deny all after a saved secret still denies every call", async () => {
		await show(card(batch([read("h1", 1), secretItem])));
		await type(
			container.querySelector("input[type='password']") as never,
			"s3cret",
		);
		await click(button("Save secret"));
		await click(button("Deny all"));
		expect(answers).toEqual([["bot_a", "turn_b", "ap-b", "deny"]]);
		expect(answerItems).toEqual([undefined]);
	});

	test("a declined secret is denied while the rest is allowed", async () => {
		await show(card(batch([read("h1", 1), secretItem])));
		await click(button("Decline"));
		expect(secrets).toEqual([]);
		expect(container.textContent).toContain("Declined");
		await click(button("Allow all once"));
		expect(answerItems).toEqual([[{ args_hash: "s1", decision: "deny" }]]);
	});

	test("answered alike: one line, and the calls open with their answers", async () => {
		await show(card(three({ answer: "once" })));
		expect(statusText()).toBe("Allowed once · 3 calls");
		expect(
			container.querySelector("[data-slot='approval-batch-list']"),
		).toBeNull();
		const toggle = button("Show calls");
		expect(toggle?.getAttribute("aria-expanded")).toBe("false");
		await click(toggle);
		expect(callRows()).toHaveLength(3);
		expect(
			(container.textContent ?? "").split("Allowed once").length - 1,
		).toBeGreaterThanOrEqual(4);
		expect(button("Allow all once")).toBeUndefined();
	});

	test("answered call by call: the tally, and each call's own answer", async () => {
		await show(card(three({ answer: "once", itemAnswers: { h2: "deny" } })));
		expect(statusText()).toBe("2 allowed, 1 denied");
		await click(button("Show calls"));
		const [web] = groups();
		expect(web).toContain("Allowed once");
		expect(web).toContain("Denied");
	});

	test("unanswered for more than seven days: expired, without answers", async () => {
		await show(card(three({ postedAt: Date.now() - 8 * DAY })));
		expect(statusText()).toBe("Expired");
		expect(container.textContent).toContain(
			"Nobody answered within 7 days, so Scout stopped.",
		);
		expect(button("Allow all once")).toBeUndefined();
		expect(button("Choose per call")).toBeUndefined();
	});

	test("refused as expired: the card turns expired", async () => {
		refusal = {
			code: "BOT_APPROVAL_EXPIRED",
			message: "[BOT_APPROVAL_EXPIRED] This approval has expired",
		};
		await show(card(three()));
		await click(button("Allow all once"));
		expect(statusText()).toBe("Expired");
		expect(button("Allow all once")).toBeUndefined();
	});

	test("answered on another device: says so with the buttons disabled", async () => {
		refusal = { code: "BOT_TURN_NOT_WAITING", message: "not waiting" };
		await show(card(three()));
		await click(button("Allow all once"));
		expect(container.textContent).toContain("Answered on another device");
		expect(button("Allow all once")?.disabled).toBe(true);
		expect(button("Deny all")?.disabled).toBe(true);
		expect(container.querySelector("[role='alert']")).toBeNull();
	});

	test("a cancelled task closes the card", async () => {
		await show(card(three({ cancelled: true })));
		expect(statusText()).toBe("Task cancelled");
		expect(footerButtons()).toEqual([]);
	});
});

describe("BotToolGrants", () => {
	const bot = { id: "bot_a", name: "Scout" } as unknown as IBot;
	const grant = (tool: string, scope: string): IBotToolGrant => ({
		tool,
		scope,
		granted_at: "2026-11-03T10:00:00Z",
		granted_by: "me",
	});

	test("a person grant names the person once the directory answers", async () => {
		grants = [grant("message_person", "person:user-anna")];
		await show(createElement(BotToolGrants, { bot, name: "Scout" }));
		await settle(60);
		expect(container.textContent).toContain("Message Anna Schmidt");
	});

	test("a person the directory can't name reads as their id", async () => {
		grants = [grant("message_person", "person:user-gone")];
		await show(createElement(BotToolGrants, { bot, name: "Scout" }));
		await settle(60);
		expect(container.textContent).toContain("Message user-gone");
	});

	test("lists standing grants in plain words and hides answer tokens", async () => {
		grants = [
			grant("shell_run", "command:git"),
			grant("email_send", "email:example.com"),
			grant("browser_open", "browser:github.com"),
			grant("shell_run", "once:abc"),
			grant("shell_run", "deny:abc"),
		];
		await show(createElement(BotToolGrants, { bot, name: "Scout" }));
		const text = container.textContent ?? "";
		expect(text).toContain("Allowed actions");
		expect(text).toContain("Run `git` on the command line");
		expect(text).toContain("Send email to example.com");
		expect(text).toContain("Open sites on github.com");
		expect(container.querySelectorAll("li")).toHaveLength(3);
	});

	test("Revoke takes the grant back and the list refreshes", async () => {
		grants = [grant("shell_run", "command:git")];
		await show(createElement(BotToolGrants, { bot, name: "Scout" }));
		await click(button("Stop allowing: Run `git` on the command line"));
		expect(revoked).toEqual([["bot_a", "shell_run", "command:git"]]);
		expect(container.querySelectorAll("li")).toHaveLength(0);
		expect(container.textContent).toContain("Nothing yet");
	});
});

describe("NeedsYouSection", () => {
	const needs = (approval: Record<string, unknown>) => ({
		id: "c1",
		kind: "direct",
		title: null,
		members: [],
		unread_count: 0,
		last_activity_at: "2026-11-03T10:00:00Z",
		last_entry: {
			id: "e1",
			message_id: "ap-1",
			conversation_id: "c1",
			sender_id: "bot_a",
			kind: "message",
			created_at: "2026-11-03T10:00:00Z",
			cursor: "1:e1",
			envelope: { v: 1, enc: "none", body: { text: "", approval } },
		},
	});
	const section = (conversation: unknown) =>
		createElement(NeedsYouSection, {
			items: [{ type: "conversation", conversation }] as never,
			directory: (() => ({ name: "Scout", isBot: true })) as never,
			text: {
				title: () => "Scout",
				entryText: () => "Scout wants to make 3 calls",
				activity: () => "10:00",
			} as never,
			onOpenConversation: () => undefined,
			onOpenAssistant: () => undefined,
		});

	test("a batch says how many calls wait", async () => {
		const item = (hash: string, calls?: number) => ({
			args_hash: hash,
			tool: "web_request",
			summary: "x",
			scope: "web:cisa.gov:read",
			...(calls ? { calls } : {}),
		});
		await show(
			section(
				needs({
					id: "ap-1",
					turn_id: "t1",
					tool: "approval_batch",
					summary: "x",
					scope: "",
					args_hash: "hb",
					items: [item("h1"), item("h2", 2)],
				}),
			),
		);
		expect(container.textContent).toContain("3 calls wait for you · 10:00");
	});

	test("an unanswered card says it is waiting for you", async () => {
		const conversation = {
			id: "c1",
			kind: "direct",
			title: null,
			members: [],
			unread_count: 0,
			last_activity_at: "2026-11-03T10:00:00Z",
			last_entry: {
				id: "e1",
				message_id: "ap-1",
				conversation_id: "c1",
				sender_id: "bot_a",
				kind: "message",
				created_at: "2026-11-03T10:00:00Z",
				cursor: "1:e1",
				envelope: {
					v: 1,
					enc: "none",
					body: {
						text: "Run git",
						approval: {
							id: "ap-1",
							turn_id: "t1",
							tool: "shell_run",
							summary: "Run git",
							scope: "command:git",
							args_hash: "h",
						},
					},
				},
			},
		};
		await show(
			createElement(NeedsYouSection, {
				items: [{ type: "conversation", conversation }] as never,
				directory: (() => ({ name: "Scout", isBot: true })) as never,
				text: {
					title: () => "Scout",
					entryText: () => "Run git",
					activity: () => "10:00",
				} as never,
				onOpenConversation: () => undefined,
				onOpenAssistant: () => undefined,
			}),
		);
		expect(container.textContent).toContain("Waiting for you · 10:00");
		expect(container.textContent).toContain("Run git");
	});
});

describe("the approval card in the bot's chat transcript", () => {
	const entry = (
		seq: number,
		sender: string,
		messageId: string,
		body: Record<string, unknown>,
	) => ({
		id: `e${seq}`,
		message_id: messageId,
		conversation_id: "c1",
		sender_id: sender,
		kind: "message" as const,
		envelope: { v: 1 as const, enc: "none" as const, body },
		created_at: new Date(Date.now() - (60 - seq) * 60_000).toISOString(),
		cursor: `${seq}:e${seq}`,
		seq,
	});
	const card = entry(1, "bot_a", "ap-1", {
		text: "Run git",
		approval: {
			id: "ap-1",
			turn_id: "turn_1",
			tool: "shell_run",
			summary: "Run git",
			scope: "command:git",
			args_hash: "h",
		},
	});
	const answer = entry(2, "me", "ar-1", {
		approval_response: { approval_id: "ap-1", decision: "always" },
	});

	test("an approval entry becomes a card message and its answer is not a message", () => {
		const { messages } = transcriptOf([card, answer], "bot_a", "bot:bot_a");
		expect(messages).toHaveLength(1);
		expect(messages[0].approval).toMatchObject({
			id: "ap-1",
			botId: "bot_a",
			turnId: "turn_1",
			answer: "always",
		});
		expect(messages[0].inner.content).toBe("");
	});

	test("the card answers from the transcript and the invalidation reaches the inbox queries", async () => {
		const approval = transcriptOf([card], "bot_a", "bot:bot_a").messages[0]
			.approval;
		const invalidated: unknown[] = [];
		const original = client.invalidateQueries.bind(client);
		client.invalidateQueries = ((filters: unknown) => {
			invalidated.push(filters);
			return original(filters as never);
		}) as never;
		await show(
			createElement(ChatApprovalCard, { approval: approval as never }),
		);
		expect(container.textContent).toContain("Scout wants to run git");
		await click(button("Allow once"));
		expect(answers).toEqual([["bot_a", "turn_1", "ap-1", "once"]]);
		expect(container.textContent).toContain("Allowed once");
		expect(invalidated).toContainEqual({ queryKey: ["inbox"] });
	});

	test("a batch card with a single item renders as the card for one call", async () => {
		const one = entry(3, "bot_a", "ap-2", {
			text: "x",
			approval: {
				id: "ap-2",
				turn_id: "turn_2",
				tool: "approval_batch",
				summary: "The bot wants to make 1 call",
				scope: "",
				args_hash: "hb",
				items: [
					{
						args_hash: "h1",
						tool: "shell_run",
						summary: "Run git",
						scope: "command:git",
					},
				],
			},
		});
		const approval = transcriptOf([one], "bot_a", "bot:bot_a").messages[0]
			.approval;
		await show(
			createElement(ChatApprovalCard, { approval: approval as never }),
		);
		expect(container.textContent).toContain("Scout wants to run git");
		expect(container.textContent).toContain("Run `git` on the command line");
		expect(
			container.querySelector("[data-slot='approval-batch-list']"),
		).toBeNull();
		await click(button("Always allow"));
		expect(answers).toEqual([["bot_a", "turn_2", "ap-2", "always"]]);
		expect(answerItems).toEqual([undefined]);
	});

	test("after a reload the answered card shows the decision without buttons", async () => {
		const approval = transcriptOf([card, answer], "bot_a", "bot:bot_a")
			.messages[0].approval;
		await show(
			createElement(ChatApprovalCard, { approval: approval as never }),
		);
		expect(container.textContent).toContain("Always allowed");
		expect(container.querySelectorAll("button")).toHaveLength(0);
	});
});
