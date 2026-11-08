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
	) => {
		if (failAnswer) throw new Error("offline");
		events.push("answer");
		answers.push([botId, turnId, approvalId, decision]);
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
		expect(container.textContent).toContain("internet");
		expect(button("Always allow")).toBeUndefined();
		expect(button("Allow once")).toBeDefined();
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
		created_at: new Date(Date.UTC(2026, 10, 3, 10, seq)).toISOString(),
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
