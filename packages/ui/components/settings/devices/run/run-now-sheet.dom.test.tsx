import { afterAll, afterEach, describe, expect, test } from "bun:test";
import { act } from "react";
import {
	advance,
	byRole,
	click,
	installDom,
	queryByRole,
	typeInto,
} from "../testing/dom-harness";

const dom = installDom();
const { cleanupDevices, mountDevices, preloadDevices } = await import(
	"../testing/mount-devices"
);
await preloadDevices();
// Device-lib modules load after the DOM exists (a static import breaks combined runs, W2-FAKES).
const { createFakeWorkspace } = await import("../testing/fake-workspace");
const { formField } = await import("../testing/fake-runs");
const { QUICK_REPLY, SHOP, serveQuickReplyOnEdge, serveShopOnEdge } =
	await import("../testing/schedule-scenarios");
const { ACTIVITY_STORAGE_PREFIX } = await import(
	"../../../../lib/device-management/workspace/activity"
);
const { useOverlayStore } = await import("../workspace/overlay-store");
const { runPace } = await import("./run-store");
const { RunsYouStarted } = await import("./runs-you-started");

Object.assign(runPace, { fastMs: 15, fastForMs: 10_000, slowMs: 15 });

afterEach(async () => {
	await act(async () => useOverlayStore.getState().close());
	await cleanupDevices();
	await dom.cleanup();
});
afterAll(dom.restore);

type Fake = Awaited<ReturnType<typeof createFakeWorkspace>>;
type Mounted = Awaited<ReturnType<typeof mountDevices>>;
type FormSpec = Fake["hub"]["eventForms"][string];

const text = (root: ParentNode | null | undefined) =>
	(root?.textContent ?? "").replace(/\s+/g, " ").trim();
const sheet = () => byRole("dialog");
const button = (name: string | RegExp) => byRole("button", name, sheet());
const field = (name: string) =>
	sheet().querySelector<HTMLElement>(`[data-run-field="${name}"]`);
const control = (name: string) =>
	field(name)?.querySelector<HTMLInputElement | HTMLTextAreaElement>(
		"input,textarea",
	) ?? null;
const sentRuns = (fake: Fake) =>
	fake.api.commands.filter(([, type]) => type === "run_event");

interface OpenOptions {
	/** The form the flow defines; Shop Assistant's return form by default. */
	form?: FormSpec;
	/** Support Portal's quick action (local-only) instead of Shop Assistant's form (online). */
	quickReply?: boolean;
	/** Changes the world after the service runs and before the sheet opens. */
	arrange?(fake: Fake): void | Promise<void>;
	/** Also mounts "Runs you started" of the service. */
	started?: boolean;
	/** Another computer that can't reach the device: it has only the device's published status. */
	offline?: boolean;
	fake?: Fake;
}

const SHOP_TARGET = {
	deviceId: SHOP.device,
	serviceId: SHOP.service,
	eventId: SHOP.form,
};
const QUICK_TARGET = {
	deviceId: QUICK_REPLY.device,
	serviceId: QUICK_REPLY.service,
	eventId: QUICK_REPLY.event,
};

async function serve(options: OpenOptions) {
	const fake = options.fake ?? (await createFakeWorkspace());
	if (options.form) fake.hub.eventForms[SHOP.form] = options.form;
	if (options.quickReply) await serveQuickReplyOnEdge(fake);
	else await serveShopOnEdge(fake, { events: [SHOP.orders, SHOP.form] });
	await options.arrange?.(fake);
	return fake;
}

/** Shop Assistant's form (online) or Support Portal's quick action (local-only) on edge-berlin-01, its sheet open. */
async function reader(world: Fake, deviceId: string, offline?: boolean) {
	if (!offline) {
		await world.workspace.live.refreshInspection(deviceId);
		return world;
	}
	world.api.hub.publishStatus(deviceId, world.agent(deviceId));
	world.agent(deviceId).online = false;
	return createFakeWorkspace(undefined, { api: world.api, viewFacts: false });
}

async function open(options: OpenOptions = {}) {
	const target = options.quickReply ? QUICK_TARGET : SHOP_TARGET;
	const fake = await reader(
		await serve(options),
		target.deviceId,
		options.offline,
	);
	const mounted = await mountDevices(
		options.started ? (
			<RunsYouStarted
				deviceId={target.deviceId}
				serviceId={target.serviceId}
				events={[{ id: SHOP.form, name: "Return request" }]}
			/>
		) : (
			<div />
		),
		{ fake, overlays: true },
	);
	await openSheet(mounted, target);
	return { fake, mounted, target };
}

async function openSheet(
	mounted: Mounted,
	target: { deviceId: string; serviceId: string; eventId: string },
	operationId?: string,
) {
	await act(async () =>
		useOverlayStore
			.getState()
			.openRunNow({ ...target, ...(operationId ? { operationId } : {}) }),
	);
	await mounted.settle();
}

/** Lets the sheet read until it shows `selector`, at most four seconds. */
async function until(mounted: Mounted, selector: string) {
	for (let round = 0; round < 160; round++) {
		if (document.querySelector(selector)) return;
		await advance(25);
		await mounted.settle();
	}
	throw new Error(`The sheet never showed ${selector}: ${text(sheet())}`);
}

async function run(mounted: Mounted, end = "[data-run-phase=ended]") {
	await click(button("Run"));
	await until(mounted, end);
}

/** A form with one field of each kind a device can send from here. */
const EVERY_KIND: FormSpec = {
	description: "Files a note on the shop's board.",
	fields: [
		formField("title", { label: "Title" }),
		formField("count", { data_type: "Integer", label: "Count", default: 2 }),
		formField("ratio", { data_type: "Float", label: "Ratio", optional: true }),
		formField("urgent", {
			data_type: "Boolean",
			label: "Urgent",
			optional: true,
		}),
		formField("due", {
			data_type: "Date",
			label: "Due",
			default: "2026-10-03",
		}),
		formField("size", {
			label: "Size",
			options: ["S", "M", "L"],
			default: "M",
		}),
		formField("meta", { data_type: "Struct", label: "Meta", optional: true }),
		formField("tags", {
			value_type: "Array",
			label: "Tags",
			optional: true,
		}),
		formField("pin", { label: "PIN", sensitive: true, optional: true }),
	],
};

describe("Run now… · the form", () => {
	test("a quick action: no input, what it does, and a run to its result", async () => {
		const { fake, mounted } = await open({ quickReply: true });
		expect(text(byRole("heading", undefined, sheet()))).toBe(
			"Run Quick reply on edge-berlin-01",
		);
		expect(text(sheet())).toContain("This action takes no input.");
		expect(text(sheet())).toContain("Runs the flow once on edge-berlin-01.");
		expect(text(sheet())).toContain(
			"It uses edge-berlin-01's own copy of the app's data.",
		);
		expect(text(sheet())).not.toContain("What you enter becomes");
		expect(text(sheet())).toContain("Can you undo it?No.");
		fake.agent(QUICK_REPLY.device).runs.script = {
			queuedReads: 1,
			runningReads: 1,
			end: { output: { reply: "Thanks" } },
		};
		await run(mounted);
		expect(text(sheet())).toContain("The run succeeded.");
		expect(text(sheet().querySelector("[data-run-output]"))).toContain(
			'"reply": "Thanks"',
		);
		expect(sentRuns(fake)).toHaveLength(1);
		expect(sentRuns(fake)[0]?.[2]).not.toHaveProperty("payload");
		expect(button("Run again")).toBeTruthy();
	});

	test("a form with every field kind: defaults filled, values sent as the device takes them, the input said to be recorded", async () => {
		const { fake, mounted } = await open({ form: EVERY_KIND });
		expect(text(sheet())).toContain("Files a note on the shop's board.");
		const controls = Object.fromEntries(
			EVERY_KIND.fields.map((spec) => [
				spec.name,
				field(spec.name)?.dataset.control,
			]),
		);
		expect(controls).toEqual({
			title: "text",
			count: "integer",
			ratio: "number",
			urgent: "switch",
			due: "date",
			size: "select",
			meta: "json",
			tags: "json",
			pin: "password",
		});
		expect(control("count")?.value).toBe("2");
		expect(control("due")?.value).toBe("2026-10-03");
		expect(text(field("size"))).toContain("M");
		expect(control("pin")?.type).toBe("password");
		expect(text(field("title"))).toContain("required");
		expect(text(field("ratio"))).not.toContain("required");
		expect(text(sheet())).toContain(
			"It runs with shop-assistant's cloud access, not as you.",
		);
		expect(text(sheet())).toContain(
			"What you enter becomes the run's input. shop-assistant keeps it in its run records on edge-berlin-01.",
		);

		const title = control("title");
		const count = control("count");
		const meta = control("meta");
		const tags = control("tags");
		const pin = control("pin");
		if (!title || !count || !meta || !tags || !pin)
			throw new Error("A field has no control.");
		await typeInto(title, "Hello");
		await typeInto(count, "5");
		await typeInto(meta, '{"source": "devices"}');
		await typeInto(tags, '["a", "b"]');
		await typeInto(pin, "4711");
		await click(byRole("switch", /Urgent/, sheet()));
		fake.agent(SHOP.device).runs.script = {
			end: { output: { note: "N-1", secret: "s3cr3t-output" } },
		};
		await run(mounted);
		expect(sentRuns(fake)[0]?.[2]).toMatchObject({
			placement_id: SHOP.service,
			event_id: SHOP.form,
			payload: {
				title: "Hello",
				count: 5,
				urgent: true,
				due: "2026-10-03",
				size: "M",
				meta: { source: "devices" },
				tags: ["a", "b"],
				pin: "[redacted]",
			},
		});
		expect(sentRuns(fake)[0]?.[2].payload).not.toHaveProperty("ratio");
		expect(text(sheet())).toContain("s3cr3t-output");

		// What the browser keeps of the run: ids, state and times, never the input or the output.
		const stored = Object.keys(localStorage)
			.filter((key) => key.startsWith(ACTIVITY_STORAGE_PREFIX))
			.map((key) => localStorage.getItem(key) ?? "")
			.join("\n");
		expect(stored).toContain(SHOP.form);
		expect(stored).toContain('"event_run"');
		for (const secret of ["4711", "Hello", "s3cr3t-output", "N-1", "devices"])
			expect(stored).not.toContain(secret);
	});

	test("a required field left empty sends nothing and says so under the field", async () => {
		const { fake } = await open({ form: EVERY_KIND });
		await click(button("Run"));
		expect(text(field("title"))).toContain("Enter a value.");
		expect(sentRuns(fake)).toHaveLength(0);
		const count = control("count");
		if (!count) throw new Error("No count field.");
		await typeInto(count, "2.5");
		await click(button("Run"));
		expect(text(field("count"))).toContain("Enter a whole number.");
		expect(sentRuns(fake)).toHaveLength(0);
	});

	test("a form with a file field: the control is off and the Endpoint is offered", async () => {
		const { fake, mounted } = await open();
		expect(text(sheet())).toContain(
			"This form takes a file. Open it on the service page.",
		);
		expect(sheet().querySelector("[data-run-fields]")).toBeNull();
		expect(button("Run").hasAttribute("disabled")).toBe(true);
		await click(button("Open Endpoint"));
		await mounted.settle();
		expect(queryByRole("dialog")).toBeNull();
		expect(mounted.navigations.at(-1)?.href).toContain("tab=endpoint");
		expect(sentRuns(fake)).toHaveLength(0);
	});

	test("a field of a type this app doesn't know turns the form off: no text box for it", async () => {
		await open({
			form: {
				fields: [
					formField("title"),
					formField("area", { data_type: "Polygon", label: "Area" }),
				],
			},
		});
		expect(text(sheet())).toContain(
			"This form has a field this app can't show. Update the app, or open it on the service page.",
		);
		expect(sheet().querySelector("input,textarea")).toBeNull();
		expect(button("Run").hasAttribute("disabled")).toBe(true);
	});

	test("text from the device is plain text: Markdown and links are never rendered", async () => {
		const markdown =
			"**Careful** [docs](https://example.com) <a href='x'>x</a>";
		const { fake, mounted } = await open({
			form: {
				description: markdown,
				fields: [
					formField("title", {
						label: "Title [see](https://example.com)",
						description: "_Short_ <b>title</b>",
					}),
				],
			},
		});
		expect(text(sheet())).toContain(markdown);
		expect(text(sheet())).toContain("Title [see](https://example.com)");
		expect(text(sheet())).toContain("_Short_ <b>title</b>");
		expect(sheet().querySelectorAll("a, strong, em").length).toBe(0);
		const title = control("title");
		if (!title) throw new Error("No title field.");
		await typeInto(title, "x");
		fake.agent(SHOP.device).runs.script = {
			end: { text: "# Done\n[open](https://example.com)" },
		};
		await run(mounted);
		expect(text(sheet().querySelector("[data-run-output] pre"))).toBe(
			"# Done [open](https://example.com)",
		);
		expect(sheet().querySelectorAll("a, h1").length).toBe(0);
	});
});

describe("Run now… · progress and results", () => {
	const ONE_FIELD: FormSpec = { fields: [formField("title")] };

	async function filled(options: OpenOptions = {}) {
		const opened = await open({ form: ONE_FIELD, ...options });
		const title = control("title");
		if (!title) throw new Error("No title field.");
		await typeInto(title, "Hello");
		return opened;
	}

	test("sent, waiting, running with Stop this run, then stopped", async () => {
		const { fake, mounted } = await filled();
		fake.agent(SHOP.device).runs.script = {
			queuedReads: 1,
			runningReads: null,
		};
		await click(button("Run"));
		await until(mounted, "[data-run-state=running]");
		expect(text(sheet())).toMatch(/Running · 0:0\d/);
		expect(text(sheet())).toContain(
			"Closing doesn't stop the run. Its outcome stays under Activity.",
		);
		await click(button("Stop this run"));
		await until(mounted, "[data-run-phase=ended]");
		expect(text(sheet())).toContain("Stopped.");
		const cancels = fake.api.commands.filter(
			([, type]) => type === "cancel_run",
		);
		expect(cancels).toHaveLength(1);
	});

	test("each way a run fails has its own sentence", async () => {
		const cases: [Record<string, unknown>, string][] = [
			[
				{ code: "flow_failed" },
				"The flow failed. People with Logs can read why in shop-assistant's logs.",
			],
			[
				{ code: "interrupted" },
				"shop-assistant stopped, updated or restarted during the run. It isn't started again.",
			],
			[
				{ code: "not_started" },
				"No running instance of shop-assistant took the run within 30 seconds.",
			],
			[
				{ code: "needs_interaction" },
				"The flow asked a question. That can't be answered from here.",
			],
			[{ code: "cancelled" }, "Stopped."],
			[{ code: "timed_out" }, "Stopped after 0:00."],
		];
		const { fake, mounted } = await filled();
		for (const [end, sentence] of cases) {
			fake.agent(SHOP.device).runs.script = { end };
			await run(mounted);
			expect(text(sheet().querySelector("[data-run-result]"))).toContain(
				sentence,
			);
			await click(button("Run again…"));
		}
	});

	test("invalid_fields names the refused fields, and Run again marks them", async () => {
		const { fake, mounted } = await filled();
		fake.agent(SHOP.device).runs.script = {
			end: { code: "invalid_fields", fields: ["title"] },
		};
		await run(mounted);
		expect(text(sheet())).toContain(
			"edge-berlin-01 refused these fields: title.",
		);
		await click(button("Run again…"));
		expect(control("title")?.value).toBe("Hello");
		expect(text(field("title"))).toContain(
			"edge-berlin-01 refused this value.",
		);
	});

	test("a result larger than 8 KiB comes back shortened, and says so", async () => {
		const { fake, mounted } = await filled();
		fake.agent(SHOP.device).runs.script = { end: { text: '"'.repeat(9_000) } };
		await run(mounted);
		expect(text(sheet())).toContain(
			"Shortened: the full result is larger than 8 KiB.",
		);
		const shown = sheet().querySelector("[data-run-output] pre")?.textContent;
		expect(shown?.length).toBeGreaterThan(0);
		expect(shown?.length).toBeLessThan(9_000);
	});

	test("after an agent restart the result is gone: the sheet reads the run again and says so", async () => {
		const { fake, mounted } = await filled({ started: true });
		fake.agent(SHOP.device).runs.script = { end: { output: { id: 42 } } };
		await run(mounted);
		expect(text(sheet())).toContain('"id": 42');
		await click(button("Close"));
		await mounted.settle();
		fake.agent(SHOP.device).restartAgent();
		const row = document.querySelector<HTMLElement>(
			`[data-run-started="${SHOP.form}"]`,
		);
		expect(text(row)).toContain("Return request");
		expect(text(row)).toContain("Succeeded");
		if (!row) throw new Error("No run in Runs you started.");
		await click(byRole("button", "Show result…", row));
		await until(mounted, "[data-run-phase=ended]");
		expect(text(sheet())).toContain(
			"The result is no longer on edge-berlin-01 (its agent restarted).",
		);
		expect(text(sheet())).not.toContain('"id": 42');
	});

	test("refusals before anything runs: busy, a changed service, the command limit", async () => {
		const cases: [string, string][] = [
			[
				"busy",
				"shop-assistant is running as many actions as it accepts. Try again in a moment.",
			],
			[
				"revision_conflict",
				"shop-assistant changed or isn't running. Reload the form.",
			],
			[
				"limit",
				"edge-berlin-01 has recorded as many of your commands today as it accepts. Try again later, or ask the device's owner.",
			],
		];
		for (const [code, sentence] of cases) {
			const { fake, mounted } = await filled();
			fake.agent(SHOP.device).reject("run_event", code, `refused: ${code}`);
			await run(mounted, "[data-run-phase=rejected]");
			expect(text(sheet())).toContain(sentence);
			if (code === "revision_conflict") {
				await click(button("Reload the form"));
				await until(mounted, "[data-run-fields]");
			} else expect(button("Run again…")).toBeTruthy();
			await cleanupDevices();
			await act(async () => useOverlayStore.getState().close());
		}
	});

	test("no reply: it may have run, Check again looks the run up and never sends it twice", async () => {
		const { fake, mounted } = await filled();
		fake.agent(SHOP.device).loseReplyNext("run_event");
		fake.agent(SHOP.device).runs.script = { end: { output: { id: 7 } } };
		await run(mounted, "[data-run-phase=no_reply]");
		expect(text(sheet())).toContain(
			"No reply from edge-berlin-01. The run may have started: check before you run it again.",
		);
		expect(
			fake.workspace.activity
				.list()
				.filter(
					(item) => item.kind === "event_run" && item.state === "unknown",
				),
		).toHaveLength(1);
		await click(button("Check again"));
		await until(mounted, "[data-run-phase=ended]");
		expect(text(sheet())).toContain('"id": 7');
		expect(sentRuns(fake)).toHaveLength(1);
		const items = fake.workspace.activity
			.list()
			.filter((item) => item.kind === "event_run");
		expect(items.map((item) => item.state)).toEqual(["done"]);
	});
});

describe("Run now… · when it can't be run", () => {
	test("each closed gate shows its reason instead of the form", async () => {
		type Reason = [
			string,
			((fake: Fake) => void | Promise<void>) | undefined,
			RegExp,
			Partial<OpenOptions>?,
		];
		const reasons: Reason[] = [
			[
				"the device can't be reached from this computer",
				undefined,
				/connecting to edge-berlin-01|can't be reached|offline/i,
				{ offline: true },
			],
			[
				"the agent is too old",
				(fake) => {
					const agent = fake.agent(SHOP.device);
					const { on_demand_events: _, ...older } = agent.features;
					agent.features = older;
				},
				/agent/i,
			],
			[
				"the service isn't running",
				async (fake) => {
					const placement = fake.agent(SHOP.device).placement(SHOP.service);
					await fake.workspace.live.call(SHOP.device)({
						type: "stop",
						placement_id: SHOP.service,
						expected_revision: placement?.config_revision ?? 0,
					});
				},
				/running/i,
			],
			[
				"the device is locked",
				async (fake) => {
					await fake.workspace.keys.lock(SHOP.device);
				},
				/unlock|locked/i,
			],
		];
		for (const [, arrange, reason, more] of reasons) {
			const { fake } = await open({
				form: EVERY_KIND,
				...(arrange ? { arrange } : {}),
				...more,
			});
			expect(sheet().querySelector("[data-gate]")).not.toBeNull();
			expect(text(sheet().querySelector("[data-gate]"))).toMatch(reason);
			expect(sheet().querySelector("[data-run-fields]")).toBeNull();
			// The notice says why once: Run is off without a second copy of the reason.
			expect(button("Run").hasAttribute("disabled")).toBe(true);
			expect(sheet().querySelectorAll("[data-gate-inline]")).toHaveLength(0);
			await click(button("Run"));
			expect(sentRuns(fake)).toHaveLength(0);
			await cleanupDevices();
			await act(async () => useOverlayStore.getState().close());
		}
	});
});
