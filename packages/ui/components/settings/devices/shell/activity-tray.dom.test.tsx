import { afterAll, afterEach, describe, expect, test } from "bun:test";
import { type ReactNode, act } from "react";
import { SAMPLE_IDS } from "../../../../lib/device-management/model/__fixtures__/sample-fleet";
import type {
	DevicesRoute,
	DevicesScope,
} from "../../../../lib/device-management/model/types";
import type {
	ActivityAction,
	ActivityItem,
	ActivityKind,
} from "../../../../lib/device-management/workspace/types";
import type { AreaTime, DevicesT } from "../primitives/area-context";
import {
	allByRole,
	byRole,
	click,
	fire,
	installDom,
	keyDown,
	queryByRole,
	settle,
} from "../testing/dom-harness";
import type { MountDevicesOptions } from "../testing/mount-devices";

const dom = installDom();
const { cleanupDevices, mountDevices, preloadDevices } = await import(
	"../testing/mount-devices"
);
await preloadDevices();
const { getI18n } = await import("@flow-like/locales");
const area = await import("../primitives/area-context");
const { ActivityButton } = await import("./activity-button");
const { useOverlayStore } = await import("../workspace");
const tray = await import("./activity-tray");
const {
	ACTIVITY_TRAY_ID,
	ActivityTray,
	ActivityTrayView,
	activityDetail,
	activityRoute,
	activityTarget,
	activityTitle,
	trayCounts,
	traySections,
	useActivityTray,
} = tray;

const NOW_MS = 1_790_769_600_000;
const t = getI18n().getFixedT("en", "devices") as DevicesT;

const TIME: AreaTime = {
	now: NOW_MS,
	nowS: NOW_MS / 1000,
	locale: "en",
	ago: (atS) => `${Math.round(NOW_MS / 1000 - atS)} s ago`,
	clock: (atS) => `clock:${atS}`,
	abs: (atS) => `abs:${atS}`,
	at: (atS) => `at:${atS}`,
	countdown: (untilS) => `${Math.round(untilS - NOW_MS / 1000)} s`,
};

function item(id: string, over: Partial<ActivityItem> = {}): ActivityItem {
	return {
		id,
		kind: "safe_update",
		target: {
			deviceId: "5b794764-0000-4000-8000-000000000001",
			deviceName: "edge-berlin-01",
			serviceId: "invoice-extractor",
		},
		state: "active",
		label: { code: "safe_update" },
		startedAt: NOW_MS - 240_000,
		updatedAt: NOW_MS - 10_000,
		startedBy: "you",
		actions: ["open"],
		href: {
			screen: "service",
			deviceId: "5b794764-0000-4000-8000-000000000001",
			serviceId: "invoice-extractor",
			tab: "status",
		},
		...over,
	};
}

const ITEMS: ActivityItem[] = [
	item("rollout", {
		progress: { done: 1, total: 2, unit: "instances" },
		detail: { code: "instances_progress" },
		deadlineAt: NOW_MS + 87_000,
	}),
	item("upload", {
		kind: "upload",
		label: { code: "upload" },
		state: "paused",
		detail: { code: "resumable_until", params: { until: NOW_MS + 3_600_000 } },
		actions: ["open", "resume", "discard"],
	}),
	item("rules", {
		kind: "access_rules",
		label: { code: "access_rules" },
		state: "waiting",
		detail: { code: "waiting_for_device" },
	}),
	item("stop", {
		kind: "command",
		label: { code: "command", params: { command: "stop" } },
		state: "unknown",
		detail: { code: "no_reply" },
		actions: ["open", "check_again", "dismiss"],
		resume: {
			type: "operation",
			operationId: "op-1",
			command: "stop",
			issuedAt: NOW_MS / 1000 - 60,
		},
	}),
	item("agent", {
		kind: "agent_update",
		label: { code: "agent_update" },
		state: "done",
		finishedAt: NOW_MS - 5_000,
		detail: { code: "done" },
		actions: ["open", "dismiss"],
	}),
	item("reboot", {
		kind: "reboot",
		label: { code: "reboot" },
		state: "failed",
		finishedAt: NOW_MS - 2_000,
		detail: { code: "rejected" },
		actions: ["dismiss"],
		href: undefined,
	}),
];

interface AtProps {
	children: ReactNode;
}

function At({ children }: Readonly<AtProps>) {
	return (
		<area.AreaNowContext.Provider value={NOW_MS}>
			{children}
		</area.AreaNowContext.Provider>
	);
}

const sectionTitles = (root: ParentNode) =>
	Array.from(root.querySelectorAll("[data-tray-section] h3"), (el) => {
		return (el.textContent ?? "").replace(/\s+/g, " ").trim();
	});

const idOf = (entry: ActivityItem) => entry.id;
const opOf = (el: Element) => el.getAttribute("data-op");

const opsIn = (root: ParentNode, section: string) =>
	Array.from(
		root.querySelectorAll(`[data-tray-section=${section}] [data-op]`),
		opOf,
	);

const ACCOUNT: DevicesScope = { kind: "account" };

async function mountTray(options: MountDevicesOptions = {}) {
	const navigations: DevicesRoute[] = [];
	const navigate = (route: DevicesRoute) => {
		navigations.push(route);
	};
	const mounted = await mountDevices(
		<div className="relative">
			<ActivityButton />
			<ActivityTray scope={ACCOUNT} onNavigate={navigate} />
		</div>,
		options,
	);
	return { mounted, navigations };
}

afterEach(async () => {
	useOverlayStore.getState().close();
	await cleanupDevices();
	await dom.cleanup();
	useActivityTray.setState({ open: false });
});
afterAll(dom.restore);

describe("tray model", () => {
	test("groups In progress, No reply received and Finished; counts what is in flight", () => {
		const sections = traySections(ITEMS);
		expect(sections.progress.map(idOf)).toEqual(["rollout", "upload", "rules"]);
		expect(sections.unknown.map(idOf)).toEqual(["stop"]);
		expect(sections.finished.map(idOf)).toEqual(["agent", "reboot"]);
		expect(trayCounts(ITEMS)).toEqual({ inFlight: 2, active: 1 });
		expect(trayCounts([])).toEqual({ inFlight: 0, active: 0 });
	});

	test("every kind has a title; commands name their verb; no code reaches the text", () => {
		const kinds: ActivityKind[] = [
			"safe_update",
			"upload",
			"access_rules",
			"account_backup",
			"agent_update",
			"reboot",
			"command",
			"secret_write",
			"history_readers",
			"metric_readers",
			"offline_write_retry",
			"signing_request",
			"setup",
			"event_run",
		];
		for (const kind of kinds) {
			const title = activityTitle(
				t,
				item("x", { kind, label: { code: kind } }),
			);
			expect(title.length).toBeGreaterThan(2);
			expect(title).not.toContain("_");
		}
		expect(activityTitle(t, ITEMS[3])).toBe("Stop");
		const quick = item("q", {
			kind: "command",
			label: { code: "command", params: { command: "apply" } },
		});
		expect(activityTitle(t, quick)).toBe("Quick update");
		expect(activityTarget(ITEMS[0])).toBe("edge-berlin-01 › invoice-extractor");
		expect(
			activityTarget(
				item("y", { target: { deviceId: "5b794764-aaaa" } }),
				() => "warehouse-pi",
			),
		).toBe("warehouse-pi");
		expect(
			activityTarget(item("z", { target: { deviceId: "5b794764-aaaa" } })),
		).toBe("5b794764");
	});

	test("details state progress, deadlines and waits in sentences", () => {
		expect(activityDetail(t, TIME, ITEMS[0])).toBe(
			"1 of 2 instances ready · time limit 87 s",
		);
		expect(activityDetail(t, TIME, ITEMS[1])).toBe(
			`Paused. You can resume until at:${(NOW_MS + 3_600_000) / 1000}.`,
		);
		expect(activityDetail(t, TIME, ITEMS[2])).toBe(
			"Saved. The device applies it the next time it checks in.",
		);
		expect(activityDetail(t, TIME, ITEMS[3])).toBeUndefined();
		expect(activityDetail(t, TIME, ITEMS[5])).toBe("The device refused this.");
		expect(
			activityDetail(
				t,
				TIME,
				item("f", {
					kind: "upload",
					detail: { code: "files_progress" },
					progress: { done: 12, total: 38, unit: "files" },
				}),
			),
		).toBe("12 of 38 files");
	});

	test("a run started from Devices: its own title and waiting sentence, nothing of its input or output", () => {
		const run = item("run", {
			kind: "event_run",
			label: { code: "event_run" },
			state: "waiting",
			detail: { code: "waiting_for_device" },
			target: {
				deviceId: "5b794764-0000-4000-8000-000000000001",
				deviceName: "edge-berlin-01",
				serviceId: "shop-assistant",
				eventId: "evt_shop_return",
			},
			href: undefined,
		});
		expect(activityTitle(t, run)).toBe("Action or form run");
		expect(activityDetail(t, TIME, run)).toBe(
			"Sent. Waiting for the device to start the run.",
		);
		expect(activityTarget(run)).toBe("edge-berlin-01 › shop-assistant");
		expect(activityRoute(run)).toEqual({
			screen: "service",
			deviceId: "5b794764-0000-4000-8000-000000000001",
			serviceId: "shop-assistant",
			tab: "status",
		});
	});
});

describe("where a tray item opens", () => {
	test("its own link first, else the page of what it works on", () => {
		const device = "5b794764-0000-4000-8000-000000000001";
		expect(activityRoute(ITEMS[0])).toEqual(ITEMS[0].href as DevicesRoute);
		const bare = (over: Partial<ActivityItem>) =>
			activityRoute(item("x", { href: undefined, ...over }));
		expect(bare({ kind: "upload", target: { deviceId: device } })).toEqual({
			screen: "deploy",
			deviceIds: [device],
			mode: "update",
			step: "copy_upload",
		});
		expect(bare({ kind: "secret_write" })).toEqual({
			screen: "service",
			deviceId: device,
			serviceId: "invoice-extractor",
			tab: "configuration",
		});
		expect(
			bare({ kind: "access_rules", target: { deviceId: device } }),
		).toEqual({ screen: "device", deviceId: device, tab: "access" });
		expect(bare({ kind: "command", target: { deviceId: device } })).toEqual({
			screen: "device",
			deviceId: device,
			tab: "overview",
		});
		expect(
			bare({
				kind: "setup",
				resume: { type: "setup", enrollmentId: "enr-1" },
			}),
		).toEqual({ screen: "setup", enrollmentId: "enr-1" });
	});
});

describe("activity tray", () => {
	test("closed: nothing is rendered", async () => {
		const { container } = await dom.render(
			<At>
				<ActivityTrayView open={false} onOpenChange={() => {}} items={ITEMS} />
			</At>,
		);
		expect(container.querySelector("aside")).toBeNull();
		expect(queryByRole("dialog")).toBeNull();
	});

	test("open: an in-flow drawer with three counted sections and the persistence note", async () => {
		const { container } = await dom.render(
			<At>
				<ActivityTrayView open onOpenChange={() => {}} items={ITEMS} />
			</At>,
		);
		const aside = container.querySelector("aside") as HTMLElement;
		expect(aside.id).toBe(ACTIVITY_TRAY_ID);
		expect(aside.getAttribute("aria-label")).toBe("Activity");
		expect(aside.className).toContain("absolute");
		expect(container.innerHTML).not.toContain("fixed");
		expect(queryByRole("dialog")).toBeNull();
		expect(sectionTitles(container)).toEqual([
			"In progress · 3",
			"No reply received · 1",
			"Finished · 2",
		]);
		expect(opsIn(container, "progress")).toEqual([
			"safe_update",
			"upload",
			"access_rules",
		]);
		expect(opsIn(container, "unknown")).toEqual(["command"]);
		expect(opsIn(container, "finished")).toEqual(["agent_update", "reboot"]);
		expect(aside.textContent).toContain("Kept on this computer");
		expect(aside.textContent).toContain(
			"Results stay until you dismiss them, across reloads and reconnects.",
		);
		expect(aside.textContent).toContain("It may have run.");
		expect(aside.textContent).not.toContain("safe_update");
	});

	test("empty sections say so instead of looking broken", async () => {
		const { container } = await dom.render(
			<At>
				<ActivityTrayView open onOpenChange={() => {}} items={[]} />
			</At>,
		);
		expect(sectionTitles(container)).toEqual([
			"In progress · 0",
			"No reply received · 0",
			"Finished · 0",
		]);
		expect(container.textContent).toContain("Nothing is running.");
		expect(container.textContent).toContain(
			"Every command you sent got a reply.",
		);
		expect(container.textContent).toContain("Nothing has finished yet.");
	});

	test("Check result, Dismiss and Open report the item; unfinished work offers no Dismiss link", async () => {
		const actions: [string, ActivityAction][] = [];
		const opened: string[] = [];
		await dom.render(
			<At>
				<ActivityTrayView
					open
					onOpenChange={() => {}}
					items={ITEMS}
					onAction={(entry, action) => actions.push([entry.id, action])}
					onOpen={(entry) => opened.push(entry.id)}
					busy={new Set(["nothing"])}
				/>
			</At>,
		);
		await click(byRole("button", "Check result"));
		await click(byRole("button", "Resume"));
		await click(byRole("button", "Discard…"));
		const dismiss = allByRole("button", "Dismiss");
		expect(dismiss).toHaveLength(3);
		for (const button of dismiss) await click(button);
		expect(actions).toEqual([
			["stop", "check_again"],
			["upload", "resume"],
			["upload", "discard"],
			["stop", "dismiss"],
			["agent", "dismiss"],
			["reboot", "dismiss"],
		]);
		const open = allByRole("button", "Open");
		expect(open).toHaveLength(ITEMS.length);
		await click(open[0]);
		expect(opened).toEqual(["rollout"]);
	});

	test("a result check in flight shows on its button", async () => {
		await dom.render(
			<At>
				<ActivityTrayView
					open
					onOpenChange={() => {}}
					items={ITEMS}
					busy={new Set(["stop"])}
				/>
			</At>,
		);
		expect(byRole("button", "Check result").getAttribute("aria-busy")).toBe(
			"true",
		);
	});

	test("Esc and the close button close it; a key another layer handled does not", async () => {
		const changes: boolean[] = [];
		await dom.render(
			<At>
				<ActivityTrayView
					open
					onOpenChange={(open) => changes.push(open)}
					items={ITEMS}
				/>
			</At>,
		);
		const handled = new window.KeyboardEvent("keydown", {
			key: "Escape",
			bubbles: true,
			cancelable: true,
		}) as unknown as KeyboardEvent;
		handled.preventDefault();
		await fire(document.body, handled);
		expect(changes).toEqual([]);
		await keyDown(document.body, "Enter");
		expect(changes).toEqual([]);
		await keyDown(document.body, "Escape");
		expect(changes).toEqual([false]);
		await click(byRole("button", "Close activity"));
		expect(changes).toEqual([false, false]);
	});

	test("phone: the same tray as a modal sheet", async () => {
		const changes: boolean[] = [];
		const { container } = await dom.render(
			<At>
				<ActivityTrayView
					open
					sheet
					onOpenChange={(open) => changes.push(open)}
					items={ITEMS}
				/>
			</At>,
		);
		await settle();
		expect(container.querySelector("aside")).toBeNull();
		const sheet = byRole("dialog", "Activity");
		expect(sectionTitles(sheet)).toHaveLength(3);
		await click(byRole("button", "Close activity"));
		expect(changes).toEqual([false]);
	});

	test("the open state is one store the button and screens share", () => {
		const store = useActivityTray.getState();
		expect(store.open).toBe(false);
		store.toggle();
		expect(useActivityTray.getState().open).toBe(true);
		useActivityTray.getState().setOpen(false);
		expect(useActivityTray.getState().open).toBe(false);
	});
});

describe("activity over the workspace", () => {
	test("the button counts what is in flight and opens the tray with this account's items", async () => {
		const { mounted } = await mountTray();
		const button = byRole("button", "Activity: 3 in progress");
		expect(button.getAttribute("aria-expanded")).toBe("false");
		expect(mounted.container.querySelector("aside")).toBeNull();
		await click(button);
		await mounted.settle();
		expect(button.getAttribute("aria-expanded")).toBe("true");
		const aside = mounted.container.querySelector("aside") as HTMLElement;
		expect(aside.id).toBe(button.getAttribute("aria-controls") ?? "");
		expect(sectionTitles(aside)).toEqual([
			"In progress · 4",
			"No reply received · 0",
			"Finished · 1",
		]);
		expect(aside.textContent).toContain("edge-berlin-01 › invoice-extractor");
		expect(aside.textContent).not.toMatch(/\b[a-z]+_[a-z_]+\b/);
		await keyDown(document.body, "Escape");
		await mounted.settle();
		expect(mounted.container.querySelector("aside")).toBeNull();
		expect(button.getAttribute("aria-expanded")).toBe("false");
	});

	test("Dismiss removes a finished result; Open and Resume go to the item's page", async () => {
		const { mounted, navigations } = await mountTray();
		const { activity } = mounted.fake.workspace;
		await click(byRole("button", /^Activity/));
		await mounted.settle();
		const finished = activity
			.list()
			.filter((entry) => entry.state === "done").length;
		await click(byRole("button", "Dismiss"));
		await mounted.settle();
		expect(
			activity.list().filter((entry) => entry.state === "done"),
		).toHaveLength(finished - 1);
		expect(sectionTitles(mounted.container)).toContain("Finished · 0");

		await click(allByRole("button", "Open")[0]);
		await click(byRole("button", "Resume"));
		expect(navigations).toHaveLength(2);
		expect(navigations[0]).toMatchObject({ deviceId: SAMPLE_IDS.edge });
		expect(navigations[1]).toMatchObject({ screen: "deploy" });
		expect(mounted.container.querySelector("aside")).not.toBeNull();
	});

	test("Check result on a device whose keys are closed asks to unlock first", async () => {
		const { mounted } = await mountTray();
		const { activity } = mounted.fake.workspace;
		await act(async () => {
			const id = activity.start({
				kind: "command",
				target: { deviceId: SAMPLE_IDS.lab, deviceName: "lab-gpu-02" },
				state: "active",
				label: { code: "command", params: { command: "restart" } },
				startedBy: "you",
				actions: [],
				resume: {
					type: "operation",
					operationId: "op-lab-1",
					command: "restart",
					issuedAt: Math.floor(mounted.fake.clock.now() / 1000),
				},
			});
			activity.finish(id, "unknown", { code: "no_reply" });
		});
		await click(byRole("button", /^Activity/));
		await mounted.settle();
		expect(sectionTitles(mounted.container)).toContain("No reply received · 1");
		const commands = mounted.fake.api.commands.length;
		await click(byRole("button", "Check result"));
		await mounted.settle();
		expect(useOverlayStore.getState().overlay).toMatchObject({
			kind: "unlock",
			deviceId: SAMPLE_IDS.lab,
			connectLive: true,
		});
		expect(mounted.fake.api.commands).toHaveLength(commands);
	});

	test("a finished item raises a toast unless its page, or the Rollout step of its own run, shows it", async () => {
		const { toast } = await import("sonner");
		const edge = SAMPLE_IDS.edge;
		const rollout: DevicesRoute = {
			screen: "deploy",
			deviceIds: [edge, SAMPLE_IDS.studio],
			appId: "app_visitor_checkin",
			step: "rollout",
		};
		const raised = async (
			route: DevicesRoute | undefined,
			deviceId: string = edge,
		) => {
			const mounted = await mountDevices(
				<ActivityTray scope={ACCOUNT} onNavigate={() => {}} route={route} />,
			);
			const { activity } = mounted.fake.workspace;
			const before = toast.getHistory().length;
			await act(async () => {
				const id = activity.start({
					kind: "safe_update",
					target: { deviceId, serviceId: "check-in-page" },
					state: "active",
					label: { code: "safe_update" },
					startedBy: "you",
					actions: [],
					href: rollout,
				});
				activity.finish(id, "done", { code: "done" });
			});
			const count = toast.getHistory().length - before;
			await cleanupDevices();
			return count;
		};
		expect(await raised(undefined)).toBe(1);
		expect(await raised({ screen: "fleet", view: "devices" })).toBe(1);
		expect(await raised(rollout)).toBe(0);
		expect(await raised({ ...rollout, step: "review" })).toBe(1);
		expect(await raised({ ...rollout, appId: "app_crm_sync" })).toBe(1);
		expect(await raised(rollout, SAMPLE_IDS.warehouse)).toBe(1);
		expect(
			await raised({ screen: "device", deviceId: edge, tab: "services" }),
		).toBe(0);
	});

	test("phone: the tray is a modal sheet", async () => {
		const { mounted } = await mountTray({ widthBucket: "phone" });
		await click(byRole("button", /^Activity/));
		await mounted.settle();
		expect(mounted.container.querySelector("aside")).toBeNull();
		expect(sectionTitles(byRole("dialog", "Activity"))).toHaveLength(3);
	});

	test("older hub: the tray is this computer's record and reads the same", async () => {
		const { mounted } = await mountTray({ hubVersion: "old" });
		await click(byRole("button", "Activity: 3 in progress"));
		await mounted.settle();
		expect(sectionTitles(mounted.container)[0]).toBe("In progress · 4");
	});
});
