import { afterAll, afterEach, describe, expect, test } from "bun:test";
import { byRole, click, installDom, queryByRole } from "../testing/dom-harness";

const dom = installDom();
const { cleanupDevices, preloadDevices } = await import(
	"../testing/mount-devices"
);
await preloadDevices();
// Device-lib modules load after the DOM exists (a static import breaks combined runs, W2-FAKES).
const { SAMPLE_NOW } = await import(
	"../../../../lib/device-management/model/__fixtures__/sample-fleet"
);
const { AGENT_FEATURES } = await import(
	"../../../../lib/device-management/model/types"
);
const { SHOP, openShop, shopRow, stopShop, text } = await import(
	"./status-test-kit"
);

afterEach(async () => {
	await cleanupDevices();
	await dom.cleanup();
});
afterAll(dom.restore);

type Fake = Parameters<typeof shopRow>[0];
type ServiceBot = NonNullable<ReturnType<typeof shopRow>["bots"]>[number];

const block = (root: ParentNode) =>
	root.querySelector<HTMLElement>("#service-bots");
const bot = (root: ParentNode, eventId: string = SHOP.telegram) =>
	root.querySelector<HTMLElement>(`[data-bot="${eventId}"]`);
const lines = (root: ParentNode, eventId: string = SHOP.telegram) =>
	[...(bot(root, eventId)?.querySelectorAll("[data-bot-line]") ?? [])].map(
		(line) => text(line),
	);

/** What the Telegram bot's process reports, written as the device writes it. */
function facts(
	values: Partial<ServiceBot>,
	eventId: string = SHOP.telegram,
): (fake: Fake) => void {
	return (fake) => {
		const agent = fake.agent(SHOP.device);
		agent.botFacts.set(eventId, values);
		agent.report(shopRow(fake));
	};
}

/** Shop Assistant with its two bots only. */
const open = (options: Parameters<typeof openShop>[0] = {}) =>
	openShop({ events: [SHOP.telegram, SHOP.discord], ...options });

describe("Service › Status · Bots", () => {
	test("connected: who it is connected as, since when, and the device's numbers", async () => {
		const { container } = await open({
			arrange: facts({
				last_message_at: SAMPLE_NOW - 600,
				last_outcome: "succeeded",
				runs_today: 3,
				runs: 12,
				failed: 1,
				running: 1,
				dropped: 2,
			}),
		});
		const item = bot(container);
		expect(item?.dataset.botState).toBe("connected");
		expect(text(item)).toContain("Shop helper");
		expect(text(item)).toContain("Telegram bot");
		expect(lines(container)).toEqual([
			expect.stringMatching(/^Connected as Shop helper since .+\.$/),
			"Last message 10 min. ago · its run succeeded",
			"3 runs today · 12 since it started · 1 failed",
			"1 run is going now.",
			"Not answered: 2 messages (too many at once, or too old after a start).",
		]);
		expect(text(bot(container, SHOP.discord))).toContain("Discord bot");
		expect(text(block(container))).toContain(
			"A bot runs in one place. It connects once this service may run it, and stays connected while the service runs.",
		);
		expect(queryByRole("button", "Run it here", container)).toBeNull();
	});

	test("a bot without a name it may show is still connected", async () => {
		const { container } = await open({ arrange: facts({ bot_name: null }) });
		expect(bot(container)?.dataset.botState).toBe("connected");
		expect(lines(container)[0]).toMatch(/^Connected since .+\.$/);
		expect(text(bot(container))).not.toContain("Connected as");
	});

	test("from the published status: connected or reconnecting, and no live numbers", async () => {
		const { container } = await open({
			reader: "status",
			arrange: facts({ runs_today: 3, runs: 12 }),
		});
		expect(bot(container)?.dataset.botState).toBe("ok");
		expect(lines(container)).toEqual([
			"Connected or reconnecting when this status was published.",
		]);
	});

	test("without this computer's keys nothing is said about it", async () => {
		const { container } = await open({ reader: "locked" });
		expect(text(container)).not.toContain("Connected");
		const item = bot(container);
		if (item) {
			expect(item.dataset.botState).toBe("locked");
			expect(lines(container)).toEqual(["Unknown until unlocked"]);
		}
	});

	test("an agent without the bot flags can't say: the row asks for the agent update", async () => {
		const features = Object.fromEntries(
			AGENT_FEATURES.filter(
				(flag) => flag !== "telegram_bots" && flag !== "discord_bots",
			).map((flag) => [flag, 1 as const]),
		);
		const { container } = await open({ agentFeatures: features });
		expect(bot(container)?.dataset.botState).toBe("needs_agent");
		expect(lines(container)).toEqual([
			"Update the device agent to see its bots.",
		]);
	});

	test("a stopped service connects nothing, whatever its row still carries", async () => {
		const { container } = await open({ arrange: stopShop });
		expect(bot(container)?.dataset.botState).toBe("stopped");
		expect(lines(container)).toEqual([
			"Not connected while the service is not running.",
		]);
	});

	test("a running service that has not said anything about it yet", async () => {
		const { container } = await open({
			arrange: (fake) => {
				shopRow(fake).bots = undefined;
			},
		});
		expect(bot(container)?.dataset.botState).toBe("not_reported");
		expect(lines(container)).toEqual(["Not reported yet."]);
	});

	test("refused by its provider: what happened and where to fix it", async () => {
		const { container } = await open({
			arrange: (fake) => {
				facts({ state: "token_refused" })(fake);
				facts({ state: "intents_refused" }, SHOP.discord)(fake);
			},
		});
		expect(bot(container)?.dataset.botState).toBe("token_refused");
		expect(lines(container)[0]).toBe(
			"Telegram refused the token. Enter a new one under Configuration.",
		);
		const fix = byRole(
			"link",
			"Open Configuration",
			bot(container) as HTMLElement,
		);
		expect(fix.getAttribute("href")).toContain("tab=configuration");
		expect(lines(container, SHOP.discord)[0]).toBe(
			"Discord refused the bot's permissions. Turn on the message content intent in the Discord Developer Portal, then restart shop-assistant.",
		);
	});

	test("another program or a webhook takes its messages", async () => {
		const { container } = await open({
			arrange: (fake) => {
				facts({ state: "conflict" })(fake);
				facts({ state: "reconnecting" }, SHOP.discord)(fake);
			},
		});
		expect(lines(container)[0]).toBe(
			"Another program uses this bot's token. A bot runs in one place: stop it there.",
		);
		expect(lines(container, SHOP.discord)[0]).toBe("Reconnecting.");
	});
});

describe("Service › Status · a bot nobody moved here", () => {
	const HELD =
		"Not connected: nobody who can edit this app's events has moved it to this service.";

	test("it says why it isn't connected; Run it here moves it after a confirm that says so", async () => {
		const { container, world, settle } = await open({ release: false });
		const item = bot(container) as HTMLElement;
		expect(item.dataset.botState).toBe("held");
		expect(lines(container)).toEqual([HELD]);
		await click(byRole("button", "Run it here", item));
		expect(text(bot(container))).toContain(
			"shop-assistant connects Shop helper at its next check, within 5 minutes.",
		);
		expect(text(bot(container))).toContain(
			"A bot runs in one place: no other device answers its messages afterwards.",
		);
		expect(text(bot(container))).toContain("Choose Take it back in Events.");
		expect(world.api.sent("PUT", /device-schedules/).length).toBe(0);
		const confirm = [...(bot(container)?.querySelectorAll("button") ?? [])]
			.filter((button) => text(button) === "Run it here")
			.at(-1);
		await click(confirm as HTMLElement);
		await settle();
		const [call] = world.api.sent("PUT", /device-schedules/);
		expect(call?.[1]).toContain(
			`apps/${SHOP.app}/device-schedules/${SHOP.telegram}`,
		);
		expect(call?.[2]).toEqual({
			device_id: SHOP.device,
			placement_id: SHOP.service,
		});
	});

	test("waiting for the hub, and a hub that can't hand bots over", async () => {
		const { container } = await open({
			arrange: (fake) => {
				for (const entry of shopRow(fake).bots ?? []) {
					entry.state = "waiting";
					entry.hold =
						entry.event_id === SHOP.telegram
							? "hub_unreachable"
							: "hub_too_old";
				}
			},
		});
		expect(lines(container)).toEqual([
			"Waiting for the hub to confirm that this service may run it.",
		]);
		expect(lines(container, SHOP.discord)).toEqual([
			"This hub can't hand bots to devices yet. Update the hub.",
		]);
	});
});

test("a service without bots shows no Bots block", async () => {
	const { container } = await openShop({ events: [SHOP.once] });
	expect(block(container)).toBeNull();
});

describe("Service · Stop, Start and instances with bots", () => {
	const bar = (root: ParentNode) =>
		root.querySelector("[data-service-actions]") as HTMLElement;
	const confirm = (root: ParentNode) =>
		text(root.querySelector("[data-inline-confirm]"));

	test("Stop: its bots disconnect, and what happens to their messages meanwhile", async () => {
		const { container, world } = await open();
		await click(byRole("button", "Stop…", bar(container)));
		expect(confirm(container)).toContain(
			"Its bots disconnect. Telegram keeps messages for a day; after a start edge-berlin-01 answers those of the last 15 minutes. Discord messages sent meanwhile are not answered.",
		);
		expect(world.api.commands.some(([, type]) => type === "stop")).toBe(false);
	});

	test("Start: they connect again, also when the stopped service reports none", async () => {
		const { container } = await open({
			arrange: (fake) => {
				stopShop(fake);
				Object.assign(shopRow(fake), {
					applied_revision: shopRow(fake).config_revision,
					bots: undefined,
				});
			},
		});
		await click(byRole("button", "Start", bar(container)));
		expect(confirm(container)).toContain("Its bots connect again.");
	});

	test("one instance, and why: two would answer every message twice", async () => {
		const { container } = await open();
		expect(text(bar(container))).toContain(
			"This service runs one instance: two would answer every message to its bot twice.",
		);
	});

	test("a service without bots says nothing about bots", async () => {
		const { container } = await openShop({ events: [SHOP.once] });
		await click(byRole("button", "Stop…", bar(container)));
		expect(confirm(container)).not.toContain("bots");
		expect(text(bar(container))).toContain(
			"This service runs one instance: two would start every scheduled run twice.",
		);
	});
});
