import { afterAll, afterEach, describe, expect, test } from "bun:test";
import { type ReactNode, useMemo, useState } from "react";
import type { EventsOverviewProps } from "../../events/events-overview";
import {
	allByRole,
	byRole,
	click,
	installDom,
	queryByRole,
	typeInto,
} from "../testing/dom-harness";
import type { FakeWorkspaceOptions } from "../testing/fake-workspace";
import type { SampleApp } from "./events-test-kit";

const dom = installDom();
const { cleanupDevices, mountDevices, preloadDevices } = await import(
	"../testing/mount-devices"
);
await preloadDevices();
const { AppRouterContext } = await import(
	"next/dist/shared/lib/app-router-context.shared-runtime"
);
const { SearchParamsContext } = await import(
	"next/dist/shared/lib/hooks-client-context.shared-runtime"
);
const { createFakeWorkspace } = await import("../testing/fake-workspace");
const { EventsOverview } = await import("../../events/events-overview");
const { EVENTS_BLOCK_ID } = await import("./on-devices-strip");
const { runsOnReasonId } = await import("./runs-on-cell");
const { samplePageEvents } = await import("./events-test-kit");
const { useOverlayStore } = await import("../workspace/overlay-store");

afterEach(async () => {
	await cleanupDevices();
	useOverlayStore.getState().close();
	await dom.cleanup();
});
afterAll(dom.restore);

const APP: SampleApp = "app_invoice_ai";
const HUB_ON = { standalone: { enabled: true } } as never;
const HUB_OFF = { standalone: { enabled: false } } as never;
const DEPLOY = `/library/config/devices?id=${APP}&flow=deploy&mode=new`;

interface Navigation {
	mode: "push" | "replace";
	href: string;
}

/** Next's router and search params over an in-memory URL: `replace` and `push` update what the page reads. */
function RouterFrame({
	search,
	log,
	children,
}: Readonly<{ search: string; log: Navigation[]; children: ReactNode }>) {
	const [query, setQuery] = useState(search);
	const router = useMemo(() => {
		const go = (mode: Navigation["mode"]) => (href: string) => {
			log.push({ mode, href });
			const [path, next = ""] = href.split("?");
			if (path === window.location.pathname) setQuery(next);
		};
		return {
			push: go("push"),
			replace: go("replace"),
			back: () => undefined,
			forward: () => undefined,
			refresh: () => undefined,
			prefetch: () => undefined,
		};
	}, [log]);
	const params = useMemo(() => new URLSearchParams(query), [query]);
	return (
		<AppRouterContext.Provider value={router as never}>
			<SearchParamsContext.Provider value={params}>
				{children}
			</SearchParamsContext.Provider>
		</AppRouterContext.Provider>
	);
}

const EVENTS_BACKEND = {
	routeState: {
		getRoutes: async () => [],
		setRoute: async () => undefined,
		deleteRouteByPath: async () => undefined,
	},
	boardState: {
		getBoardSummaries: async () => [],
		getBoard: async () => ({ nodes: {} }),
		listRuns: async () => [],
	},
};

interface PageOptions extends FakeWorkspaceOptions {
	search?: string;
	signedIn?: boolean;
	props?: Partial<EventsOverviewProps>;
}

async function mountPage(appId: SampleApp, options: PageOptions = {}) {
	const { search = `id=${appId}`, signedIn, props, ...workspace } = options;
	const fake = await createFakeWorkspace(undefined, workspace);
	const calls = fake.api.calls.length;
	const navigations: Navigation[] = [];
	const edited: string[] = [];
	const view = await mountDevices(
		({ overrides }) => (
			<RouterFrame search={search} log={navigations}>
				<EventsOverview
					events={samplePageEvents(appId)}
					boardsMap={new Map([["board_0", "Extraction Pipeline"]])}
					appId={appId}
					eventMapping={{}}
					uiEventTypes={["simple_chat"]}
					onEdit={(event) => edited.push(event.id)}
					onDelete={() => undefined}
					onNavigateToNode={() => undefined}
					onCreateEvent={() => undefined}
					hub={HUB_ON}
					devicesHarness={{ overrides }}
					{...props}
				/>
			</RouterFrame>
		),
		{
			fake,
			providers: false,
			signedIn,
			backend: EVENTS_BACKEND as never,
		},
	);
	const row = (eventId: string) => {
		const found = document.getElementById(`event-row-${eventId}`);
		if (!found) throw new Error(`no row for ${eventId}`);
		return found;
	};
	return {
		...view,
		navigations,
		edited,
		row,
		run: (eventId: string) => {
			const found = row(eventId).querySelector<HTMLElement>(
				"[data-run-on-device]",
			);
			if (!found) throw new Error(`no Run on a device… in ${eventId}`);
			return found;
		},
		callsSince: () => fake.api.calls.slice(calls),
	};
}

describe("Events list: Devices column", () => {
	test("each section has a Devices column head and every row a Devices cell", async () => {
		const view = await mountPage(APP);
		const heads = view.container.querySelectorAll("[aria-hidden='true']");
		expect(
			[...heads].filter((head) => /Devices/.test(head.textContent ?? ""))
				.length,
		).toBe(2);
		for (const event of samplePageEvents(APP))
			expect(document.getElementById(runsOnReasonId(event.id))).toBeTruthy();
		expect(
			byRole(
				"link",
				"edge-berlin-01",
				document.getElementById(runsOnReasonId("evt_extract_http")) ??
					undefined,
			),
		).toBeTruthy();
		expect(view.container.textContent).toContain(
			"3 of 6 events can run on a device.",
		);
		expect(
			document.querySelectorAll("[data-dv-primary]").length,
		).toBeLessThanOrEqual(1);
	});

	test("normal rows keep their own actions after Run on a device…", async () => {
		const view = await mountPage(APP);
		const row = view.row("evt_gpu_extract");
		const labels = allByRole("button", undefined, row)
			.concat(allByRole("link", undefined, row))
			.map((el) => el.getAttribute("aria-label"))
			.filter(Boolean);
		expect(labels).toContain("Pause event");
		expect(labels).toContain("Configure event");
		expect(labels).toContain("Open in flow");
		expect(labels).toContain("Delete event");
		const actions = view.run("evt_gpu_extract").parentElement;
		expect(actions?.firstElementChild).toBe(view.run("evt_gpu_extract"));
	});
});

describe("Run on a device… (APP §4.5)", () => {
	test("opens deploy for that event, coming from Events", async () => {
		const view = await mountPage(APP);
		const run = view.run("evt_gpu_extract");
		expect(run.getAttribute("href")).toBe(
			`${DEPLOY}&event=evt_gpu_extract&from=events`,
		);
		expect(run.getAttribute("aria-label")).toBe(
			"Run Extract invoice (GPU) on a device…",
		);
		await click(run);
		expect(view.navigations).toEqual([
			{ mode: "push", href: `${DEPLOY}&event=evt_gpu_extract&from=events` },
		]);
	});

	test("an event already on a device reads “on another device”", async () => {
		const view = await mountPage(APP);
		expect(view.run("evt_extract_http").getAttribute("aria-label")).toBe(
			"Run Extract invoice on another device…",
		);
	});

	test("an event that can't run: disabled, described by the Devices cell, and a click sends nothing (R7)", async () => {
		const view = await mountPage(APP);
		const run = view.run("evt_invoice_review");
		expect(run.getAttribute("aria-disabled")).toBe("true");
		const reason = document.getElementById(
			run.getAttribute("aria-describedby") ?? "",
		);
		expect(reason?.id).toBe(runsOnReasonId("evt_invoice_review"));
		expect(reason?.textContent).toContain("Follows the latest flow");
		expect(run.getAttribute("aria-label")).toBe(
			"Run Review queue on a device… Unavailable: Can't run on devices: Follows the latest flow",
		);
		const before = view.callsSince().length;
		await click(run);
		expect(view.navigations).toEqual([]);
		expect(view.callsSince()).toHaveLength(before);
	});

	test("hub-off: every row is off with the one banner as the reason, and no strip", async () => {
		const view = await mountPage(APP, {
			unlock: "none",
			props: { hub: HUB_OFF },
		});
		expect(document.querySelectorAll("[data-events-block]")).toHaveLength(1);
		for (const event of samplePageEvents(APP)) {
			const run = view.run(event.id);
			expect(run.getAttribute("aria-disabled")).toBe("true");
			expect(run.getAttribute("aria-describedby")).toBe(EVENTS_BLOCK_ID);
			expect(
				document.getElementById(runsOnReasonId(event.id))?.textContent,
			).toBe("Device status off on this hub");
		}
		expect(document.getElementById(EVENTS_BLOCK_ID)?.textContent).toContain(
			"Device support is off on this hub",
		);
		expect(view.container.querySelector("[data-on-devices]")).toBeNull();
		expect(view.callsSince()).toEqual([]);
	});

	test("signed out: cells and banner ask to sign in", async () => {
		const view = await mountPage(APP, { signedIn: false, unlock: "none" });
		expect(
			document.getElementById(runsOnReasonId("evt_extract_http"))?.textContent,
		).toBe("Sign in to see devices");
		expect(view.run("evt_extract_http").getAttribute("aria-describedby")).toBe(
			EVENTS_BLOCK_ID,
		);
		expect(view.callsSince()).toEqual([]);
	});
});

describe("read-only role (APP §4.6)", () => {
	test("without ReadBoards the Flow names notice is the block reason and offers the request for the owner", async () => {
		const view = await mountPage(APP, {
			unlock: "none",
			props: { canEdit: false, canReadBoards: false },
		});
		const notice = document.getElementById(EVENTS_BLOCK_ID);
		expect(notice?.textContent).toContain("Flow names unavailable");
		expect(notice?.textContent).toContain(
			"This page also can't tell which devices run its events, and Run on a device… is off.",
		);
		expect(view.container.textContent).toContain(
			"Events are read-only for you",
		);
		expect(document.querySelectorAll(`[id="${EVENTS_BLOCK_ID}"]`)).toHaveLength(
			1,
		);
		expect(
			document.getElementById(runsOnReasonId("evt_extract_http"))?.textContent,
		).toBe("Unknown: you can't read this app's flows");
		expect(view.run("evt_extract_http").getAttribute("aria-describedby")).toBe(
			EVENTS_BLOCK_ID,
		);
		await click(byRole("button", "Copy a request for the owner"));
		expect(dom.clipboard.at(-1)).toContain(
			"Read boards permission on Invoice AI (app_invoice_ai)",
		);
		expect(view.callsSince()).toEqual([]);
	});
});

describe("event= deep link (APP §4.1)", () => {
	test("highlights the row and opens its Devices popover", async () => {
		const view = await mountPage(APP, {
			search: `id=${APP}&event=evt_extract_http`,
		});
		expect(view.row("evt_extract_http").hasAttribute("data-event-target")).toBe(
			true,
		);
		expect(view.row("evt_gpu_extract").hasAttribute("data-event-target")).toBe(
			false,
		);
		expect(allByRole("dialog")).toHaveLength(1);
		expect(byRole("dialog", "Where Extract invoice runs")).toBeTruthy();
		expect(view.navigations).toEqual([]);
	});

	test("an unknown id says so, shows every event and drops the param with replace", async () => {
		const view = await mountPage(APP, {
			search: `id=${APP}&event=evt_gone`,
		});
		expect(view.container.textContent).toContain(
			"No event with the ID evt_gone in Invoice AI. Showing all its events.",
		);
		expect(view.navigations).toEqual([
			{ mode: "replace", href: `/?id=${APP}` },
		]);
		expect(queryByRole("dialog")).toBeNull();
		for (const event of samplePageEvents(APP))
			expect(view.row(event.id)).toBeTruthy();
	});

	test("the page's own eventId= param is left alone", async () => {
		const view = await mountPage(APP, {
			search: `id=${APP}&eventId=evt_extract_http`,
		});
		expect(queryByRole("dialog")).toBeNull();
		expect(view.navigations).toEqual([]);
	});
});

describe("row menu on narrow lists (APP §4.7)", () => {
	async function openMenu(eventId: string, name: string) {
		const view = await mountPage(APP);
		await click(byRole("button", `Actions for ${name}`, view.row(eventId)));
		return { view, menu: byRole("menu") };
	}

	test("Run on a device… comes first, then where it runs, then the row's own actions", async () => {
		const { menu } = await openMenu("evt_extract_http", "Extract invoice");
		expect(
			allByRole("menuitem", undefined, menu).map((item) => item.textContent),
		).toEqual([
			"Run on another device…",
			"Where it runs",
			"Pause event",
			"Configure event",
			"Open in flow",
			"Delete event",
		]);
	});

	test("choosing Run on a device… opens deploy for the event", async () => {
		const { view, menu } = await openMenu(
			"evt_gpu_extract",
			"Extract invoice (GPU)",
		);
		await click(byRole("menuitem", "Run on a device…", menu));
		expect(view.navigations).toEqual([
			{ mode: "push", href: `${DEPLOY}&event=evt_gpu_extract&from=events` },
		]);
	});

	test("an event that can't run keeps the item, disabled, with its reason", async () => {
		const { view, menu } = await openMenu("evt_invoice_review", "Review queue");
		const run = allByRole("menuitem", undefined, menu)[0];
		expect(run.textContent).toBe(
			"Run on a device…Can't run on devices: Follows the latest flow",
		);
		expect(run.getAttribute("aria-disabled")).toBe("true");
		expect(
			byRole("menuitem", "Why it can't run on devices", menu),
		).toBeTruthy();
		await click(run);
		expect(view.navigations).toEqual([]);
	});

	test("Where it runs opens the Devices popover of that row", async () => {
		const { view, menu } = await openMenu(
			"evt_extract_http",
			"Extract invoice",
		);
		await click(byRole("menuitem", "Where it runs", menu));
		await view.settle();
		expect(byRole("dialog", "Where Extract invoice runs")).toBeTruthy();
	});
});

describe("the rest of the list stays as it was", () => {
	test("a search with no match shows the empty filter state and no Devices cells", async () => {
		const view = await mountPage(APP);
		await typeInto(byRole("textbox", "Search events"), "zzz-no-such-event");
		await view.settle();
		expect(view.container.textContent).toContain(
			"No event matches this search.",
		);
		expect(view.container.querySelector("[data-runs-on]")).toBeNull();
		expect(byRole("button", "Clear filters")).toBeTruthy();
	});

	test("the event name still opens the editor, and the popover's fix does too", async () => {
		const view = await mountPage(APP);
		await click(
			byRole("button", "Review queue", view.row("evt_invoice_review")),
		);
		expect(view.edited).toEqual(["evt_invoice_review"]);
		await click(
			byRole(
				"button",
				"Why Review queue can't run on devices",
				document.getElementById(runsOnReasonId("evt_invoice_review")) ??
					undefined,
			),
		);
		await click(byRole("button", "Open the event…"));
		expect(view.edited).toEqual(["evt_invoice_review", "evt_invoice_review"]);
	});
});
