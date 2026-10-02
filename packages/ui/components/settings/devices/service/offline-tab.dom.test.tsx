import { afterAll, afterEach, describe, expect, test } from "bun:test";
import {
	byRole,
	click,
	inPortal,
	installDom,
	queryByRole,
} from "../testing/dom-harness";

const dom = installDom();
const { cleanupDevices, preloadDevices } = await import(
	"../testing/mount-devices"
);
await preloadDevices();
const {
	EDGE,
	STUDIO,
	WAREHOUSE,
	MACHINE,
	commandsOf,
	openTab,
	patchConfig,
	primaries,
	sent,
	text,
	until,
	writes,
} = await import("./config-test-kit");
type Kit = typeof import("./config-test-kit");
type View = Awaited<ReturnType<Kit["openTab"]>>;

afterEach(async () => {
	await cleanupDevices();
	await dom.cleanup();
});
afterAll(dom.restore);

const CURRENT_SCOPE =
	"25db08116711b6e982332459d70bb8657fedb1e468cde18ac13a1761e256f7f6";
const HEAD = "f40615e0-b484-42d1-acd5-408b2c9e9938";

const open = (
	device: string,
	service: string,
	options: Partial<Parameters<Kit["openTab"]>[0]> = {},
) => openTab({ tab: "offline", device, service, ...options });

const fieldNotes = (options: Partial<Parameters<Kit["openTab"]>[0]> = {}) =>
	open(STUDIO, "field-notes", options);

const summary = (view: View) =>
	view.container.querySelector<HTMLElement>("[data-queue-summary]");
const cards = (view: View) =>
	Array.from(view.container.querySelectorAll<HTMLElement>("[data-queue-head]"));
const card = (view: View, state: string) =>
	view.container.querySelector<HTMLElement>(
		`[data-queue-head=${state}]`,
	) as HTMLElement;

async function ready(view: View) {
	await until(() => summary(view) !== null);
}

describe("queues", () => {
	test("load on connect without a click: counts, sizes, ages and warnings", async () => {
		const view = await fieldNotes({
			arrange: (fake) => {
				const queues = fake.agent(STUDIO).offlineQueues["field-notes"];
				if (queues?.[0])
					queues[0].mirror_error = "The local table copy is 40 minutes old.";
			},
		});
		await ready(view);
		expect(queryByRole("button", /refresh/i, view.container)).toBeNull();
		expect(text(summary(view) as HTMLElement)).toBe("17 waiting · 2 need you");
		const page = text(view.container);
		expect(page).toContain("For your current approval");
		expect(page).toContain("For your earlier approval");
		expect(page).toContain("Needs you: conflict");
		expect(page).toContain("Paused: cloud access changed");
		expect(page).toContain("3.0 MiB of 256 MiB");
		expect(page).toContain("of 7 days max · kept until");
		expect(page).toContain("kept while paused");
		expect(page).toContain("The local table copy is 40 minutes old.");
		expect(page).toContain(
			"Each queue replays under the approval it was queued with.",
		);
		expect(page).toContain(
			"Write buffering can be turned off once nothing waits: 17 changes still wait.",
		);
		expect(
			view.container.querySelector("#svc-queues [data-stamp][data-src=live]"),
		).not.toBeNull();
		expect(
			view.container
				.querySelector("#svc-queues table")
				?.getAttribute("data-stack"),
		).toBe("900");
		expect(sent(view, STUDIO)).toContain("offline_queue");
		expect(writes(view)).toEqual([]);
		expect(primaries()).toBe(0);
		expect(page).not.toMatch(MACHINE);
	});

	test("the next change of a queue names its target, attempts and what waits behind it", async () => {
		const view = await fieldNotes();
		await ready(view);
		const conflict = card(view, "conflict");
		const page = text(conflict);
		expect(page).toContain("Conflicts with newer cloud data");
		expect(page).toContain("table notes in Project storage");
		expect(page).toContain("13 changes wait");
		expect(page).toContain(
			"The cloud copy of this record changed after this change was queued",
		);
		expect(page).toContain(
			"The cloud table changed since this write was queued.",
		);
		const blocked = card(view, "blocked");
		expect(text(blocked)).toContain(
			"file exports/2026-09-21-summary.pdf in User files",
		);
	});

	test("an empty queue is up to date, with no next change", async () => {
		const view = await fieldNotes({
			arrange: (fake) => {
				fake.agent(STUDIO).offlineQueues["field-notes"] = [
					{
						scope: CURRENT_SCOPE,
						quarantined: false,
						pending_count: 0,
						pending_bytes: 0,
						oldest_at: null,
						mirror_error: null,
						head: null,
					},
				];
			},
		});
		await ready(view);
		await until(
			() => text(summary(view) as HTMLElement) === "Up to date · 0 waiting",
		);
		expect(cards(view)).toEqual([]);
		expect(text(view.container)).not.toContain("Next change in line");
	});
});

describe("try again", () => {
	test("retries exactly the change at the head of its queue", async () => {
		const view = await fieldNotes();
		await ready(view);
		await click(byRole("button", "Try again", card(view, "conflict")));
		await until(() => commandsOf(view, "offline_queue_retry").length === 1);
		expect(commandsOf(view, "offline_queue_retry")[0]).toMatchObject({
			placement_id: "field-notes",
			scope: CURRENT_SCOPE,
			queued_operation_id: HEAD,
		});
		expect(writes(view)).toEqual(["offline_queue_retry"]);
		await until(() => text(view.container).includes("Tried again at"));
		expect(text(view.container)).toContain("the change still needs you");
	});

	test("a definitive refusal shows the device's reason", async () => {
		const view = await fieldNotes({
			arrange: (fake) => {
				fake
					.agent(STUDIO)
					.reject(
						"offline_queue_retry",
						"invalid",
						"The queued write changed since it was read.",
					);
			},
		});
		await ready(view);
		await click(byRole("button", "Try again", card(view, "conflict")));
		await until(() =>
			text(view.container).includes(
				"The queued write changed since it was read.",
			),
		);
		expect(text(view.container)).toContain("was refused by the device");
	});

	test("an older agent's refusal without a reason keeps the plain sentence", async () => {
		const view = await fieldNotes({
			arrange: (fake) => {
				fake.agent(STUDIO).handle("offline_queue_retry", () => ({
					state: "rejected",
					result: {},
				}));
			},
		});
		await ready(view);
		await click(byRole("button", "Try again", card(view, "conflict")));
		await until(() => text(view.container).includes("didn't run."));
		expect(text(view.container)).toContain("The device rejected the request.");
	});
});

describe("discard", () => {
	test("an attempted change needs a reason and the acknowledgement that it may already be saved", async () => {
		const view = await fieldNotes();
		await ready(view);
		await click(
			byRole("button", "Discard this change…", card(view, "conflict")),
		);
		const sheet = inPortal("alertdialog");
		expect(text(sheet)).toContain("Discard this queued change?");
		expect(text(sheet)).toContain(
			"This change will never be applied to the cloud.",
		);
		expect(text(sheet)).toContain("The 13 changes behind it continue.");
		expect(text(sheet)).toContain("No, this is permanent.");
		const confirm = byRole("button", "Discard change", sheet);
		expect(confirm.getAttribute("aria-disabled")).toBe("true");
		await click(confirm);
		expect(writes(view)).toEqual([]);
		await click(byRole("radio", /The cloud data is newer and correct/, sheet));
		expect(confirm.getAttribute("aria-disabled")).toBe("true");
		await click(
			byRole(
				"checkbox",
				/It may already have reached the cloud\. I still want to discard it\./,
				sheet,
			),
		);
		expect(
			byRole("button", "Discard change", sheet).getAttribute("aria-disabled"),
		).toBeNull();
		await click(byRole("button", "Discard change", sheet));
		await until(() => commandsOf(view, "offline_queue_skip").length === 1);
		expect(commandsOf(view, "offline_queue_skip")[0]).toMatchObject({
			placement_id: "field-notes",
			scope: CURRENT_SCOPE,
			queued_operation_id: HEAD,
			reason: "The cloud data is newer and correct",
			acknowledge_uncertain: true,
		});
		await until(() => text(view.container).includes("Discarded at"));
	});

	test("Cancel sends nothing", async () => {
		const view = await fieldNotes();
		await ready(view);
		await click(
			byRole("button", "Discard this change…", card(view, "conflict")),
		);
		await click(byRole("button", "Cancel", inPortal("alertdialog")));
		expect(writes(view)).toEqual([]);
	});
});

describe("paused queues", () => {
	test("stay visible without replay controls and point to Cloud access", async () => {
		const view = await fieldNotes();
		await ready(view);
		const blocked = card(view, "blocked");
		expect(queryByRole("button", "Try again", blocked)).toBeNull();
		expect(queryByRole("button", /Discard/, blocked)).toBeNull();
		expect(text(blocked)).toContain(
			"Paused until the queue uses current cloud access.",
		);
		expect(text(blocked)).toContain(
			"The cloud access this change was queued under was replaced",
		);
		const fix = byRole("link", "Fix cloud access", blocked);
		expect(fix.getAttribute("href")).toContain("tab=cloud");
		expect(fix.getAttribute("href")).toContain("service=field-notes");
	});
});

describe("change kind and finished changes", () => {
	test("an agent that reports them: the change kind, its size and what happened to earlier changes", async () => {
		const view = await fieldNotes({
			arrange: (fake) => {
				fake.agent(STUDIO).handle("offline_queue_operations", (command) => ({
					state: "completed",
					result: {
						operations:
							command.scope !== CURRENT_SCOPE
								? []
								: command.terminal
									? [
											{
												sequence: 870,
												operation_id: "0a0b0c0d-1111-4222-8333-444455556666",
												resource:
													'{"kind":"table","purpose":"storage","database":"db","table":"notes"}',
												mutation_kind: null,
												state: "skipped",
												attempts: 1,
												created_at: 1_790_750_000,
												bytes: 0,
												error: null,
												error_code: null,
											},
										]
									: [
											{
												sequence: 881,
												operation_id: HEAD,
												resource:
													'{"kind":"table","purpose":"storage","database":"db","table":"notes"}',
												mutation_kind: "table_upsert",
												state: "conflict",
												attempts: 2,
												created_at: 1_790_758_200,
												bytes: 2048,
												error: null,
												error_code: "conflict",
											},
										],
						next: null,
					},
				}));
			},
		});
		await ready(view);
		await until(
			() =>
				card(view, "conflict").querySelector("[data-change-kind='']") !== null,
		);
		expect(text(card(view, "conflict"))).toContain(
			"Adds or updates rows · 2.0 KiB",
		);
		const finished = view.container.querySelector(
			"[data-finished]",
		) as HTMLElement;
		expect(text(finished)).toContain("Last 1 finished change");
		expect(text(finished)).toContain("Discarded");
		expect(text(view.container)).not.toMatch(MACHINE);
	});

	test("an older agent: the interim line renders and the lookup is never sent", async () => {
		const view = await fieldNotes({ agentFeatures: {} });
		await ready(view);
		expect(text(card(view, "conflict"))).toContain(
			"Update the device agent to see the change kind.",
		);
		expect(text(view.container)).toContain(
			"Badges on other pages update while a live connection is open.",
		);
		expect(view.container.querySelector("[role=alert]")).toBeNull();
		expect(sent(view, STUDIO)).not.toContain("offline_queue_operations");
		expect(sent(view, STUDIO)).not.toContain("offline_queue_lookup");
	});

	test("a current agent reports queue summaries in its status, so the live-only sentence is gone", async () => {
		const view = await fieldNotes();
		await ready(view);
		expect(text(view.container)).not.toContain(
			"Badges on other pages update while a live connection is open.",
		);
	});
});

describe("not used", () => {
	test("an offline copy keeps its data on the device, so nothing is buffered or read", async () => {
		const view = await open(EDGE, "support-bot");
		await until(() =>
			text(view.container).includes(
				"Not used: this service runs an offline copy",
			),
		);
		expect(text(view.container)).toContain(
			"An offline copy keeps its data only on edge-berlin-01 and never syncs back, so nothing waits for the cloud.",
		);
		expect(sent(view, EDGE)).not.toContain("offline_queue");
		expect(summary(view)).toBeNull();
	});

	test("an online service with buffering off says how to turn it on", async () => {
		const view = await fieldNotes({
			arrange: (fake) =>
				patchConfig(fake, STUDIO, "field-notes", (config) => {
					config.offline_writes = null;
				}),
		});
		await until(() =>
			text(view.container).includes("Not used: write buffering is off"),
		);
		expect(text(view.container)).toContain(
			"Changes go straight to the cloud; nothing is queued on the device. Turn it on with Update… to keep accepting changes when the internet drops.",
		);
		expect(sent(view, STUDIO)).not.toContain("offline_queue");
	});
});

describe("states without settings", () => {
	test("without Deploy & configure the queues still show, without budgets", async () => {
		const view = await fieldNotes({
			arrange: (fake) => {
				fake
					.agent(STUDIO)
					.reject(
						"placement_configuration",
						"unauthorized",
						"Deploy capability required.",
					);
			},
		});
		await ready(view);
		const page = text(view.container);
		expect(page).toContain("3.0 MiB");
		expect(page).not.toContain("of 256 MiB");
		expect(page).toContain("Needs you: conflict");
	});

	test("a queue read the device turns down ends in a sentence and Try again, not in an endless wait", async () => {
		let lift: () => void = () => undefined;
		// Refused from the first read on, so no earlier read is left to fall back on.
		const view = await fieldNotes({
			unlock: "none",
			arrange: async (fake) => {
				lift = fake
					.agent(STUDIO)
					.reject("offline_queue", "busy", "The device database is busy.");
				await fake.unlock(STUDIO, { connectLive: true });
			},
		});
		await until(() =>
			text(view.container).includes(
				"The queues couldn't be read from studio-mac-mini",
			),
		);
		const page = text(view.container);
		expect(page).toContain("Buffered changes stay on the device.");
		expect(page).not.toContain("Needs View status");
		expect(view.container.querySelector("[data-kind=loading]")).toBeNull();
		expect(page).not.toMatch(MACHINE);
		lift();
		await click(byRole("button", "Try again", view.container));
		await ready(view);
		expect(text(summary(view) as HTMLElement)).toBe("17 waiting · 2 need you");
	});

	test("an offline device: the queues aren't known and nothing is sent", async () => {
		const view = await open(WAREHOUSE, "scanner-ingest");
		expect(text(view.container)).toContain("Settings aren't known");
		expect(text(view.container)).not.toContain("Nothing here yet");
		expect(sent(view, WAREHOUSE)).toEqual([]);
	});

	test("an older hub: the tab reads the device and shows no error", async () => {
		const view = await fieldNotes({ hubVersion: "old" });
		await ready(view);
		expect(text(summary(view) as HTMLElement)).toBe("17 waiting · 2 need you");
		expect(view.container.querySelector("[role=alert]")).toBeNull();
	});
});
