import { afterAll, afterEach, describe, expect, test } from "bun:test";
import {
	APPS,
	HASH,
} from "../../../../lib/device-management/model/__fixtures__/apps";
import {
	SAMPLE_IDS,
	SAMPLE_NOW,
	emptyInput,
	fleetState,
	keySession,
	observation,
	placement,
	sampleFleet,
} from "../../../../lib/device-management/model/__fixtures__/sample-fleet";
import { generateFleet } from "../../../../lib/device-management/model/__fixtures__/sample-fleet-200";
import type { AppInput } from "../../../../lib/device-management/model/app-plan";
import type { IBackendState } from "../../../../state/backend-state";
import {
	advance,
	allByRole,
	byRole,
	click,
	inPortal,
	installDom,
	queryByRole,
	typeInto,
} from "../testing/dom-harness";
import type { DeviceSeed } from "../testing/fake-device-api";
import type { MountDevicesOptions } from "../testing/mount-devices";

const dom = installDom();
const { cleanupDevices, mountDevices, preloadDevices } = await import(
	"../testing/mount-devices"
);
await preloadDevices();
const { act, useState } = await import("react");
const { AppDevicesScreen } = await import("./app-devices-screen");
const { useDevicesRoute } = await import("../routing/use-devices-route");
const { useActivityTray } = await import("../shell/activity-tray");
const { useOverlayStore } = await import("../workspace/overlay-store");

afterEach(async () => {
	await cleanupDevices();
	await dom.cleanup();
	useActivityTray.getState().setOpen(false);
	useOverlayStore.getState().close();
});
afterAll(dom.restore);

const INVOICE = "app_invoice_ai";
const SUPPORT = "app_support_portal";
const CRM = "app_crm_sync";
const SCANNER = "app_warehouse_scan";
const NOTES = "app_field_notes";
const VISITOR = "app_visitor_checkin";
const PARTNER = "app_partner_reports";
const ID = SAMPLE_IDS;

const apps = APPS as Record<string, AppInput>;

/** The real backend's `getApp` carries the app's version text and change time; the fake's doesn't. */
function appBackend(
	extra: Partial<IBackendState> = {},
): Partial<IBackendState> {
	const appOf = (id: string) => {
		const app = apps[id];
		if (!app) throw new Error(`no app ${id}`);
		return app;
	};
	return {
		appState: {
			getApps: async () =>
				Object.values(apps).map((app) => [
					{ id: app.id, visibility: app.visibility },
					{ name: app.name, description: "" },
				]),
			getApp: async (id: string) => {
				const app = appOf(id);
				const newest = app.versions?.[0];
				return {
					id: app.id,
					visibility: app.visibility,
					version: newest?.label?.replace(/^v/, "") ?? null,
					updated_at: {
						secs_since_epoch: newest?.builtAt ?? 0,
						nanos_since_epoch: 0,
					},
				};
			},
			getAppMeta: async (id: string) => ({
				name: appOf(id).name,
				description: "",
			}),
		} as unknown as IBackendState["appState"],
		...extra,
	};
}

function Routed() {
	const { route, scope } = useDevicesRoute();
	if (scope.kind !== "app") return null;
	return <AppDevicesScreen route={route} scope={scope} appId={scope.appId} />;
}

const mountApp = (
	appId: string,
	options: MountDevicesOptions & { query?: string } = {},
) => {
	const { query, ...rest } = options;
	return mountDevices(<Routed />, {
		host: "app",
		search: `id=${appId}${query ? `&${query}` : ""}`,
		backend: appBackend(),
		...rest,
	});
};

/** lab-gpu-02 unlocked: its encrypted status shows the shared app's service, two versions back. */
function labUnlockedSeed() {
	const seed = sampleFleet();
	seed.fleet[ID.lab] = fleetState(ID.lab, {
		status: {
			observations: [
				observation(
					ID.lab,
					[
						placement({
							id: "invoice-extractor-gpu",
							project_id: INVOICE,
							revision: HASH.invoice13,
							desired_state: "running",
							observed_state: "running",
							config_revision: 4,
							applied_revision: 4,
							source: "online",
							events: [
								{
									event_id: "evt_gpu_extract",
									event_version: [1, 0, 2],
									board_version: [1, 3, 0],
								},
							],
						}),
					],
					SAMPLE_NOW - 21,
					"1ab000000000001a",
				),
			],
			observedAt: SAMPLE_NOW - 21,
			bootId: "1ab000000000001a",
			sequence: 12,
		},
		reader: { revision: 1, expiresAt: SAMPLE_NOW + 300 * 86_400 },
	});
	seed.keys = seed.keys.map((session) =>
		session.deviceId === ID.lab
			? keySession(ID.lab, "unlocked", "shared", session.grantId)
			: session,
	);
	return seed;
}

const primaries = (root: ParentNode) =>
	root.querySelectorAll("[data-dv-primary]").length;

/** Whether a node was found. Never hand a DOM node to `expect`: printing one on failure crashes bun. */
const present = (node: unknown) => node !== null && node !== undefined;

/** Rendered words without ids, names and code: no wire value or gate code may be among them (R3). */
function prose(root: HTMLElement): string {
	const copy = root.cloneNode(true) as HTMLElement;
	for (const node of copy.querySelectorAll(
		"[data-idref], code, .font-mono, [data-person]",
	))
		node.remove();
	return copy.textContent ?? "";
}
const MACHINE_WORDS = /\b[a-z]+_[a-z_]+\b|\bG\d{1,2}\b|\bD\d\b/;

const block = (container: HTMLElement, id: string) => {
	const found = container.querySelector<HTMLElement>(`#${id}`);
	if (!found) throw new Error(`no block #${id}`);
	return found;
};

const text = (el: Element | null | undefined) => el?.textContent ?? "";
const attr = (el: Element | null | undefined, name: string) =>
	el?.getAttribute(name) ?? null;
const layout = (container: HTMLElement) =>
	attr(container.querySelector("[data-app-devices]"), "data-app-devices");

const hrefOf = (name: string | RegExp, root?: ParentNode | null) =>
	attr(byRole("link", name, root ?? undefined), "href") ?? "";

const lastNavigation = (navigations: { mode: string; href: string }[]) =>
	navigations.at(-1);

const appUrl = (appId: string, rest = "") =>
	`/library/config/devices?id=${appId}${rest}`;

describe("App › Devices · normal (online app, one update running)", () => {
	test("header, strip, headline and coverage", async () => {
		const { container } = await mountApp(INVOICE);
		expect(byRole("heading", "Devices").tagName).toBe("H1");
		expect(text(container.querySelector("[data-app-header]"))).toContain(
			"Where Invoice AI runs, on devices you can see",
		);
		expect(hrefOf("All devices")).toBe("/settings/devices");
		expect(text(container.querySelector("[data-idref]"))).toContain(INVOICE);
		const strip = container.querySelector("[data-mode]");
		expect(attr(strip, "data-mode")).toBe("online");
		expect(text(strip)).toContain("Runs online");
		expect(text(strip)).toContain("Newestv1.5.0");
		expect(text(container.querySelector("[data-headline]"))).toContain(
			"invoice-extractor on edge-berlin-01 is switching to settings v12.",
		);
		const coverage = container.querySelector("[data-app-coverage]");
		expect(text(coverage)).toContain("Status from 3 of 5 devices you can see");
		expect(text(coverage)).toContain("1 unknown");
		expect(text(coverage)).toContain("1 hasn't checked in");
		expect(text(coverage)).toContain(
			"Partial access on lab-gpu-02: you see Invoice AI only.",
		);
		expect(primaries(container)).toBe(1);
		expect(
			present(
				byRole("link", "Deploy to devices…", container).closest(
					"[data-dv-primary]",
				),
			),
		).toBe(true);
		expect(prose(container)).not.toMatch(MACHINE_WORDS);
	});

	test("every block states its source and age", async () => {
		const { container } = await mountApp(INVOICE);
		// App metrics is W3-OBS's block: only this page's own blocks are listed.
		const own = [
			"ad-prog",
			"ad-where",
			"ad-versions",
			"ad-else",
			"ad-spending",
			"ad-access",
		];
		const blocks = [...container.querySelectorAll("[data-block]")].filter(
			(entry) => own.includes(entry.id),
		);
		expect(blocks.map((entry) => entry.id)).toEqual(own);
		expect(
			blocks.map((entry) =>
				present(entry.querySelector("header [data-stamp]")),
			),
		).toEqual(blocks.map(() => true));
		// A stamp with its own words still carries the age (R5).
		expect(
			text(
				block(container, "ad-versions").querySelector("header [data-stamp]"),
			),
		).toContain("published versions · checked");
	});

	test("the coverage line unlocks the one locked device", async () => {
		const { container, fake } = await mountApp(INVOICE);
		const before = fake.api.calls.length;
		await click(
			byRole(
				"button",
				"Unlock lab-gpu-02…",
				container.querySelector("[data-app-coverage]") ?? undefined,
			),
		);
		expect(useOverlayStore.getState().overlay).toEqual({
			kind: "unlock",
			deviceId: ID.lab,
		});
		expect(fake.api.calls.length).toBe(before);
	});

	test("in progress lists the running update with Follow and Open service", async () => {
		const { container } = await mountApp(INVOICE);
		const progress = block(container, "ad-prog");
		expect(text(progress)).toContain(
			"Safe update · invoice-extractor on edge-berlin-01 · Switching over",
		);
		expect(text(progress)).toContain("Settings v11 → v12.");
		expect(hrefOf("Open service", progress)).toBe(
			appUrl(
				INVOICE,
				`&device=${ID.edge}&service=invoice-extractor&tab=status`,
			),
		);
		await click(byRole("button", "Follow", progress));
		expect(useActivityTray.getState().open).toBe(true);
	});

	test("by device: one service row and one locked device, never 'not deployed'", async () => {
		const { container } = await mountApp(INVOICE);
		const where = block(container, "ad-where");
		const row = where.querySelector('[data-service="invoice-extractor"]');
		expect(text(row)).toContain("Extract invoice");
		expect(text(row)).toContain("127.0.0.1:8081");
		expect(text(row)).toContain("0 of 1 ready · max 1");
		expect(attr(row?.querySelector("[data-drift]"), "data-drift")).toBe(
			"behind",
		);
		expect(text(row)).toContain("In the cloud");
		expect(text(row)).toContain("Buffers writes · nothing waiting");
		expect(text(row)).toContain("€7.41 of €25");
		expect(text(row)).toContain("Files: read & write");
		expect(text(row)).toContain("Safe update · switching");
		expect(hrefOf("invoice-extractor", where)).toContain(
			"service=invoice-extractor&tab=status",
		);
		expect(hrefOf("edge-berlin-01", where)).toBe(
			appUrl(INVOICE, `&device=${ID.edge}&tab=services`),
		);
		const state = where.querySelector("[data-state-row]");
		expect(text(state)).toContain(
			"Unlock lab-gpu-02 to see which Invoice AI services run there.",
		);
		expect(present(state?.querySelector('[data-kind="locked"]'))).toBe(true);
		expect(text(where)).not.toContain("Not deployed");
		const foot = text(where.querySelector("[data-where-foot]"));
		expect(foot).toContain("Newest version v1.5.0");
		expect(foot).toContain("isn't running anywhere yet.");
	});

	test("versions: newest first, drift from the pins, Roll out… only on the newest", async () => {
		const { container } = await mountApp(INVOICE);
		const versions = block(container, "ad-versions");
		const rows = [...versions.querySelectorAll("[data-version]")];
		expect(rows).toHaveLength(2);
		expect(text(rows[0])).toContain("v1.5.0");
		expect(text(rows[0])).toContain("Newest");
		expect(text(rows[0])).toContain("3 events");
		// The same wording as the strip above it ("built today 13:00").
		expect(text(rows[0])).toMatch(/Built today \d\d:\d\d/);
		expect(text(container.querySelector("[data-where-foot]"))).toMatch(
			/built today \d\d:\d\d, isn't running anywhere yet\./,
		);
		expect(text(rows[0])).toContain("Extract invoice 1.4.0 → 1.5.0");
		expect(text(rows[0])).toContain("lab-gpu-02 · unknown until unlocked");
		expect(present(queryByRole("button", "Roll out…", rows[0]))).toBe(true);
		expect(present(queryByRole("button", "Roll out…", rows[1]))).toBe(false);
		expect(text(rows[1])).toContain("Can't be prepared again.");
		expect(text(rows[1])).toContain("invoice-extractor");
	});

	test("everywhere else: honest groups and the verbatim note", async () => {
		const { container } = await mountApp(INVOICE);
		const elsewhere = block(container, "ad-else");
		const groups = [...elsewhere.querySelectorAll("section[data-group]")].map(
			(group) => group.getAttribute("data-group"),
		);
		expect(groups).toEqual(["not-deployed", "unknown", "never"]);
		expect(text(elsewhere)).toContain("Runs field-notes (Field Notes).");
		expect(text(elsewhere)).toContain(
			"Registered 12 minutes ago and hasn't sent any status yet, so nothing runs there.",
		);
		expect(text(elsewhere.querySelector("[data-else-note]"))).toBe(
			"This view never counts a device as not deployed unless its status was readable.",
		);
		expect(text(elsewhere.querySelector("[data-revoked]"))).toBe(
			"old-kiosk and partner-edge are revoked, so they aren't read.",
		);
		expect(hrefOf("Start instructions", elsewhere)).toContain(
			`device=${ID.cold}`,
		);
	});

	test("a device that can't take a deploy keeps its row, disabled, with the reason", async () => {
		const { container, fake, navigations } = await mountApp(INVOICE);
		const row = block(container, "ad-else").querySelector(
			`li[data-device="${ID.warehouse}"]`,
		);
		if (!row) throw new Error("no warehouse row");
		expect(text(row)).toContain("needs a live connection");
		const deploy = byRole("button", "Deploy here", row);
		expect(deploy.getAttribute("aria-disabled")).toBe("true");
		expect(
			byRole("checkbox", "Select warehouse-pi", row).hasAttribute("disabled"),
		).toBe(true);
		const before = fake.api.calls.length;
		await click(deploy);
		expect(navigations).toEqual([]);
		expect(fake.api.calls.length).toBe(before);
		await click(byRole("button", "Diagnose", row));
		expect(useOverlayStore.getState().overlay).toMatchObject({
			kind: "diagnose",
			deviceId: ID.warehouse,
		});
	});

	test("ticking devices offers one deploy for all of them, not in coral", async () => {
		const { container } = await mountApp(INVOICE);
		const elsewhere = block(container, "ad-else");
		// One device is ready: "several at once" has nothing to offer.
		expect(text(elsewhere)).not.toContain("several at once");
		await click(byRole("checkbox", "Select studio-mac-mini", elsewhere));
		const link = byRole("link", "Deploy to 1 selected…", elsewhere);
		const target = appUrl(INVOICE, `&flow=deploy&mode=new&device=${ID.studio}`);
		expect(link.getAttribute("href")).toBe(target);
		expect(present(link.closest("[data-dv-primary]"))).toBe(false);
		expect(hrefOf("Deploy here", elsewhere)).toBe(target);
	});

	test("cloud access & spending: the approval, its meter and both revokes", async () => {
		const { container } = await mountApp(INVOICE);
		const spending = block(container, "ad-spending");
		expect(text(spending.querySelector("h2"))).toContain(
			"Cloud access & spending",
		);
		const approval = spending.querySelector("[data-approval]");
		expect(attr(approval, "data-approval")).toBe(
			`${ID.edge}/invoice-extractor`,
		);
		expect(text(approval)).toContain("Active");
		expect(text(approval)).toContain("Read & write");
		expect(text(approval)).toContain("€7.41 used");
		expect(present(queryByRole("button", "Revoke approval…", spending))).toBe(
			true,
		);
		expect(
			present(queryByRole("button", "Revoke spending limit…", spending)),
		).toBe(true);
	});

	test("revoking an approval asks first and sends one request", async () => {
		const { container, fake } = await mountApp(INVOICE);
		const before = fake.api.writes().length;
		await click(
			byRole("button", "Revoke approval…", block(container, "ad-spending")),
		);
		const sheet = inPortal();
		expect(text(sheet)).toContain(
			"invoice-extractor's instances lose cloud access within minutes.",
		);
		expect(fake.api.writes().length).toBe(before);
		await click(byRole("button", /Revoke the cloud access/, sheet));
		const sent = fake.api.writes().slice(before);
		expect(sent.map(([method]) => method)).toEqual(["DELETE"]);
		expect(sent[0][1]).toContain("resource-grants/");
	});

	test("access to this app: people per device with presets", async () => {
		const { container } = await mountApp(INVOICE);
		const access = block(container, "ad-access");
		const rows = [...access.querySelectorAll("[data-grant]")];
		expect(rows.length).toBe(3);
		expect(text(rows[0])).toContain("Whole device");
		expect(text(rows[0])).toContain("Viewer · 3 permissions");
		expect(text(rows[1])).toContain("App Invoice AI");
		expect(text(rows[2])).toContain("You");
		expect(hrefOf("All access", access)).toBe(
			"/settings/devices?view=access&tab=people",
		);
	});

	test("access to this app: rules state, how soon it ends, and where to change it", async () => {
		const { container } = await mountApp(INVOICE);
		const access = block(container, "ad-access");
		expect(text(access.querySelector("header [data-stamp]"))).toContain(
			"access rules · checked",
		);
		const head = [...access.querySelectorAll("thead th")].map((cell) =>
			text(cell),
		);
		expect(head).toEqual([
			"Person",
			"Device",
			"Applies to",
			"Permissions",
			"Ends",
			"Actions",
		]);
		const rows = [...access.querySelectorAll("[data-grant]")];
		// Someone else's access on a device of the viewer: the rules the device runs, and the device's Access tab.
		expect(text(rows[0])).toMatch(/Active · rules v\d+/);
		expect(present(rows[0].querySelector("[data-ends-soon]"))).toBe(true);
		expect(present(rows[1].querySelector("[data-ends-soon]"))).toBe(false);
		expect(hrefOf("Open device access", rows[0])).toBe(
			appUrl(INVOICE, `&device=${ID.edge}&tab=access`),
		);
		// The viewer's own access to a shared device can only be asked for.
		expect(text(rows[2])).not.toContain("rules v");
		expect(hrefOf("Ask to renew", rows[2])).toBe(
			"/settings/devices?view=access&tab=shared&action=request",
		);
		expect(primaries(access)).toBe(0);
	});
});

describe("App › Devices · header actions", () => {
	test("Deploy one event… lists eligible events first, the others with their reason", async () => {
		const { container } = await mountApp(INVOICE);
		await click(byRole("button", /Deploy one event…/, container));
		const menu = inPortal("menu");
		const items = allByRole("menuitem", undefined, menu).map((item) =>
			text(item),
		);
		expect(items.slice(0, 3)).toEqual([
			"Extract invoiceon 1 device",
			"Extract invoice (GPU)not on a device",
			"Invoice tools (MCP)not on a device",
		]);
		expect(items.at(-1)).toBe("Manage events");
		const blocked = allByRole("menuitem", /Nightly reconciliation/, menu)[0];
		expect(blocked.getAttribute("aria-disabled")).toBe("true");
		expect(text(blocked)).toContain("Schedules can't run on a device yet.");
		expect(
			allByRole("menuitem", /Extract invoice \(GPU\)/, menu)[0].getAttribute(
				"href",
			),
		).toBe(appUrl(INVOICE, "&flow=deploy&mode=new&event=evt_gpu_extract"));
		expect(
			allByRole("menuitem", "Manage events", menu)[0].getAttribute("href"),
		).toBe(`/library/config/events?id=${INVOICE}`);
	});

	test("More: Refresh status re-reads the hub and says when", async () => {
		const { container, fake, settle } = await mountApp(INVOICE);
		const lists = fake.api.sent("GET", "devices").length;
		await click(byRole("button", "More", container));
		await click(allByRole("menuitem", "Refresh status", inPortal("menu"))[0]);
		await settle();
		expect(fake.api.sent("GET", "devices").length).toBeGreaterThan(lists);
		expect(text(container.querySelector("[data-app-header]"))).toContain(
			"Refreshed at",
		);
	});

	test("More: Refresh status never says 'refreshed' when the hub didn't answer", async () => {
		const { container, fake, settle } = await mountApp(INVOICE);
		fake.api.fail({ method: "GET", path: "devices" });
		await click(byRole("button", "More", container));
		await click(allByRole("menuitem", "Refresh status", inPortal("menu"))[0]);
		// The device list is tried again twice before the read counts as failed.
		await advance(3500);
		await settle();
		const header = text(container.querySelector("[data-app-header]"));
		expect(header).toContain("The hub didn't answer at");
		expect(header).not.toContain("Refreshed at");
	}, 15_000);

	test("More: Copy app ID and Show in Fleet overview", async () => {
		const { container } = await mountApp(INVOICE);
		await click(byRole("button", "More", container));
		const menu = inPortal("menu");
		expect(
			allByRole("menuitem", "Show in Fleet overview", menu)[0].getAttribute(
				"href",
			),
		).toBe("/settings/devices?view=services&q=Invoice+AI");
		await click(allByRole("menuitem", "Copy app ID", menu)[0]);
		expect(dom.clipboard.at(-1)).toBe(INVOICE);
	});

	test("Deploy to devices… opens the app-first deploy flow", async () => {
		const { container, navigations } = await mountApp(INVOICE);
		const deploy = byRole("link", "Deploy to devices…", container);
		const target = appUrl(INVOICE, "&flow=deploy&mode=new");
		expect(deploy.getAttribute("href")).toBe(target);
		await click(deploy);
		expect(lastNavigation(navigations)).toEqual({ mode: "push", href: target });
	});
});

describe("App › Devices · by event", () => {
	test("the matrix: served, not served with Deploy here, unknown until unlocked", async () => {
		const { container, navigations } = await mountApp(INVOICE, {
			query: "by=event",
		});
		const matrix = container.querySelector("[data-by-event]");
		if (!matrix) throw new Error("no matrix");
		const heads = [...matrix.querySelectorAll("th[data-device]")].map((th) =>
			th.getAttribute("data-device"),
		);
		expect(heads).toEqual([ID.edge, ID.lab]);
		const extract = matrix.querySelector('[data-event="evt_extract_http"]');
		expect(text(extract)).toContain("Extract invoice");
		expect(text(extract)).toContain("Web request");
		expect(text(extract)).toContain("event 1.5.0 · flow 2.2.0");
		expect(text(extract)).toContain(
			"Served by the device · checks its web server",
		);
		const served = extract?.querySelector('[data-matrix-cell="served"]');
		expect(text(served)).toContain("invoice-extractor");
		expect(text(served)).toContain("1.4.0 · flow 2.1.0");
		expect(text(served)).toContain("→ 1.5.0");
		expect(text(served)).toContain(":8081");
		expect(
			attr(extract?.querySelector('[data-matrix-cell="unknown"]'), "data-why"),
		).toBe("locked");
		const gpu = matrix.querySelector('[data-event="evt_gpu_extract"]');
		const notServed = gpu?.querySelector('[data-matrix-cell="not_served"]');
		expect(text(notServed)).toContain("Not served");
		expect(text(container.querySelector("#ad-where footer"))).toContain(
			"Unknown never means not deployed.",
		);
		await click(byRole("button", "Deploy here", notServed ?? undefined));
		expect(lastNavigation(navigations)?.href).toBe(
			appUrl(
				INVOICE,
				`&flow=deploy&mode=new&device=${ID.edge}&event=evt_gpu_extract`,
			),
		);
	});

	test("the served cell opens the service inside the area", async () => {
		const { container, navigations } = await mountApp(INVOICE, {
			query: "by=event",
		});
		const cell = container.querySelector('[data-matrix-cell="served"]');
		await click(byRole("link", /invoice-extractor/, cell ?? undefined));
		expect(lastNavigation(navigations)).toEqual({
			mode: "push",
			href: appUrl(
				INVOICE,
				`&device=${ID.edge}&service=invoice-extractor&tab=status`,
			),
		});
	});

	test("Can't run on devices is collapsed and gives the plain reason with its fix", async () => {
		const { container } = await mountApp(INVOICE, { query: "by=event" });
		const toggle = byRole("button", "Can't run on devices · 3", container);
		expect(toggle.getAttribute("aria-expanded")).toBe("false");
		expect(
			present(container.querySelector('[data-event="evt_invoice_review"]')),
		).toBe(false);
		await click(toggle);
		const review = container.querySelector('[data-event="evt_invoice_review"]');
		expect(text(review)).toContain("Follows the latest flow edits.");
		expect(hrefOf("Pin a flow version in Events", review)).toBe(
			`/library/config/events?id=${INVOICE}&event=evt_invoice_review`,
		);
		expect(text(review)).toContain("Not offered when you deploy");
	});

	test("switching the view replaces the URL; event= marks its row", async () => {
		const { container, navigations } = await mountApp(INVOICE, {
			query: "by=event&event=evt_invoice_mcp",
		});
		expect(
			attr(
				container.querySelector('[data-event="evt_invoice_mcp"]'),
				"data-target",
			),
		).toBe("true");
		await click(byRole("tab", "By device", container));
		expect(lastNavigation(navigations)?.mode).toBe("replace");
		expect(lastNavigation(navigations)?.href).not.toContain("by=event");
	});
});

describe("App › Devices · shared device unlocked", () => {
	test("its service appears with an older version and hidden cloud access", async () => {
		const { container } = await mountApp(INVOICE, { seed: labUnlockedSeed() });
		const where = block(container, "ad-where");
		const row = where.querySelector('[data-service="invoice-extractor-gpu"]');
		expect(text(row)).toContain("Extract invoice (GPU)");
		expect(text(row)).toContain("2 behind");
		expect(text(row)).toContain(
			"Hidden: only the approver and the device owner see it",
		);
		expect(present(where.querySelector("[data-state-row]"))).toBe(false);
		expect(text(block(container, "ad-spending"))).toContain(
			"invoice-extractor-gpu on lab-gpu-02: approved by someone else; only they and the device owner see the details.",
		);
		expect(text(container.querySelector("[data-app-coverage]"))).toContain(
			"Status from 4 of 5",
		);
		expect(
			[...block(container, "ad-versions").querySelectorAll("[data-version]")]
				.length,
		).toBe(3);
	});

	test("only this app's services are listed; a later status replaces what was observed", async () => {
		const { container, fake, settle } = await mountApp(INVOICE, {
			seed: labUnlockedSeed(),
		});
		const where = block(container, "ad-where");
		const services = () =>
			[...where.querySelectorAll("[data-service]")].map((row) =>
				row.getAttribute("data-service"),
			);
		// edge-berlin-01 also runs support-bot and nightly-sync: other apps' services never show here.
		expect(services()).toEqual(["invoice-extractor", "invoice-extractor-gpu"]);
		expect(text(where)).not.toContain("support-bot");
		expect(text(where)).not.toContain("nightly-sync");

		const agent = fake.agent(ID.edge);
		agent.placements = agent.placements.filter(
			(row) => row.id !== "invoice-extractor",
		);
		fake.hub.publishStatus(ID.edge, agent);
		await click(byRole("button", "More", container));
		await click(allByRole("menuitem", "Refresh status", inPortal("menu"))[0]);
		await settle();
		expect(services()).toEqual(["invoice-extractor-gpu"]);
		const notDeployed = block(container, "ad-else").querySelector(
			'[data-group="not-deployed"]',
		);
		expect(
			present(notDeployed?.querySelector(`[data-device="${ID.edge}"]`)),
		).toBe(true);
	});
});

describe("App › Devices · local-only app", () => {
	test("offline copy: data on the device, model access, this computer's versions", async () => {
		const { container } = await mountApp(SUPPORT);
		const strip = container.querySelector("[data-mode]");
		expect(attr(strip, "data-mode")).toBe("offline");
		expect(text(strip)).toContain("Local only");
		expect(text(strip)).toContain("Offline copy");
		const row = container.querySelector('[data-service="support-bot"]');
		expect(text(row)).toContain("Support chat · Support API");
		expect(text(row)).toContain("On the device");
		expect(text(row)).toContain("Since: unknown");
		expect(text(row)).toContain("No cloud access");
		expect(text(row)).toContain("Settings v7 · date unknown");
		expect(text(row)).toContain("1 behind");
		const spending = block(container, "ad-spending");
		expect(text(spending.querySelector("h2"))).toContain(
			"Model access & spending",
		);
		expect(text(spending)).toContain("No model access");
		expect(text(spending)).toContain(
			"Local-only apps can't get access to cloud files: their data isn't on the hub.",
		);
		const versions = block(container, "ad-versions");
		expect(
			attr(versions.querySelector("header [data-stamp]"), "data-src"),
		).toBe("local");
		expect(text(versions)).toContain("Changed on this computer");
		expect(text(block(container, "ad-where"))).toContain(
			"Changes on this computer reach a device only when you update it.",
		);
	});

	test("a device shared for another app is 'no access', never unknown or not deployed", async () => {
		const { container } = await mountApp(SUPPORT);
		const elsewhere = block(container, "ad-else");
		const noAccess = elsewhere.querySelector('[data-group="no-access"]');
		expect(text(noAccess)).toContain("lab-gpu-02");
		expect(text(noAccess)).toContain(
			"so Support Portal can't be seen there, even after unlocking.",
		);
		expect(hrefOf("Request access", noAccess)).toBe(
			"/settings/devices?view=access&tab=shared&action=request",
		);
		expect(present(elsewhere.querySelector('[data-group="unknown"]'))).toBe(
			false,
		);
		expect(
			present(block(container, "ad-where").querySelector("[data-state-row]")),
		).toBe(false);
		expect(text(container.querySelector("[data-app-coverage]"))).toContain(
			"Your access to lab-gpu-02 doesn't cover Support Portal.",
		);
	});

	test("on the web a local-only app is a platform gate and reads no device", async () => {
		const { container, fake } = await mountApp(SUPPORT, { platform: "web" });
		expect(layout(container)).toBe("web-local");
		expect(text(container)).toContain(
			"Support Portal only exists on the computer that created it",
		);
		expect(present(container.querySelector("[data-block]"))).toBe(false);
		expect(fake.api.sent("GET", /device-placements/)).toEqual([]);
	});
});

describe("App › Devices · staged update", () => {
	const stagedSeed = () => {
		const seed = sampleFleet();
		const live = seed.live[ID.edge];
		live.rollouts = [
			...(live.rollouts ?? []),
			{
				rollout_id: "5f0e3a52-0c0c-4a0b-9d5e-0a1b2c3d4e5f",
				placement_id: "support-bot",
				project_id: SUPPORT,
				state: "staged",
				failure_code: null,
				base_revision: 7,
				created_at: SAMPLE_NOW - 900,
				updated_at: SAMPLE_NOW - 900,
				deadline_at: null,
				stable_since: null,
			},
		];
		return seed;
	};

	test("In progress offers Activate… and Discard…; Update everywhere waits for it", async () => {
		const { container } = await mountApp(SUPPORT, { seed: stagedSeed() });
		const progress = block(container, "ad-prog");
		const row = progress.querySelector('[data-progress="staged"]');
		expect(text(row)).toContain("Update ready · support-bot on edge-berlin-01");
		expect(text(row)).toContain("not active yet");
		expect(text(row)).toContain("Discarded automatically on");
		expect(present(queryByRole("button", "Activate…", row ?? undefined))).toBe(
			true,
		);
		expect(present(queryByRole("button", "Discard…", row ?? undefined))).toBe(
			true,
		);
		expect(
			text(container.querySelector('[data-service="support-bot"]')),
		).toContain("Update staged");
		const update = byRole("button", "Update everywhere…", container);
		expect(update.getAttribute("aria-disabled")).toBe("true");
		expect(text(container.querySelector("[data-app-header]"))).toContain(
			"No service can take an update right now. An update to v2.4.0 is staged. Activate or discard it first.",
		);
	});

	test("Activate… shows what switches and sends one command after the confirm", async () => {
		const { container, fake, settle } = await mountApp(SUPPORT, {
			seed: stagedSeed(),
		});
		const before = fake.api.commands.length;
		await click(byRole("button", "Activate…", block(container, "ad-prog")));
		const sheet = inPortal();
		expect(text(sheet)).toContain(
			"The current version keeps serving until the new one is healthy.",
		);
		expect(fake.api.commands.length).toBe(before);
		await click(byRole("button", /Activate the update of support-bot/, sheet));
		await settle();
		expect(
			fake.api.commands
				.slice(before)
				.map(([, type]) => type)
				.filter((type) => type === "activate_rollout"),
		).toEqual(["activate_rollout"]);
	});
});

describe("App › Devices · crashing on an offline device", () => {
	test("last known, attention for this app, events unknown from the snapshot", async () => {
		const { container } = await mountApp(SCANNER);
		expect(text(container.querySelector("[data-headline]"))).toContain(
			"scanner-ingest on warehouse-pi kept crashing when last seen.",
		);
		const attention = block(container, "ad-attn");
		expect(text(attention.querySelector("h2"))).toContain("Needs attention");
		expect(text(attention.querySelector("h2"))).toContain("3");
		expect(present(attention.querySelector("header [data-stamp]"))).toBe(true);
		const row = container.querySelector('[data-service="scanner-ingest"]');
		expect(text(row?.querySelector('[data-events="unknown"]'))).toBe(
			"Events unknown: the status snapshot has no event list",
		);
		expect(text(row)).toContain("last known");
		expect(attr(row?.querySelector("[data-drift]"), "data-drift")).toBe(
			"unknown",
		);
		const update = byRole("button", "Update everywhere…", container);
		expect(update.getAttribute("aria-disabled")).toBe("true");
		expect(text(container.querySelector("[data-app-header]"))).toContain(
			"No service can take an update right now.",
		);
		// A service whose version can't be told is never "not running anywhere".
		const runs = text(
			block(container, "ad-versions").querySelector("[data-ver-runs]"),
		);
		expect(runs).toContain("Not running on any service whose version is known");
		expect(runs).toContain("warehouse-pi › scanner-ingest · version unknown");
		expect(runs).not.toContain("Not running anywhere");
		const foot = text(container.querySelector("[data-where-foot]"));
		expect(foot).toContain("isn't on any service whose version is known.");
		expect(foot).not.toContain("isn't running anywhere yet");
		expect(prose(container)).not.toMatch(MACHINE_WORDS);
	});

	test("by event: unknown because the snapshot has no event list, never 'not served'", async () => {
		const { container } = await mountApp(SCANNER, { query: "by=event" });
		const cells = [
			...container.querySelectorAll("[data-by-event] [data-matrix-cell]"),
		].map((cell) => [
			cell.getAttribute("data-matrix-cell"),
			cell.getAttribute("data-why"),
		]);
		expect(cells.length).toBeGreaterThan(0);
		expect(cells.filter(([state]) => state === "not_served")).toEqual([]);
		expect(
			cells.filter(([state]) => state === "unknown").map(([, why]) => why),
		).toEqual(
			cells.filter(([state]) => state === "unknown").map(() => "snapshot"),
		);
		expect(text(container.querySelector("[data-by-event]"))).toContain(
			"Unknown: the status snapshot has no event list",
		);
		expect(text(container.querySelector("[data-by-event]"))).toContain(
			"Shared for another app",
		);
	});
});

describe("App › Devices · buffered writes need you", () => {
	test("the data cell splits waiting, conflicts and paused changes", async () => {
		const { container } = await mountApp(NOTES);
		expect(text(container.querySelector("[data-headline]"))).toContain(
			"Buffered changes from field-notes on studio-mac-mini need you.",
		);
		const row = container.querySelector('[data-service="field-notes"]');
		expect(text(row?.querySelector('[data-writes="waiting"]'))).toBe(
			"14 waiting · 1 conflict",
		);
		expect(text(row?.querySelector('[data-writes="paused"]'))).toBe(
			"3 paused: cloud access changed",
		);
		expect(text(block(container, "ad-attn").querySelector("h2"))).toContain(
			"2",
		);
		const update = byRole("button", "Update everywhere…", container);
		expect(update.getAttribute("aria-disabled")).toBe("true");
		expect(text(container.querySelector("[data-app-header]"))).toContain(
			"Every service you can see already runs v2.0.0.",
		);
	});

	test("a gated Update everywhere… sends nothing and opens nothing", async () => {
		const { container, fake } = await mountApp(NOTES);
		const calls = fake.api.calls.length;
		const commands = fake.api.commands.length;
		await click(byRole("button", "Update everywhere…", container));
		expect(present(queryByRole("dialog"))).toBe(false);
		expect(fake.api.calls.length).toBe(calls);
		expect(fake.api.commands.length).toBe(commands);
	});
});

describe("App › Devices · events not on any device", () => {
	test("a stopped service and a paused upload in In progress", async () => {
		const { container } = await mountApp(CRM);
		expect(text(container.querySelector("[data-headline]"))).toContain(
			"nightly-sync on edge-berlin-01 is stopped, as you asked.",
		);
		const progress = block(container, "ad-prog");
		const row = progress.querySelector('[data-progress="upload"]');
		expect(text(row)).toContain("Upload paused · edge-berlin-01 · CRM Sync");
		expect(text(row)).toContain("12 of 38 files");
		expect(text(row)).toContain("resumable until");
		expect(hrefOf("Resume in deploy", row)).toBe(
			appUrl(
				CRM,
				`&flow=deploy&mode=update&device=${ID.edge}&step=copy_upload`,
			),
		);
		expect(
			attr(progress.querySelector("header [data-stamp]"), "data-src"),
		).toBe("local");
		expect(
			text(container.querySelector('[data-service="nightly-sync"]')),
		).toContain("Uploading a new version · paused");
	});

	test("by event: the events nobody serves are 'Not served', with Deploy here", async () => {
		const { container } = await mountApp(CRM, { query: "by=event" });
		const webhook = container.querySelector('[data-event="evt_crm_webhook"]');
		expect(
			present(webhook?.querySelector('[data-matrix-cell="not_served"]')),
		).toBe(true);
		const nightly = container.querySelector('[data-event="evt_crm_nightly"]');
		expect(present(nightly?.querySelector('[data-matrix-cell="served"]'))).toBe(
			true,
		);
	});
});

describe("App › Devices · never deployed", () => {
	test("the layout: what can run, how it runs, where it could run, one coral", async () => {
		const { container } = await mountApp(VISITOR);
		expect(layout(container)).toBe("never");
		const headline = text(container.querySelector("[data-headline]"));
		expect(headline).toContain(
			"Visitor Check-in isn't on any device you can see yet.",
		);
		expect(headline).toContain("2 of its 4 events can run on a device.");
		expect(
			[...container.querySelectorAll("[data-block]")].map((entry) => entry.id),
		).toEqual(["ad-can-run", "ad-mode", "ad-else"]);
		const canRun = block(container, "ad-can-run");
		expect(text(canRun.querySelector("h2"))).toContain("2 of 4");
		expect(text(canRun.querySelector("header [data-stamp]"))).toContain(
			"events · checked",
		);
		expect(
			text(block(container, "ad-mode").querySelector("header [data-stamp]")),
		).toContain("app settings · checked");
		expect(text(canRun)).toContain("Check-in page");
		expect(
			present(queryByRole("button", "Can't run on devices · 2", canRun)),
		).toBe(true);
		expect(present(canRun.querySelector("[data-matrix-cell]"))).toBe(false);
		expect(
			present(
				block(container, "ad-mode").querySelector("[data-mode-explainer]"),
			),
		).toBe(true);
		const could = block(container, "ad-else");
		expect(text(could.querySelector("h2"))).toContain("Where it could run");
		expect(text(container)).toContain("Deploy Visitor Check-in to a device");
		expect(primaries(container)).toBe(1);
		const header = container.querySelector("[data-app-header]");
		expect(present(header?.querySelector("[data-dv-primary]"))).toBe(false);
		const update = byRole("button", "Update everywhere…", container);
		expect(update.getAttribute("aria-disabled")).toBe("true");
		expect(text(header)).toContain(
			"Nothing to update: Visitor Check-in isn't on any device you can see.",
		);
		expect(prose(container)).not.toMatch(MACHINE_WORDS);
	});

	test("a device shared for another app is listed as no access, with the coverage line", async () => {
		const { container } = await mountApp(VISITOR);
		const could = block(container, "ad-else");
		expect(text(could.querySelector('[data-group="no-access"]'))).toContain(
			"lab-gpu-02",
		);
		expect(present(could.querySelector('[data-group="unknown"]'))).toBe(false);
		expect(text(could.querySelector('[data-group="never"]'))).toContain(
			"cold-storage-nas",
		);
		expect(present(container.querySelector("[data-app-coverage]"))).toBe(true);
	});
});

describe("App › Devices · every device locked", () => {
	test("one locked state with Unlock several…, nothing called 'not deployed'", async () => {
		const { container, fake } = await mountApp(INVOICE, { unlock: "none" });
		expect(layout(container)).toBe("all_unknown");
		expect(text(container.querySelector("[data-headline]"))).toContain(
			"Unlock to see where Invoice AI runs.",
		);
		expect(text(container.querySelector("[data-app-coverage]"))).toContain(
			"Status from 0 of 5",
		);
		// Locked is not "not on any device": the update button says what unlocks it.
		const header = text(container.querySelector("[data-app-header]"));
		expect(header).toContain(
			"Unlock first: which services run on your devices isn't readable yet.",
		);
		expect(header).not.toContain("isn't on any device");
		const where = block(container, "ad-where");
		expect(present(where.querySelector('[data-kind="locked"]'))).toBe(true);
		expect(present(where.querySelector("table"))).toBe(false);
		// The block is locked on this computer: its stamp says so, not "hub, checked now".
		const stamp = where.querySelector("header [data-stamp]");
		expect(attr(stamp, "data-src")).toBe("local");
		expect(attr(stamp, "data-age")).toBe("locked");
		expect(text(stamp)).toContain("locked · 5 devices");
		expect(text(where)).toContain("4 devices are locked here: ");
		expect(text(where)).toContain("and lab-gpu-02.");
		expect(text(where)).toContain(
			"Until you unlock them they count as unknown, never as not deployed.",
		);
		const calls = fake.api.calls.length;
		await click(byRole("button", "Unlock several…", where));
		expect(useOverlayStore.getState().overlay).toEqual({
			kind: "unlock_several",
		});
		expect(fake.api.calls.length).toBe(calls);
		expect(
			present(
				block(container, "ad-else").querySelector(
					'[data-group="not-deployed"]',
				),
			),
		).toBe(false);
		expect(primaries(container)).toBeLessThanOrEqual(1);
	});
});

describe("App › Devices · a multi-device run is tracked on this computer", () => {
	test("In progress shows it and Update everywhere… waits", async () => {
		const { container, fake, settle } = await mountApp(INVOICE);
		await act(async () => {
			const base = {
				kind: "command" as const,
				label: { code: "command" as const },
				startedBy: "you" as const,
				actions: [],
			};
			fake.workspace.activity.startRun({
				title: {
					code: "deploy",
					params: { what: "Invoice tools (MCP)", count: 2 },
				},
				oneAtATime: true,
				stopOnFail: true,
				items: [
					{
						...base,
						state: "done",
						target: {
							deviceId: ID.edge,
							serviceId: "tools",
							projectId: INVOICE,
						},
					},
					{
						...base,
						state: "active",
						target: {
							deviceId: ID.studio,
							serviceId: "tools",
							projectId: INVOICE,
						},
					},
				],
			});
		});
		await settle();
		const row = block(container, "ad-prog").querySelector(
			'[data-progress="run"]',
		);
		expect(text(row)).toContain(
			"Deploy Invoice tools (MCP) to 2 devices · 1 of 2 done",
		);
		const update = byRole("button", "Update everywhere…", container);
		expect(update.getAttribute("aria-disabled")).toBe("true");
		expect(text(container.querySelector("[data-app-header]"))).toContain(
			"Wait for “Deploy Invoice tools (MCP) to 2 devices” to finish.",
		);
	});
});

describe("App › Devices · Update everywhere sheet", () => {
	const openSheet = () =>
		mountApp(INVOICE, { query: "action=update-all", seed: labUnlockedSeed() });

	test("action=update-all opens it once and leaves the URL clean", async () => {
		const { navigations } = await mountApp(INVOICE, {
			query: "action=update-all",
		});
		const sheet = inPortal("dialog");
		expect(text(sheet)).toContain("Update Invoice AI everywhere");
		expect(text(sheet)).toContain(
			"Every service of Invoice AI you can see · one version for all",
		);
		expect(lastNavigation(navigations)).toEqual({
			mode: "replace",
			href: appUrl(INVOICE),
		});
	});

	test("choose: only the newest version, a busy service gated, the rest ticked", async () => {
		await openSheet();
		const sheet = inPortal("dialog");
		expect(text(sheet)).toContain("v1.5.0");
		expect(text(sheet)).toContain("Re-pins each service");
		const rows = [...sheet.querySelectorAll("[data-update-row]")];
		expect(rows.map((row) => row.getAttribute("data-update-row"))).toEqual([
			`${ID.edge}/invoice-extractor`,
			`${ID.lab}/invoice-extractor-gpu`,
		]);
		expect(text(rows[0])).toContain("An update is already running.");
		expect(
			byRole(
				"checkbox",
				/invoice-extractor on edge-berlin-01/,
				rows[0],
			).hasAttribute("disabled"),
		).toBe(true);
		expect(
			byRole(
				"checkbox",
				/invoice-extractor-gpu on lab-gpu-02/,
				rows[1],
			).getAttribute("aria-checked"),
		).toBe("true");
		expect(hrefOf("Change settings or events too…", sheet)).toBe(
			appUrl(INVOICE, `&flow=deploy&mode=update&device=${ID.lab}`),
		);
		// Three steps, the first one current; one coral, the way on.
		expect(
			[...sheet.querySelectorAll("ol li")].map((step) => [
				step.getAttribute("data-s"),
				text(step),
			]),
		).toEqual([
			["current", "1Choose"],
			["todo", "2Strategy"],
			["todo", "3Review"],
		]);
		expect(text(sheet)).toContain("Services per device");
		expect(text(sheet)).toContain("Step 1 of 3 · Choose");
		expect(primaries(sheet)).toBe(1);
		expect(
			present(
				byRole("button", "Next: strategy", sheet).closest("[data-dv-primary]"),
			),
		).toBe(true);
	});

	test("review states the consequences; nothing is sent before the last button", async () => {
		const { fake } = await openSheet();
		const writes = fake.api.writes().length;
		const commands = fake.api.commands.length;
		await click(byRole("button", "Next: strategy", inPortal("dialog")));
		expect(text(inPortal("dialog"))).toContain("One device at a time");
		await click(byRole("button", "Next: review", inPortal("dialog")));
		const review = inPortal("dialog");
		expect(text(review)).toContain(
			"invoice-extractor-gpu switches to v1.5.0 with a safe update.",
		);
		expect(text(review)).toContain(
			"Each device can roll back on its own; a finished update can be replaced by another update.",
		);
		expect(primaries(review)).toBe(1);
		expect(present(queryByRole("button", "Update 1 service", review))).toBe(
			true,
		);
		expect(fake.api.writes().length).toBe(writes);
		expect(fake.api.commands.length).toBe(commands);
	});

	test("starting shows the run on the page, one row per device", async () => {
		const { container, settle } = await openSheet();
		await click(byRole("button", "Next: strategy", inPortal("dialog")));
		await click(byRole("button", "Next: review", inPortal("dialog")));
		await click(byRole("button", "Update 1 service", inPortal("dialog")));
		await settle();
		expect(present(queryByRole("dialog"))).toBe(false);
		const run = block(container, "ad-prog").querySelector(
			"[data-fleet-rollout]",
		);
		expect(attr(run, "data-fleet-rollout")).toBe("update");
		expect(text(run)).toContain("Update Invoice AI everywhere");
		expect(text(run)).toContain("one device at a time");
		expect(text(run)).toContain("lab-gpu-02");
	});
});

describe("App › Devices · service row menu", () => {
	const openMenu = async (container: HTMLElement, service: string) => {
		await click(byRole("button", `More for ${service}`, container));
		return inPortal("menu");
	};

	test("the items in order; what can't run stays visible with its reason", async () => {
		const { container } = await mountApp(SUPPORT);
		const menu = await openMenu(container, "support-bot");
		const items = allByRole("menuitem", undefined, menu);
		expect(items.map((item) => item.getAttribute("data-menu-item"))).toEqual([
			"open-service",
			"open-device",
			"update",
			"settings",
			"add-event",
			"start",
			"restart",
			"stop",
			"logs",
			"remove",
			"copy-id",
		]);
		expect(text(items[2])).toContain("Update to v2.4.0…");
		expect(items[2].getAttribute("href")).toBe(
			appUrl(SUPPORT, `&flow=deploy&device=${ID.edge}&service=support-bot`),
		);
		expect(items[3].getAttribute("href")).toContain("step=settings");
		expect(items[4].getAttribute("href")).toContain("step=what");
		expect(items[8].getAttribute("href")).toContain("tab=activity");
	});

	test("Stop… confirms inline with who notices in this app, then sends one command", async () => {
		const { container, fake, settle } = await mountApp(SUPPORT);
		const menu = await openMenu(container, "support-bot");
		const before = fake.api.commands.length;
		await click(allByRole("menuitem", "Stop…", menu)[0]);
		const confirm = container.querySelector("[data-confirm-row]");
		expect(text(confirm)).toContain("Stop support-bot?");
		expect(text(confirm)).toContain(
			"Support chat and Support API at 0.0.0.0:8443 stops answering on edge-berlin-01. Support Portal then runs nowhere you can see.",
		);
		expect(text(confirm)).toContain(
			"Settings v7 and the data on the device stay.",
		);
		expect(fake.api.commands.length).toBe(before);
		await click(byRole("button", "Stop support-bot", confirm ?? undefined));
		await settle();
		expect(fake.api.commands.slice(before).map(([, type]) => type)).toContain(
			"stop",
		);
		expect(present(container.querySelector("[data-confirm-row]"))).toBe(false);
		expect(present(container.querySelector("[data-result-row]"))).toBe(true);
	});

	test("Cancel sends nothing", async () => {
		const { container, fake } = await mountApp(SUPPORT);
		const menu = await openMenu(container, "support-bot");
		const before = fake.api.commands.length;
		await click(allByRole("menuitem", "Restart…", menu)[0]);
		const confirm = container.querySelector("[data-confirm-row]");
		await click(byRole("button", "Cancel", confirm ?? undefined));
		expect(present(container.querySelector("[data-confirm-row]"))).toBe(false);
		expect(fake.api.commands.length).toBe(before);
	});

	test("Remove service… needs the typed service id; the button alone sends nothing", async () => {
		const { container, fake } = await mountApp(SUPPORT);
		const menu = await openMenu(container, "support-bot");
		const before = fake.api.commands.length;
		await click(allByRole("menuitem", "Remove service…", menu)[0]);
		const sheet = inPortal();
		expect(text(sheet)).toContain("Remove support-bot?");
		expect(text(sheet)).toContain(
			"support-bot is removed from edge-berlin-01. Its ID can't be used again on this device.",
		);
		expect(text(sheet)).toContain(
			"It is running: it is stopped first, then removed.",
		);
		await click(byRole("button", "Stop and remove support-bot", sheet));
		expect(fake.api.commands.length).toBe(before);
	});

	test("Stop and remove stops the service, reads that it stopped, then removes it", async () => {
		const { container, fake, settle } = await mountApp(SUPPORT);
		const menu = await openMenu(container, "support-bot");
		await click(allByRole("menuitem", "Remove service…", menu)[0]);
		const sheet = inPortal();
		const before = fake.api.commands.length;
		await typeInto(byRole("textbox", undefined, sheet), "support-bot");
		await click(byRole("button", "Stop and remove support-bot", sheet));
		await settle();
		const sent = fake.api.commands.slice(before).map(([, type]) => type);
		const stop = sent.indexOf("stop");
		const remove = sent.indexOf("remove");
		expect(stop).toBeGreaterThanOrEqual(0);
		// The device removes only a service it reports as stopped: a status read sits between the two.
		expect(remove).toBeGreaterThan(stop + 1);
		expect(
			present(container.querySelector('[data-service="support-bot"]')),
		).toBe(false);
	});

	test("without a live connection the commands keep their reason and send nothing; the wizard links stay", async () => {
		const { container, fake, settle } = await mountApp(SUPPORT);
		await act(async () => {
			await fake.workspace.live.close(ID.edge);
		});
		await settle();
		const menu = await openMenu(container, "support-bot");
		const item = (id: string) =>
			menu.querySelector<HTMLElement>(`[data-menu-item="${id}"]`);
		for (const id of ["start", "restart", "stop", "remove"])
			expect([id, attr(item(id), "aria-disabled")]).toEqual([id, "true"]);
		expect(text(item("stop"))).toContain(
			"Connect live to edge-berlin-01 first.",
		);
		expect(text(item("remove"))).toContain(
			"Connect live to edge-berlin-01 first.",
		);
		// The deploy wizard connects by itself, so its links aren't held back.
		expect(attr(item("update"), "href")).toContain("flow=deploy");
		expect(attr(item("settings"), "aria-disabled")).not.toBe("true");
		const before = fake.api.commands.length;
		const stop = item("stop");
		if (stop) await click(stop);
		expect(present(container.querySelector("[data-confirm-row]"))).toBe(false);
		expect(fake.api.commands.length).toBe(before);
	});
});

describe("App › Devices · roles, gates and failures", () => {
	test("without Read boards the page is one locked panel and reads no device", async () => {
		const { container, fake } = await mountApp(PARTNER, {
			unlock: "none",
			backend: appBackend({
				roleState: {
					getOwnRole: async () => ({
						role_id: "role-member",
						role_name: "Member",
						permissions: 4,
						is_owner: false,
						can_leave: true,
					}),
				} as unknown as IBackendState["roleState"],
			}),
		});
		expect(layout(container)).toBe("no-role");
		expect(text(container)).toContain("Devices is locked for your role");
		expect(text(container)).toContain(
			"Your role on Partner Reports (Member) can't read its flows, so this page can't show where it runs. That doesn't mean it runs nowhere.",
		);
		expect(text(container)).toContain("Needs: Read boards.");
		expect(present(container.querySelector("[data-mode]"))).toBe(false);
		expect(present(container.querySelector("[data-block]"))).toBe(false);
		expect(fake.api.sent("GET", /device-placements/)).toEqual([]);
		expect(fake.api.sent("GET", /my-access/)).toEqual([]);
		expect(fake.api.commands).toEqual([]);
		await click(byRole("button", "Copy a request for the owner", container));
		expect(dom.clipboard.at(-1)).toContain("Read boards");
	});

	test("no devices yet: the strip stays and the page points to set-up", async () => {
		const seed = emptyInput();
		seed.hub = { state: "on", serverTime: SAMPLE_NOW };
		const { container } = await mountApp(INVOICE, { seed });
		expect(layout(container)).toBe("no_devices");
		expect(present(container.querySelector("[data-mode]"))).toBe(true);
		expect(text(container.querySelector("[data-headline]"))).toContain(
			"You don't have any devices yet.",
		);
		expect(
			hrefOf("Set up a device", container.querySelector('[data-kind="empty"]')),
		).toBe("/settings/devices?flow=setup");
		expect(
			byRole("button", "Deploy to devices…", container).getAttribute(
				"aria-disabled",
			),
		).toBe("true");
		expect(text(container.querySelector("[data-app-header]"))).toContain(
			"You have no device that can take a deploy right now.",
		);
		expect(text(container.querySelector("[data-app-header]"))).toContain(
			"Nothing to update yet: you have no devices.",
		);
		expect(primaries(container)).toBe(1);
	});

	test("a failing hub keeps the data, says so once, and marks the hub stamps", async () => {
		const { container, fake, settle } = await mountApp(INVOICE);
		expect(text(container)).not.toContain("The hub can't be reached right now");
		fake.api.fail({ method: "GET", path: "devices" });
		await act(async () => {
			await fake.queryClient.refetchQueries();
		});
		await settle();
		expect(text(container)).toContain("The hub can't be reached right now");
		expect(
			present(container.querySelector('[data-service="invoice-extractor"]')),
		).toBe(true);
		expect(
			attr(
				block(container, "ad-else").querySelector("header [data-stamp]"),
				"data-age",
			),
		).toBe("error");
	});
});

describe("App › Devices · older hub and older agent", () => {
	test("an older hub: cloud access is read per device, no error, no retry storm", async () => {
		const { container, fake } = await mountApp(INVOICE, { hubVersion: "old" });
		const spending = block(container, "ad-spending");
		expect(text(spending)).toContain(
			"This hub has no per-app list of cloud access yet: approvals are read device by device, for the devices you own.",
		);
		expect(present(spending.querySelector('[data-kind="error"]'))).toBe(false);
		expect(text(container)).not.toContain("The hub can't be reached right now");
		expect(
			fake.api.sent("GET", `apps/${INVOICE}/device-placements`).length,
		).toBeLessThanOrEqual(1);
		expect(
			present(container.querySelector('[data-service="invoice-extractor"]')),
		).toBe(true);
	});

	test("an older agent: events are unknown, nothing is called not served, no new command is sent", async () => {
		const { container, fake } = await mountApp(INVOICE, {
			agentFeatures: {},
			query: "by=event",
		});
		expect(text(container)).not.toContain("The hub can't be reached right now");
		const cells = [
			...container.querySelectorAll("[data-by-event] td [data-matrix-cell]"),
		].map((cell) => cell.getAttribute("data-matrix-cell"));
		expect(cells).not.toContain("not_served");
		expect(cells).toContain("unknown");
		const NEWER = [
			"host_operation",
			"rollout_history",
			"operations",
			"metrics_history",
			"offline_queue_operations",
			"offline_queue_lookup",
		];
		expect(
			fake.api.commands.filter(([, type]) => NEWER.includes(type)),
		).toEqual([]);
	});
});

describe("App › Devices · switching apps", () => {
	test("another app discards what the first one showed", async () => {
		function Switcher() {
			const [appId, setAppId] = useState(INVOICE);
			return (
				<>
					<button type="button" onClick={() => setAppId(NOTES)}>
						switch app
					</button>
					<AppDevicesScreen
						appId={appId}
						scope={{ kind: "app", appId }}
						route={{ screen: "app-devices", by: "device" }}
					/>
				</>
			);
		}
		const { container, settle } = await mountDevices(<Switcher />, {
			host: "app",
			search: `id=${INVOICE}`,
			backend: appBackend(),
		});
		expect(
			present(container.querySelector('[data-service="invoice-extractor"]')),
		).toBe(true);
		await click(byRole("button", "switch app", container));
		await settle();
		expect(
			present(container.querySelector('[data-service="invoice-extractor"]')),
		).toBe(false);
		expect(
			present(container.querySelector('[data-service="field-notes"]')),
		).toBe(true);
		expect(text(container.querySelector("[data-app-header]"))).toContain(
			"Where Field Notes runs",
		);
		expect(text(block(container, "ad-versions"))).not.toContain("v1.5.0");
		expect(present(container.querySelector("#ad-prog"))).toBe(false);
	});
});

describe("App › Devices · a larger fleet (long lists are capped)", () => {
	const fleet = (devices: number) => ({
		seed: generateFleet(devices).input as DeviceSeed,
	});
	const groupRows = (root: ParentNode) =>
		root.querySelectorAll("tr[data-group]").length;
	const deviceRows = (root: ParentNode) =>
		root.querySelectorAll("li[data-device]").length;
	const showMore = (root: Element | null) =>
		click(byRole("button", /^Show \d+ more$/, root ?? undefined));
	const within = (root: ParentNode, selector: string) => {
		const found = root.querySelector<HTMLElement>(selector);
		if (!found) throw new Error(`no ${selector}`);
		return found;
	};

	/** Three more locked devices on top of the two the fleet already has. */
	function lockedFleet() {
		const { seed } = fleet(43);
		const known = new Set(Object.keys(seed.fleet));
		const lock = new Set(
			seed.keys
				.filter(
					(session) =>
						session.state === "unlocked" &&
						session.deviceId !== ID.edge &&
						known.has(session.deviceId),
				)
				.slice(0, 3)
				.map((session) => session.deviceId),
		);
		seed.keys = seed.keys.map((session) =>
			lock.has(session.deviceId)
				? keySession(session.deviceId, "locked", session.role, session.grantId)
				: session,
		);
		return { seed, locked: lock.size };
	}

	test("43 devices: device groups are capped, with Show more", async () => {
		const { container } = await mountApp(INVOICE, fleet(43));
		const where = block(container, "ad-where");
		expect(groupRows(where)).toBe(8);
		const capped = within(where, "[data-capped]");
		expect(text(capped)).toMatch(/Showing the 8 most severe of \d+ devices/);
		await showMore(capped);
		expect(groupRows(where)).toBeGreaterThan(8);
		expect(present(where.querySelector("[data-capped]"))).toBe(false);
		expect(primaries(container)).toBe(1);
	});

	test("43 devices: Everywhere else is capped, names too, with Show more", async () => {
		const { container } = await mountApp(INVOICE, fleet(43));
		const notDeployed = within(
			block(container, "ad-else"),
			'[data-group="not-deployed"]',
		);
		expect(deviceRows(notDeployed)).toBe(5);
		const more = within(notDeployed, "[data-more]");
		expect(text(more)).toMatch(
			/^\d+ more: [\w-]+, [\w-]+, [\w-]+ and \d+ more/,
		);
		await showMore(more);
		expect(deviceRows(notDeployed)).toBeGreaterThan(5);
		expect(present(notDeployed.querySelector("[data-more]"))).toBe(false);
	});

	test("43 devices: a version nobody can vouch for is never 'not running anywhere'", async () => {
		const { container } = await mountApp(INVOICE, fleet(43));
		const runs = text(
			block(container, "ad-versions").querySelector("[data-ver-runs]"),
		);
		expect(runs).toContain("Not running on any device you can see");
		expect(runs).toMatch(/\d+ services · version unknown/);
		expect(runs).not.toContain("Not running anywhere");
	});

	test("43 devices: on the viewer's own devices an unlisted approval is no cloud access, never a hidden one", async () => {
		const { container } = await mountApp(INVOICE, fleet(43));
		const notes = [
			...block(container, "ad-where").querySelectorAll("[data-cloud-note]"),
		].map((note) => text(note));
		expect(notes.length).toBeGreaterThan(0);
		expect([...new Set(notes)]).toEqual(["No cloud access"]);
		expect(
			block(container, "ad-spending").querySelectorAll("[data-approval-hidden]")
				.length,
		).toBe(0);
	});

	test("43 devices: access lists people, with one note per reason and no action on a known answer", async () => {
		const { container } = await mountApp(INVOICE, fleet(43));
		const access = block(container, "ad-access");
		expect(access.querySelectorAll("[data-grant]").length).toBe(3);
		expect(text(access.querySelector("h2"))).toContain("3");
		const nobody = () =>
			text(access.querySelector('[data-access-note="nobody"]'));
		expect(access.querySelectorAll('[data-access-note="nobody"]').length).toBe(
			1,
		);
		expect(nobody()).toMatch(
			/^Nobody else has access on [\w-]+, [\w-]+, [\w-]+ and \d+ more\.$/,
		);
		expect(access.querySelectorAll("[data-access-note] button").length).toBe(0);
		const more = within(access, '[data-more="devices"]');
		expect(text(more)).toMatch(/Access on \d+ more devices isn't shown/);
		const before = nobody();
		await showMore(more);
		expect(present(access.querySelector('[data-more="devices"]'))).toBe(false);
		expect(nobody()).not.toBe(before);
	});

	test("43 devices: by event is a list, one Runs on cell per event", async () => {
		const { container } = await mountApp(INVOICE, {
			...fleet(43),
			query: "by=event",
		});
		const view = within(container, "[data-by-event]");
		expect(attr(view, "data-by-event")).toBe("list");
		expect(view.querySelectorAll("th[data-device]").length).toBe(0);
		expect([...view.querySelectorAll("th")].map((th) => text(th))).toEqual([
			"Event",
			"Runs on",
		]);
		expect(view.querySelectorAll("tr[data-event]").length).toBe(3);
	});

	test("more than two unknown devices are one row, one sentence and one Unlock several…", async () => {
		const { seed, locked } = lockedFleet();
		expect(locked).toBe(3);
		const { container, fake } = await mountApp(INVOICE, { seed });
		const where = block(container, "ad-where");
		const collapsed = within(where, "[data-collapsed-unknown]");
		expect(text(collapsed)).toMatch(/^\d+ more devices · status unknown/);
		expect(text(collapsed)).toMatch(
			/Which Invoice AI services run on [\w-]+, [\w-]+, [\w-]+, [\w-]+ and \d+ more isn't readable on this computer yet\. They count as unknown, never as not deployed\./,
		);
		expect(where.querySelectorAll("[data-state-row]").length).toBe(0);
		const calls = fake.api.calls.length;
		await click(byRole("button", "Unlock several…", collapsed));
		expect(useOverlayStore.getState().overlay).toEqual({
			kind: "unlock_several",
		});
		expect(fake.api.calls.length).toBe(calls);
		const fold = within(
			block(container, "ad-versions"),
			'[data-ver-runs] [data-ver-fold="devices"]',
		);
		expect(text(fold)).toMatch(/^\d+ devices unknown until unlocked$/);
	});
});
