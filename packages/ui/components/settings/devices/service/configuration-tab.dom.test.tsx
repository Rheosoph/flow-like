import { afterAll, afterEach, describe, expect, test } from "bun:test";
import {
	byRole,
	click,
	inPortal,
	installDom,
	keyDown,
	queryByRole,
	typeInto,
} from "../testing/dom-harness";

const dom = installDom();
const { cleanupDevices, mountDevices, preloadDevices } = await import(
	"../testing/mount-devices"
);
await preloadDevices();
const { rejected } = await import("../testing/fake-device-api");
const { formatTimeOfDay } = await import("../../../../lib/date");
const { useOverlayStore } = await import("../workspace/overlay-store");
const { ConfigUnavailable } = await import("./config-parts");
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
type ServiceConfigRead = import("./use-service-config").ServiceConfigRead;
type Config = Record<string, unknown>;

afterEach(async () => {
	await cleanupDevices();
	await dom.cleanup();
	useOverlayStore.getState().close();
});
afterAll(dom.restore);

/** Safe updates poll the device once a second until the new settings are healthy. */
const SLOW = 20_000;

const summary = (view: View) =>
	view.container.querySelector<HTMLElement>("[data-config-summary]");
const actions = (view: View) =>
	view.container.querySelector<HTMLElement>(
		"[data-config-actions]",
	) as HTMLElement;
const act = (view: View, id: string) =>
	view.container.querySelector<HTMLElement>(
		`[data-act=config-${id}]`,
	) as HTMLElement;

async function open(
	device: string,
	service: string,
	options: Partial<Parameters<Kit["openTab"]>[0]> = {},
) {
	const view = await openTab({
		tab: "configuration",
		device,
		service,
		...options,
	});
	return view;
}

async function ready(view: View) {
	await until(() => summary(view) !== null);
}

const fieldNotes = (options: Partial<Parameters<Kit["openTab"]>[0]> = {}) =>
	open(STUDIO, "field-notes", options);

/** Edit settings → change the request timeout → Review. */
async function reviewTimeout(view: View, seconds = "90") {
	await click(act(view, "edit"));
	const sheet = inPortal("dialog");
	await typeInto(
		sheet.querySelector("#svc-edit-timeout") as HTMLElement,
		seconds,
	);
	await click(byRole("button", "Review changes", sheet));
	return inPortal("dialog");
}

const quick = (sheet: HTMLElement) => byRole("radio", /Quick update/, sheet);

/** The workspace clock as the page writes it ("14:01:15"). */
const clock = (view: View) =>
	formatTimeOfDay(view.fake.clock.now(), { locale: "en", seconds: true });

describe("summary", () => {
	test("settings read live: events by name, endpoint, write buffering, how it runs and their size", async () => {
		const view = await fieldNotes();
		await ready(view);
		const page = text(view.container);
		expect(byRole("heading", /Settings v9/, view.container)).toBeTruthy();
		expect(page).toContain("Notes page");
		expect(page).toContain("event 1.2.0");
		expect(page).toContain("flow 3.0.1");
		expect(page).toContain("127.0.0.1:8090 · only this device");
		expect(page).toContain("None · plain HTTP");
		expect(page).toContain("the service page only");
		expect(page).toContain("Max instances");
		expect(page).toContain("table records in project storage");
		expect(page).toContain("queue 256 MiB");
		expect(page).toContain("max age 7 days");
		expect(page).toContain("Runs online · data stays in the cloud");
		expect(page).toMatch(/\d+ of 12,000 bytes/);
		expect(page).toContain("Settings over 12,000 bytes can't be applied.");
		expect(
			view.container.querySelector("[data-stamp][data-src=live]"),
		).not.toBeNull();
		expect(primaries()).toBeLessThanOrEqual(1);
		expect(page).not.toMatch(MACHINE);
		expect(sent(view, STUDIO)).toContain("placement_configuration");
		expect(writes(view)).toEqual([]);
	});

	test("a stored value shows with its type; a stored secret is never shown", async () => {
		const view = await fieldNotes({
			arrange: (fake) =>
				patchConfig(fake, STUDIO, "field-notes", (config) => {
					config.variables = { region: "eu-central", retries: 3 };
					config.secret_overrides = { erp_password: "variable-7f3a" };
				}),
		});
		await ready(view);
		const page = text(summary(view) as HTMLElement);
		expect(page).toContain("eu-central (Text)");
		expect(page).toContain("3 (Number)");
		expect(page).toContain("stored secret · can't be read back");
		expect(page).not.toContain("variable-7f3a");
		expect(
			byRole("button", "Change secret value…", view.container),
		).toBeTruthy();
	});

	test("what was read reaches the rest of the area, and a fact the settings no longer carry goes", async () => {
		const known = (view: View) =>
			view.fake.workspace.facts.get(STUDIO)?.placements?.["field-notes"];
		const view = await fieldNotes({
			arrange: async (fake) => {
				expect(
					fake.workspace.facts.get(STUDIO)?.placements?.["field-notes"]
						?.offlineWrites,
				).toEqual({ maxAgeS: 604_800, maxBytes: 268_435_456 });
				await patchConfig(fake, STUDIO, "field-notes", (config) => {
					config.offline_writes = undefined;
				});
			},
		});
		await ready(view);
		await until(() => known(view)?.offlineWrites === undefined);
		expect(known(view)).toEqual({
			host: "127.0.0.1",
			port: 8_090,
			tlsCertificateId: null,
			resourceGrantId: "6e2f1a9c-3b7d-4e05-8a1c-9d4b2f6e0a73",
		});
	});

	test("an offline copy says where its data lives and that nothing is buffered", async () => {
		const view = await open(EDGE, "support-bot");
		await ready(view);
		const page = text(view.container);
		expect(page).toContain("Offline copy · data lives only on edge-berlin-01");
		expect(page).toContain("0.0.0.0:8443 · all networks");
		expect(page).toContain(
			"Off · an offline copy keeps its data on the device",
		);
		expect(page).toContain("Support chat");
		expect(page).not.toMatch(MACHINE);
	});
});

describe("states without settings", () => {
	test("a locked device: the unlock gate, no read and no danger action", async () => {
		const view = await fieldNotes({ unlock: "none" });
		const page = text(view.container);
		expect(page).toMatch(/Unlock studio-mac-mini/);
		expect(summary(view)).toBeNull();
		expect(sent(view)).not.toContain("placement_configuration");
		expect(primaries()).toBeLessThanOrEqual(1);
	});

	test("an offline device: settings aren't known, nothing is sent", async () => {
		const view = await open(WAREHOUSE, "scanner-ingest");
		const page = text(view.container);
		expect(page).toContain("Settings aren't known");
		expect(page).toContain("warehouse-pi is offline");
		expect(page).not.toContain("Nothing here yet");
		expect(sent(view, WAREHOUSE)).not.toContain("placement_configuration");
		const remove = byRole("button", "Remove service…", view.container);
		expect(remove.getAttribute("aria-disabled")).toBe("true");
		await click(remove);
		expect(queryByRole("alertdialog")).toBeNull();
		expect(writes(view)).toEqual([]);
	});

	test("a device that refuses the read: needs Deploy & configure, with a request to copy", async () => {
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
		await until(() =>
			text(view.container).includes("Needs Deploy & configure on this service"),
		);
		const page = text(view.container);
		expect(page).toContain(
			"Needs Deploy & configure on this service to read its settings.",
		);
		expect(view.container.querySelector("[role=alert]")).toBeNull();
		await click(
			byRole("button", "Copy a request for the owner", view.container),
		);
		expect(dom.clipboard.at(-1)).toContain(
			"Deploy & configure for field-notes on studio-mac-mini",
		);
		expect(
			sent(view, STUDIO).filter((type) => type === "placement_configuration")
				.length,
		).toBe(1);
	});

	test("a device that turns the read down for another reason is a failed read with its sentence, not a matter of access", async () => {
		let lift: () => void = () => undefined;
		const view = await fieldNotes({
			arrange: (fake) => {
				lift = fake
					.agent(STUDIO)
					.reject(
						"placement_configuration",
						"busy",
						"The device database is busy.",
					);
			},
		});
		await until(() =>
			text(view.container).includes(
				"The settings couldn't be read from studio-mac-mini",
			),
		);
		const page = text(view.container);
		expect(page).toContain(
			"studio-mac-mini answered: “The device database is busy.” Nothing was changed.",
		);
		expect(page).not.toContain("Needs Deploy & configure");
		expect(view.container.querySelector("[data-gate=noaccess]")).toBeNull();
		expect(page).not.toMatch(MACHINE);
		lift();
		await click(byRole("button", "Try again", view.container));
		await ready(view);
		expect(text(view.container)).not.toContain("couldn't be read");
	});

	test("access withdrawn after a read: the settings leave the screen and the gate takes their place", async () => {
		const view = await fieldNotes();
		await ready(view);
		expect(text(view.container)).toContain("127.0.0.1:8090");
		view.fake
			.agent(STUDIO)
			.reject(
				"placement_configuration",
				"unauthorized",
				"Deploy capability required.",
			);
		setTimeout(
			() =>
				void view.fake.queryClient.refetchQueries({
					predicate: (query) => query.queryKey[2] === "service-config",
				}),
			0,
		);
		await until(() =>
			text(view.container).includes("Needs Deploy & configure on this service"),
		);
		expect(summary(view)).toBeNull();
		expect(text(view.container)).not.toContain("127.0.0.1:8090");
		expect(writes(view)).toEqual([]);
	});

	test("a viewer whose access lacks Deploy & configure reads which permissions they have", async () => {
		const read: ServiceConfigRead = {
			device: undefined,
			deviceLabel: "lab-gpu-02",
			service: undefined,
			unavailable: undefined,
			gate: {
				ok: false,
				gate: "G5",
				kind: "noaccess",
				hide: false,
				copy: { code: "needs_capability", params: { scope: "this service" } },
				have: ["status", "logs"],
				need: ["deploy"],
			},
			configuration: undefined,
			readAt: undefined,
			loading: false,
			refused: undefined,
			failed: false,
			freshness: { src: "live", age: "notloaded" },
			refresh: async () => undefined,
		};
		const view = await mountDevices(
			<ConfigUnavailable read={read} serviceId="invoice-extractor-gpu" />,
		);
		const page = text(view.container);
		expect(page).toContain(
			"Needs Deploy & configure on this service to read its settings.",
		);
		expect(page).toContain("You have View status and Read logs.");
		await click(
			byRole("button", "Copy a request for the owner", view.container),
		);
		expect(dom.clipboard.at(-1)).toContain(
			"Deploy & configure for invoice-extractor-gpu on lab-gpu-02",
		);
		expect(page).not.toMatch(MACHINE);
	});
});

describe("action row", () => {
	test("while an update runs every action is disabled with one sentence, and a click sends nothing", async () => {
		const view = await open(EDGE, "invoice-extractor");
		await ready(view);
		const row = actions(view);
		const line = row.querySelector("[data-gate-inline=busy]");
		expect(text(line as HTMLElement)).toMatch(
			/^An update is in progress\. Edit settings, Update, Add an event,? and Edit as JSON work again after it finishes \(by \d\d:\d\d:\d\d at the latest\)\.$/,
		);
		expect(row.querySelectorAll("[data-gate-inline]").length).toBe(1);
		expect(row.querySelector("[data-config-hint]")).toBeNull();
		for (const id of ["edit", "update", "add", "json"]) {
			const button = act(view, id);
			expect(button.getAttribute("aria-disabled")).toBe("true");
			expect(button.getAttribute("aria-describedby")).toBe(
				line?.getAttribute("id") ?? "missing",
			);
			await click(button);
		}
		expect(queryByRole("dialog")).toBeNull();
		expect(view.navigations).toEqual([]);
		expect(writes(view)).toEqual([]);
		expect(primaries()).toBe(0);
	});

	test("behind the published versions: the note names the pins, Update and Add an event open the wizard", async () => {
		const view = await open(EDGE, "invoice-extractor");
		await ready(view);
		const note = view.container.querySelector(
			"[data-update-note]",
		) as HTMLElement;
		expect(note.dataset.updateNote).toBe("behind");
		expect(text(note)).toContain(
			"re-pins invoice-extractor to the event and flow versions published now: Extract invoice 1.4.0 → 1.5.0 (flow 2.1.0 → 2.2.0). Data stays in the cloud and isn't touched.",
		);
		expect(text(summary(view) as HTMLElement)).toContain("Behind");
	});

	test("on the published versions: Update… links to the wizard for this service; nothing more can be added", async () => {
		const view = await fieldNotes();
		await ready(view);
		const update = act(view, "update");
		expect(update.tagName).toBe("A");
		const href = update.getAttribute("href") ?? "";
		expect(href).toContain("flow=deploy");
		expect(href).toContain(`device=${STUDIO}`);
		expect(href).toContain("service=field-notes");
		expect(href).toContain("app=app_field_notes");
		const add = act(view, "add");
		expect(add.getAttribute("aria-disabled")).toBe("true");
		expect(text(actions(view))).toContain(
			"Add an event: Every event of Field Notes that can run on a device is already served here.",
		);
		// Another action's reason doesn't take the place of what Edit settings does.
		expect(
			text(actions(view).querySelector("[data-config-hint]") as HTMLElement),
		).toBe(
			"Edit settings goes through a safe or quick update and keeps the installed app version.",
		);
		expect(text(actions(view))).toContain(
			"Runs the event and flow versions published now.",
		);
		expect(primaries()).toBe(1);
	});

	test("in an app's settings the wizard links stay in that app", async () => {
		const view = await fieldNotes({ appId: "app_field_notes" });
		await ready(view);
		const href = act(view, "update").getAttribute("href") ?? "";
		expect(href).toContain("/library/config/devices?id=app_field_notes");
		expect(href).toContain("mode=update");
		expect(href).toContain("service=field-notes");
	});
});

describe("edit settings", () => {
	const stored = (config: Config) => {
		config.variables = { greeting: "Welcome from this device" };
		config.secret_overrides = { credential: "stored-credential" };
		config.future_setting = { retain_me: true };
		config.restart = {
			initial_backoff_secs: 2,
			max_backoff_secs: 60,
			max_restarts: 5,
		};
	};

	test("a quick update sends one command with the revision it read and keeps every stored reference", async () => {
		const view = await fieldNotes({
			arrange: (fake) => patchConfig(fake, STUDIO, "field-notes", stored),
		});
		await ready(view);
		const hubWrites = view.fake.api.writes().length;
		const sheet = await reviewTimeout(view);
		expect(text(sheet)).toContain("settings v9 → v10");
		expect(text(sheet)).toMatch(/Request timeout\s*60 s\s*→.*90 s/);
		expect(primaries(sheet)).toBe(1);
		await click(quick(sheet));
		expect(text(sheet)).toContain(
			"The service stops, then starts with the new settings.",
		);
		await click(byRole("button", "Apply", sheet));
		await until(() => commandsOf(view, "apply").length === 1);
		const [command] = commandsOf(view, "apply");
		expect(command).toMatchObject({ expected_revision: 9, start: true });
		const config = command?.config as Config;
		const hosting = config.hosting as Config;
		expect(hosting.request_timeout_secs).toBe(90);
		expect(hosting.auth_secret).toBe("field-notes-auth");
		expect(hosting.port).toBe(8090);
		expect(config.variables).toEqual({ greeting: "Welcome from this device" });
		expect(config.secret_overrides).toEqual({
			credential: "stored-credential",
		});
		expect(config.future_setting).toEqual({ retain_me: true });
		expect(config.restart).toEqual({
			initial_backoff_secs: 2,
			max_backoff_secs: 60,
			max_restarts: 5,
		});
		expect((config.resource_grant as Config).grant_id).toBe(
			"6e2f1a9c-3b7d-4e05-8a1c-9d4b2f6e0a73",
		);
		expect(config.offline_writes).toBeTruthy();
		expect(writes(view)).toEqual(["apply"]);
		expect(view.fake.api.writes().length).toBe(hubWrites);
		await until(() =>
			text(view.container).includes("restarted with settings v10"),
		);
		expect(queryByRole("dialog")).toBeNull();
	});

	test(
		"a safe update is the default: it stages the settings with the chosen checks and reports the end",
		async () => {
			const view = await fieldNotes();
			await ready(view);
			const sheet = await reviewTimeout(view, "120");
			await typeInto(
				sheet.querySelector("#svc-apply-stable") as HTMLElement,
				"20",
			);
			const clickedAt = clock(view);
			await click(byRole("button", "Apply", sheet));
			await until(() => commandsOf(view, "activate_rollout").length === 1);
			// The device takes its time: the end is reported with the time it ended, not the time of the click.
			view.fake.clock.advance(75_000);
			const [stage] = commandsOf(view, "stage_rollout");
			expect(stage).toMatchObject({
				expected_revision: 9,
				stabilization_seconds: 20,
				deadline_seconds: 120,
			});
			expect(
				((stage?.config as Config).hosting as Config).request_timeout_secs,
			).toBe(120);
			expect(commandsOf(view, "apply")).toEqual([]);
			expect(commandsOf(view, "set_secret")).toEqual([]);
			await until(
				() => text(view.container).includes("runs settings v10"),
				10_000,
			);
			expect(clock(view)).not.toBe(clickedAt);
			expect(text(view.container)).toContain(
				`field-notes runs settings v10. Updated at ${clock(view)}.`,
			);
			const tray = view.fake.workspace.activity
				.list()
				.find(
					(item) =>
						item.kind === "safe_update" &&
						item.target.serviceId === "field-notes",
				);
			expect(tray?.state).toBe("done");
		},
		SLOW,
	);

	test("the timing of a safe update is checked before anything is sent", async () => {
		const view = await fieldNotes();
		await ready(view);
		const sheet = await reviewTimeout(view);
		await typeInto(
			sheet.querySelector("#svc-apply-deadline") as HTMLElement,
			"12",
		);
		await click(byRole("button", "Apply", sheet));
		expect(text(sheet)).toContain(
			"Time limit to start (12 s) must be more than 5 s longer than Must stay healthy for (10 s).",
		);
		expect(writes(view)).toEqual([]);
	});

	test("nothing changed and out-of-range values stop at step one", async () => {
		const view = await fieldNotes();
		await ready(view);
		await click(act(view, "edit"));
		const sheet = inPortal("dialog");
		await click(byRole("button", "Review changes", sheet));
		expect(text(sheet)).toContain(
			"Nothing changed yet. Change a value, or cancel.",
		);
		const port = sheet.querySelector("#svc-edit-port") as HTMLElement;
		expect(port.getAttribute("aria-invalid")).toBeNull();
		await typeInto(port, "0");
		expect(text(sheet)).toContain("Port is 0. Use 1 to 65535.");
		// The error belongs to its field for assistive tech too, not only by position.
		expect(port.getAttribute("aria-invalid")).toBe("true");
		expect(
			text(
				sheet.querySelector(
					`[id="${(port.getAttribute("aria-describedby") ?? "").split(" ")[0]}"]`,
				) as HTMLElement,
			),
		).toBe("Port is 0. Use 1 to 65535.");
		await click(byRole("button", "Review changes", sheet));
		expect(text(inPortal("dialog"))).toContain("Step 1 of 2 · Change");
		expect(writes(view)).toEqual([]);
	});

	test("write buffering locks the instance count and says why", async () => {
		const view = await fieldNotes();
		await ready(view);
		await click(act(view, "edit"));
		const sheet = inPortal("dialog");
		const max = sheet.querySelector("#svc-edit-max") as HTMLInputElement;
		expect(max.disabled).toBe(true);
		expect(text(sheet)).toContain(
			"Write buffering needs exactly one instance.",
		);
		expect(
			text(
				sheet.querySelector(
					`[id="${max.getAttribute("aria-describedby")}"]`,
				) as HTMLElement,
			),
		).toBe("Write buffering needs exactly one instance.");
		expect(text(sheet)).toContain(
			"To change the app version, use Update; to serve another event, use Add an event.",
		);
	});

	test("settings that changed on the device block the apply until they are reloaded and reviewed again", async () => {
		const view = await fieldNotes({
			arrange: (fake) =>
				patchConfig(fake, STUDIO, "field-notes", () => undefined),
		});
		await ready(view);
		const agent = view.fake.agent(STUDIO);
		const restore = agent.handle("apply", () => {
			restore();
			const row = agent.placement("field-notes");
			if (row) row.config_revision = 10;
			const held = agent.configs.get("field-notes") as Config;
			(held.hosting as Config).max_in_flight = 48;
			return rejected(
				"revision_conflict",
				"Placement field-notes changed on the device.",
			);
		});
		let sheet = await reviewTimeout(view);
		await click(quick(sheet));
		await click(byRole("button", "Apply", sheet));
		await until(() =>
			text(inPortal("dialog")).includes(
				"The settings changed on studio-mac-mini",
			),
		);
		sheet = inPortal("dialog");
		const apply = byRole("button", "Apply", sheet);
		expect(apply.getAttribute("aria-disabled")).toBe("true");
		await click(apply);
		expect(commandsOf(view, "apply").length).toBe(1);
		await click(byRole("button", "Reload settings", sheet));
		await until(() => text(inPortal("dialog")).includes("Settings v10"));
		sheet = inPortal("dialog");
		expect(
			(sheet.querySelector("#svc-edit-inflight") as HTMLInputElement).value,
		).toBe("48");
		expect(
			(sheet.querySelector("#svc-edit-timeout") as HTMLInputElement).value,
		).toBe("90");
		await click(byRole("button", "Review changes", sheet));
		sheet = inPortal("dialog");
		expect(text(sheet)).toContain("settings v10 → v11");
		await click(quick(sheet));
		await click(byRole("button", "Apply", sheet));
		await until(() => commandsOf(view, "apply").length === 2);
		expect(
			commandsOf(view, "apply").map((command) => command.expected_revision),
		).toEqual([9, 10]);
		const second = commandsOf(view, "apply")[1]?.config as Config;
		expect((second.hosting as Config).max_in_flight).toBe(48);
		expect((second.hosting as Config).request_timeout_secs).toBe(90);
		const ids = [...agent.journal.entries()]
			.filter(([, reply]) => reply.type === "apply")
			.map(([id]) => id);
		expect(new Set(ids).size).toBe(2);
	});

	test("a definitive refusal shows the device's reason and keeps the sheet", async () => {
		const view = await fieldNotes({
			arrange: (fake) => {
				fake
					.agent(STUDIO)
					.reject("apply", "invalid", "Port 8090 is already bound.");
			},
		});
		await ready(view);
		const sheet = await reviewTimeout(view);
		await click(quick(sheet));
		await click(byRole("button", "Apply", sheet));
		await until(() =>
			text(inPortal("dialog")).includes("Port 8090 is already bound."),
		);
		expect(text(inPortal("dialog"))).toContain("The device refused the change");
		expect(commandsOf(view, "apply").length).toBe(1);
	});

	test("the sheet waits for the device's answer: Close and Esc are held while a quick update is on the way", async () => {
		const view = await fieldNotes({
			arrange: (fake) => {
				fake
					.agent(STUDIO)
					.reject("apply", "invalid", "Port 8090 is already bound.");
			},
		});
		await ready(view);
		const sheet = await reviewTimeout(view);
		await click(quick(sheet));
		const answer = view.fake.agent(STUDIO).hold("apply");
		await click(byRole("button", "Apply", sheet));
		await until(() => commandsOf(view, "apply").length === 1);
		await click(byRole("button", "Close", inPortal("dialog")));
		await keyDown(inPortal("dialog"), "Escape");
		expect(queryByRole("dialog")).not.toBeNull();
		answer();
		await until(() =>
			text(inPortal("dialog")).includes("Port 8090 is already bound."),
		);
		await click(byRole("button", "Close", inPortal("dialog")));
		expect(queryByRole("dialog")).toBeNull();
		expect(commandsOf(view, "apply").length).toBe(1);
	});
});

describe("stored values and their definitions", () => {
	const describeVariables =
		(items: Record<string, unknown>[]) =>
		(command: Record<string, unknown>) => {
			const request = command.request as Record<string, unknown>;
			return {
				state: "completed",
				result: {
					project_id: request.project_id,
					revision: request.revision,
					event_id: request.event_id,
					items: request.event_id === "evt_support_chat" ? items : [],
					next: null,
				},
			};
		};
	const sandbox = {
		profile: "linux_sandbox",
		cpu_millis: 2000,
		memory_bytes: 2 * 1024 ** 3,
		max_processes: 256,
		disk_bytes: 4 * 1024 ** 3,
	};
	const variable = (
		id: string,
		name: string,
		dataType: string,
		rest: Record<string, unknown> = {},
	) => ({
		id,
		name,
		data_type: dataType,
		value_type: "Normal",
		secret: false,
		...rest,
	});

	const supportBot = (
		items: Record<string, unknown>[],
		patch: (config: Config) => void,
	) =>
		open(EDGE, "support-bot", {
			arrange: async (fake) => {
				fake.agent(EDGE).handle("artifact", describeVariables(items));
				await patchConfig(fake, EDGE, "support-bot", (config) => {
					config.resources = sandbox;
					patch(config);
				});
			},
		});

	test("names and types come from the version the service runs", async () => {
		const view = await supportBot(
			[
				variable("credential", "API credential", "String", { secret: true }),
				variable("greeting", "Greeting", "String"),
			],
			(config) => {
				config.variables = { greeting: "Hello" };
				config.secret_overrides = { credential: "stored-credential" };
			},
		);
		await ready(view);
		await until(() => text(summary(view) as HTMLElement).includes("Greeting"));
		const page = text(summary(view) as HTMLElement);
		expect(page).toContain("Hello (Text)");
		expect(page).toContain("API credential");
		expect(page).toContain("Sandboxed · 2 cores");
	});

	test("removed and incompatible stored values must be replaced or removed before review", async () => {
		const view = await supportBot(
			[
				variable("credential", "API credential", "String", { secret: true }),
				variable("greeting", "Greeting", "Boolean"),
			],
			(config) => {
				config.variables = { greeting: "Welcome", obsolete: "old value" };
				config.secret_overrides = { credential: "stored-credential" };
			},
		);
		await ready(view);
		await until(
			() => summary(view)?.querySelector("[data-issue=unused]") !== null,
		);
		expect(
			summary(view)?.querySelector("[data-issue=incompatible]"),
		).not.toBeNull();
		await click(act(view, "edit"));
		let sheet = inPortal("dialog");
		await click(byRole("button", "Review changes", sheet));
		sheet = inPortal("dialog");
		expect(text(sheet)).toContain("Step 1 of 2 · Change");
		expect(text(sheet)).toContain(
			"isn't used by this service's events any more. Remove it to continue.",
		);
		expect(text(sheet)).toContain("Enter a new value or remove it.");
		const row = (id: string) =>
			sheet.querySelector(`[data-field-id="var.${id}"]`) as HTMLElement;
		await click(byRole("button", "Use the app's default", row("obsolete")));
		expect(text(row("obsolete"))).toContain(
			"The stored value is removed; the app's default applies.",
		);
		await click(byRole("combobox", undefined, row("greeting")));
		await click(byRole("option", "Yes"));
		await click(byRole("button", "Review changes", inPortal("dialog")));
		sheet = inPortal("dialog");
		expect(text(sheet)).toContain("Step 2 of 2 · Apply");
		await click(quick(sheet));
		await click(byRole("button", "Apply", sheet));
		await until(() => commandsOf(view, "apply").length === 1);
		const config = commandsOf(view, "apply")[0]?.config as Config;
		expect(config.variables).toEqual({ greeting: true });
		expect(config.secret_overrides).toEqual({
			credential: "stored-credential",
		});
		expect(commandsOf(view, "set_secret")).toEqual([]);
	});
});

describe("secrets", () => {
	const withSecret = (definitions?: Record<string, unknown>) =>
		fieldNotes({
			arrange: (fake) =>
				patchConfig(fake, STUDIO, "field-notes", (config) => {
					config.secret_overrides = { erp_password: "variable-7f3a" };
				}),
			...(definitions
				? {
						mount: {
							backend: {
								eventState: {
									getEvents: async () => [],
									getEventAuthoritative: async () => ({
										id: "evt_notes_http",
										board_id: "board_notes",
									}),
								},
								boardState: {
									getBoardAuthoritative: async () => ({
										variables: { erp_password: definitions },
										layers: {},
									}),
								},
							} as never,
						},
					}
				: {}),
		});

	test("a new value travels in one encrypted command under the stored reference and is tracked", async () => {
		const view = await withSecret();
		await ready(view);
		const values: unknown[] = [];
		const agent = view.fake.agent(STUDIO);
		agent.handle("set_secret", (command) => {
			values.push(command.value);
			return {
				state: "completed",
				result: {
					placement_id: command.placement_id,
					name: command.name,
					secret: "completed",
				},
			};
		});
		await click(act(view, "secret"));
		const sheet = inPortal("dialog");
		expect(text(sheet)).toContain("allowed while it runs");
		await click(byRole("button", "Save new value", sheet));
		expect(text(sheet)).toContain("Enter the new value first.");
		expect(writes(view)).toEqual([]);
		await typeInto(
			sheet.querySelector("#svc-secret-value") as HTMLElement,
			"s3cret value",
		);
		await click(byRole("button", "Save new value", sheet));
		await until(() => commandsOf(view, "set_secret").length === 1);
		const [command] = commandsOf(view, "set_secret");
		expect(command).toMatchObject({
			placement_id: "field-notes",
			expected_revision: 9,
			name: "variable-7f3a",
		});
		expect(values).toEqual([JSON.stringify("s3cret value")]);
		expect(JSON.stringify(view.fake.api.commands)).not.toContain("s3cret");
		expect(JSON.stringify(view.fake.api.calls)).not.toContain("s3cret");
		expect(writes(view)).toEqual(["set_secret"]);
		await until(() => text(view.container).includes("New value for"));
		const item = view.fake.workspace.activity
			.list()
			.find((entry) => entry.kind === "secret_write");
		expect(item?.target.serviceId).toBe("field-notes");
		expect(JSON.stringify(item)).not.toContain("s3cret");
	});

	test("a value that doesn't fit the secret's type is refused before it is sent", async () => {
		const view = await withSecret({
			id: "erp_password",
			name: "ERP password",
			data_type: "String",
			value_type: "HashMap",
			secret: true,
			exposed: true,
		});
		await ready(view);
		await until(() =>
			text(summary(view) as HTMLElement).includes("ERP password"),
		);
		await click(act(view, "secret"));
		const sheet = inPortal("dialog");
		expect(text(sheet)).toContain("ERP password");
		expect(text(sheet)).toContain("enter it as JSON");
		await typeInto(
			sheet.querySelector("#svc-secret-value") as HTMLElement,
			"plain text",
		);
		await click(byRole("button", "Save new value", sheet));
		expect(text(sheet)).toContain("That doesn't fit ERP password.");
		expect(writes(view)).toEqual([]);
		await typeInto(
			sheet.querySelector("#svc-secret-value") as HTMLElement,
			'{"user":"erp","password":"pw"}',
		);
		await click(byRole("button", "Save new value", sheet));
		await until(() => commandsOf(view, "set_secret").length === 1);
	});
});

describe("edit as JSON", () => {
	test("preloaded with references, counted in bytes, and refused over the limit before anything is sent", async () => {
		const view = await fieldNotes({
			arrange: (fake) =>
				patchConfig(fake, STUDIO, "field-notes", (config) => {
					config.secret_overrides = { erp_password: "variable-7f3a" };
				}),
		});
		await ready(view);
		await click(act(view, "json"));
		const sheet = inPortal("dialog");
		const area = sheet.querySelector("#svc-json-text") as HTMLTextAreaElement;
		expect(area.value).toContain('"erp_password": "variable-7f3a"');
		expect(text(sheet)).toContain("secrets appear as references, never values");
		expect(text(sheet)).toMatch(/\d+ of 12,000 bytes, as sent to the device/);
		await click(byRole("button", "Apply", sheet));
		expect(text(sheet)).toContain("Nothing changed yet.");
		const config = JSON.parse(area.value) as Config;
		await typeInto(
			area,
			JSON.stringify({ ...config, padding: "x".repeat(12_000) }),
		);
		expect(
			Number(
				(sheet.querySelector("[data-json-bytes]") as HTMLElement).dataset
					.jsonBytes,
			),
		).toBeGreaterThan(12_000);
		await click(byRole("button", "Apply", sheet));
		expect(text(sheet)).toMatch(
			/The settings are [\d,]+ bytes\. The device accepts up to 12,000\./,
		);
		await typeInto(area, JSON.stringify({ ...config, id: "renamed" }));
		await click(byRole("button", "Apply", sheet));
		expect(text(sheet)).toContain("The service's identity can't change");
		await typeInto(area, "{ not json");
		await click(byRole("button", "Apply", sheet));
		expect(text(sheet)).toContain("That isn't valid JSON");
		expect(writes(view)).toEqual([]);
	});

	test("turning buffering off waits until the queue is empty", async () => {
		const view = await fieldNotes();
		await ready(view);
		await click(act(view, "json"));
		const sheet = inPortal("dialog");
		const area = sheet.querySelector("#svc-json-text") as HTMLTextAreaElement;
		const { offline_writes: _writes, ...rest } = JSON.parse(
			area.value,
		) as Config;
		await typeInto(area, JSON.stringify(rest));
		expect(text(sheet)).toContain("Write buffering");
		await click(quick(sheet));
		await click(byRole("button", "Apply", sheet));
		await until(() =>
			text(inPortal("dialog")).includes("17 buffered changes still wait."),
		);
		expect(commandsOf(view, "apply")).toEqual([]);
		view.fake.agent(STUDIO).offlineQueues["field-notes"] = [];
		await click(byRole("button", "Apply", inPortal("dialog")));
		await until(() => commandsOf(view, "apply").length === 1);
		expect(
			(commandsOf(view, "apply")[0]?.config as Config).offline_writes,
		).toBeUndefined();
	});
});

describe("danger zone", () => {
	const GRANT = "6e2f1a9c-3b7d-4e05-8a1c-9d4b2f6e0a73";
	const LIMIT = "0d4b7c1e-52a9-4f3b-9c66-1e8a7d2b5f90";
	const zone = (view: View) =>
		view.container.querySelector("#svc-danger") as HTMLElement;
	const revoked = (view: View) =>
		view.fake.api.sent("DELETE").map(([, path]) => path);

	/** Remove service… → the typed ID and the boxes to tick → the confirm button. */
	async function confirmRemoval(
		view: View,
		service: string,
		tick: readonly string[] = [],
	) {
		await click(byRole("button", "Remove service…", view.container));
		const sheet = inPortal("alertdialog");
		await typeInto(
			sheet.querySelector("#dv-confirm-typed") as HTMLElement,
			service,
		);
		for (const id of tick)
			await click(sheet.querySelector(`#${id}`) as HTMLElement);
		await click(byRole("button", new RegExp(`emove ${service}$`), sheet));
	}

	test("a stopped service is removed after its ID is typed", async () => {
		const view = await open(EDGE, "nightly-sync");
		await ready(view);
		await click(byRole("button", "Remove service…", view.container));
		const sheet = inPortal("alertdialog");
		expect(text(sheet)).toContain("Its ID is reserved forever.");
		expect(text(sheet)).toContain("No, this is permanent.");
		expect(text(sheet)).toContain("Nothing else: it has no cloud access.");
		expect(sheet.querySelector("#svc-remove-revoke")).toBeNull();
		expect(sheet.querySelector("#svc-remove-lose")).toBeNull();
		const confirm = byRole("button", "Remove nightly-sync", sheet);
		expect(confirm.getAttribute("aria-disabled")).toBe("true");
		await click(confirm);
		expect(writes(view)).toEqual([]);
		await typeInto(
			sheet.querySelector("#dv-confirm-typed") as HTMLElement,
			"nightly-sync",
		);
		await click(byRole("button", "Remove nightly-sync", sheet));
		await until(() => commandsOf(view, "remove").length === 1);
		expect(writes(view)).toEqual(["remove"]);
		expect(commandsOf(view, "remove")[0]).toMatchObject({
			placement_id: "nightly-sync",
			expected_revision: 3,
		});
		await until(() => view.navigations.length === 1);
		expect(view.navigations[0]?.href).toContain("tab=services");
		expect(revoked(view)).toEqual([]);
	});

	test("a running service offers Stop and remove, and buffered changes that would be lost must be acknowledged", async () => {
		const view = await fieldNotes();
		await ready(view);
		expect(text(view.container)).toContain(
			"It has to stop first; the next step offers Stop and remove.",
		);
		await click(byRole("button", "Remove service…", view.container));
		const sheet = inPortal("alertdialog");
		expect(text(sheet)).toContain("It's running. Confirming stops it first");
		expect(text(sheet)).toContain(
			"17 buffered changes haven't reached the cloud. Removing discards them.",
		);
		expect(text(sheet)).toContain(
			"Its service page at 127.0.0.1:8090 stops answering.",
		);
		expect(text(sheet)).toContain("Cloud access stays unless you revoke it.");
		await typeInto(
			sheet.querySelector("#dv-confirm-typed") as HTMLElement,
			"field-notes",
		);
		await click(byRole("button", "Stop and remove field-notes", sheet));
		expect(text(sheet)).toContain("Tick the box under “Do this first”");
		expect(writes(view)).toEqual([]);
		await click(sheet.querySelector("#svc-remove-lose") as HTMLElement);
		await click(byRole("button", "Stop and remove field-notes", sheet));
		await until(() => commandsOf(view, "remove").length === 1);
		expect(writes(view)).toEqual(["stop", "remove"]);
		expect(revoked(view)).toEqual([]);
		await until(() => view.navigations.length === 1);
		expect(view.navigations[0]?.href).toContain("tab=services");
		expect(text(document.body)).not.toMatch(MACHINE);
	});

	test("the removal waits until the device reports the service stopped", async () => {
		const view = await fieldNotes();
		await ready(view);
		const agent = view.fake.agent(STUDIO);
		agent.handle("stop", (command) => {
			const row = agent.placement(command.placement_id);
			if (row) row.desired_state = "stopped";
			return { state: "completed", result: { intent_recorded: true } };
		});
		await confirmRemoval(view, "field-notes", ["svc-remove-lose"]);
		await until(() => commandsOf(view, "stop").length === 1);
		await until(() => text(zone(view)).includes("field-notes is stopping."));
		expect(commandsOf(view, "remove")).toEqual([]);
		expect(
			byRole("button", "Remove service…", zone(view)).getAttribute("aria-busy"),
		).toBe("true");
		const row = agent.placement("field-notes");
		if (row) row.observed_state = "stopped";
		await until(() => commandsOf(view, "remove").length === 1);
		expect(writes(view)).toEqual(["stop", "remove"]);
	});

	test("asked to, it revokes the spending limit and the cloud access before it removes the service", async () => {
		const view = await fieldNotes({
			arrange: (fake) => {
				fake.hub.resourcesOf(STUDIO).billing.push({
					billing_grant_id: LIMIT,
					grant_id: GRANT,
					payer_id: fake.hub.me,
					authz_version: 1,
					limit_micros: 5_000_000,
					used_micros: 0,
					reserved_micros: 0,
					expires_at: 1_793_275_200,
					status: "active",
				});
			},
		});
		await ready(view);
		const agent = view.fake.agent(STUDIO);
		const before: string[][] = [];
		agent.handle("remove", () => {
			before.push(revoked(view));
			return { state: "completed", result: { removed: true } };
		});
		await click(byRole("button", "Remove service…", view.container));
		expect(text(inPortal("alertdialog"))).toContain(
			"Also revoke field-notes's cloud access and spending limit",
		);
		await click(byRole("button", "Cancel", inPortal("alertdialog")));
		await confirmRemoval(view, "field-notes", [
			"svc-remove-lose",
			"svc-remove-revoke",
		]);
		await until(() => commandsOf(view, "remove").length === 1);
		expect(before).toEqual([
			[
				`devices/${STUDIO}/billing-grants/${LIMIT}`,
				`devices/${STUDIO}/resource-grants/${GRANT}`,
			],
		]);
		expect(writes(view)).toEqual(["stop", "remove"]);
		const access = view.fake.hub.resourcesOf(STUDIO);
		expect(access.grants.map((grant) => grant.status)).toEqual(["revoked"]);
		expect(access.billing.map((limit) => limit.status)).toEqual(["revoked"]);
		await until(() => view.navigations.length === 1);
	});

	test("a revoke the hub doesn't take keeps the service and says so", async () => {
		const view = await fieldNotes();
		await ready(view);
		view.fake.api.fail({ method: "DELETE", path: /resource-grants/ });
		await confirmRemoval(view, "field-notes", [
			"svc-remove-lose",
			"svc-remove-revoke",
		]);
		await until(() =>
			text(zone(view)).includes(
				"Its cloud access wasn't revoked, so field-notes wasn't removed. It stays stopped.",
			),
		);
		expect(writes(view)).toEqual(["stop"]);
		expect(view.navigations).toEqual([]);
		expect(zone(view).querySelector("[data-result=critical]")).not.toBeNull();
		expect(text(zone(view))).not.toMatch(MACHINE);
	});

	test("Cancel sends nothing", async () => {
		const view = await open(EDGE, "nightly-sync");
		await ready(view);
		await click(byRole("button", "Remove service…", view.container));
		await click(byRole("button", "Cancel", inPortal("alertdialog")));
		expect(writes(view)).toEqual([]);
		expect(revoked(view)).toEqual([]);
	});
});

describe("older hub and older agent", () => {
	test("an older hub: the tab renders from the device and asks the hub for app placements once", async () => {
		const view = await fieldNotes({ hubVersion: "old" });
		await ready(view);
		expect(text(view.container)).toContain("Settings v9");
		expect(view.container.querySelector("[role=alert]")).toBeNull();
		const asked = view.fake.api
			.sent("GET", /app.*device|device.*placements/)
			.map(([, path]) => path);
		expect(new Set(asked).size).toBe(asked.length);
	});

	test("an older agent: settings and actions work with the commands it knows", async () => {
		const view = await fieldNotes({ agentFeatures: {} });
		await ready(view);
		expect(text(view.container)).toContain("Settings v9");
		expect(view.container.querySelector("[role=alert]")).toBeNull();
		const known = new Set([
			"inspect_page",
			"placement_configuration",
			"offline_queue",
			"certificates",
			"metrics",
			"rollout",
			"operation",
		]);
		expect(sent(view, STUDIO).filter((type) => !known.has(type))).toEqual([]);
	});
});
