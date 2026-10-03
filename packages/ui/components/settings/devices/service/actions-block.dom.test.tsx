import { afterAll, afterEach, describe, expect, test } from "bun:test";
import { act } from "react";
import {
	advance,
	allByRole,
	byRole,
	click,
	inPortal,
	installDom,
	queryByRole,
} from "../testing/dom-harness";
import type { FakeWorkspaceOptions } from "../testing/fake-workspace";

const dom = installDom();
const { cleanupDevices, mountDevices, preloadDevices } = await import(
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
const { createFakeWorkspace } = await import("../testing/fake-workspace");
const { QUICK_REPLY, serveQuickReplyOnEdge } = await import(
	"../testing/schedule-scenarios"
);
const { useDevicesRoute } = await import("../routing/use-devices-route");
const { useOverlayStore } = await import("../workspace/overlay-store");
const { runPace } = await import("../run/run-store");

Object.assign(runPace, { fastMs: 15, fastForMs: 10_000, slowMs: 15 });
const { ServiceScreen } = await import("./service-screen");
const { SHOP, openShop, stopShop, text } = await import("./status-test-kit");

afterEach(async () => {
	await cleanupDevices();
	await dom.cleanup();
	useOverlayStore.getState().close();
});
afterAll(dom.restore);

type Fake = Awaited<ReturnType<typeof createFakeWorkspace>>;
type Row = NonNullable<ReturnType<ReturnType<Fake["agent"]>["placement"]>>;

function Page() {
	const { route, scope } = useDevicesRoute();
	if (route.screen !== "service")
		return <p data-left="">{`left:${route.screen}`}</p>;
	return (
		<ServiceScreen
			route={route}
			scope={scope}
			deviceId={route.deviceId}
			serviceId={route.serviceId}
		/>
	);
}

interface OpenOptions {
	/** Changes the world after the service took the quick action and before the page opens. */
	arrange?(row: Row, fake: Fake): void | Promise<void>;
	/** `status`: another computer with the published status only; `locked`: one without the keys. */
	reader?: "status" | "locked";
	agentFeatures?: FakeWorkspaceOptions["agentFeatures"];
}

/** support-bot (local-only) on edge-berlin-01 with Support Portal's quick action, opened on its Status tab. */
async function open({
	arrange,
	reader,
	agentFeatures,
	overlays,
}: OpenOptions & { overlays?: boolean } = {}) {
	const world = await createFakeWorkspace(undefined, {
		...(agentFeatures ? { agentFeatures } : {}),
	});
	const row = await serveQuickReplyOnEdge(world);
	if (!row) throw new Error("support-bot is not on the device.");
	await arrange?.(row, world);
	world.api.hub.publishStatus(
		QUICK_REPLY.device,
		world.agent(QUICK_REPLY.device),
	);
	if (reader === "status") world.agent(QUICK_REPLY.device).online = false;
	const fake = reader
		? await createFakeWorkspace(undefined, {
				api: world.api,
				viewFacts: false,
				...(reader === "locked" ? { unlock: "none" as const } : {}),
			})
		: world;
	const view = await mountDevices(<Page />, {
		fake,
		search: `device=${QUICK_REPLY.device}&service=${QUICK_REPLY.service}&tab=status`,
		...(overlays ? { overlays } : {}),
	});
	await view.settle();
	return view;
}

const block = (root: ParentNode) =>
	root.querySelector<HTMLElement>("#service-actions");
const action = (root: ParentNode, eventId: string = QUICK_REPLY.event) =>
	root.querySelector<HTMLElement>(`[data-action="${eventId}"]`);
const lines = (root: ParentNode, eventId: string = QUICK_REPLY.event) =>
	[
		...(action(root, eventId)?.querySelectorAll("[data-action-line]") ?? []),
	].map((line) => text(line));
const runNow = (root: ParentNode, eventId: string = QUICK_REPLY.event) =>
	action(root, eventId)?.querySelector<HTMLElement>("[data-run-now]") ?? null;
const entryOf = (row: Row) => {
	const found = row.actions?.find(
		(entry) => entry.event_id === QUICK_REPLY.event,
	);
	if (!found) throw new Error("The service reports no quick action.");
	return found;
};

describe("Service › Status · Actions and forms", () => {
	test("a quick action the service runs: its kind, its numbers and Run now… that opens the sheet", async () => {
		const { container } = await open();
		const item = action(container) as HTMLElement;
		expect(item.dataset.actionState).toBe("reported");
		expect(text(item)).toContain("Quick reply");
		expect(text(item)).toContain("Quick action");
		expect(lines(container)).toEqual(["No runs since the service started."]);
		expect(text(block(container))).toContain(
			"Everyone who may start support-bot can run these from Devices.",
		);
		const button = runNow(container) as HTMLElement;
		expect(text(button)).toBe("Run now…");
		expect(button.getAttribute("aria-disabled")).toBeNull();
		await click(button);
		expect(useOverlayStore.getState().overlay).toEqual({
			kind: "run_now",
			deviceId: QUICK_REPLY.device,
			serviceId: QUICK_REPLY.service,
			eventId: QUICK_REPLY.event,
		});
	});

	test("the device's numbers: runs going now, the last run and how many failed", async () => {
		const { container } = await open({
			arrange: (row) => {
				Object.assign(entryOf(row), {
					running: 2,
					runs: 7,
					failed: 1,
					last_at: SAMPLE_NOW - 300,
					last_outcome: "failed",
				});
			},
		});
		expect(lines(container)).toEqual([
			"2 runs are going now.",
			"Last run 5 min. ago · failed",
			"7 runs since the service started · 1 failed",
		]);
	});

	test("from the published status: what it is, no numbers, and Run now… says why it can't run from here", async () => {
		const { container } = await open({
			reader: "status",
			arrange: (row) => {
				Object.assign(entryOf(row), { runs: 7, last_outcome: "failed" });
			},
		});
		expect(text(action(container))).toContain("Quick action");
		expect(lines(container)).toEqual([]);
		const button = runNow(container) as HTMLElement;
		expect(button.getAttribute("aria-disabled")).toBe("true");
		expect(
			action(container)?.querySelector("[data-gate-inline]"),
		).not.toBeNull();
		await click(button);
		expect(useOverlayStore.getState().overlay.kind).toBe("none");
	});

	test("without this computer's keys nothing is said about it and nothing can run", async () => {
		const { container } = await open({ reader: "locked" });
		const item = action(container);
		if (item) {
			expect(item.dataset.actionState).toBe("locked");
			expect(lines(container)).toEqual(["Unknown until unlocked"]);
			expect(runNow(container)).toBeNull();
		}
		expect(text(container)).not.toContain("No runs since");
	});

	test("an agent without forms and quick actions: the row asks for the agent update instead of Run now…", async () => {
		const features = Object.fromEntries(
			AGENT_FEATURES.filter((flag) => flag !== "on_demand_events").map(
				(flag) => [flag, 1 as const],
			),
		);
		const { container } = await open({ agentFeatures: features });
		expect(action(container)?.dataset.actionState).toBe("needs_agent");
		expect(lines(container)).toEqual([
			"Update the device agent to run actions from here.",
		]);
		expect(runNow(container)).toBeNull();
	});

	test("a stopped service: it can't be run, and says so instead of Run now…", async () => {
		const { container } = await open({
			arrange: (row) => {
				Object.assign(row, {
					desired_state: "stopped",
					observed_state: "stopped",
					running_replicas: 0,
					ready_replicas: 0,
					actions: undefined,
				});
			},
		});
		expect(action(container)?.dataset.actionState).toBe("stopped");
		expect(lines(container)).toEqual([
			"Can't be run while the service is not running.",
		]);
		expect(runNow(container)).toBeNull();
	});

	test("a running service that has not said anything about it yet still offers Run now…", async () => {
		const { container } = await open({
			arrange: (row) => {
				row.actions = undefined;
			},
		});
		expect(action(container)?.dataset.actionState).toBe("not_reported");
		expect(lines(container)).toEqual(["Not reported yet."]);
		expect(runNow(container)).not.toBeNull();
	});
});

describe("Service › Status · a form that takes a file", () => {
	test("its fields, and the service page instead of Run now…", async () => {
		const { container } = await openShop({
			events: [SHOP.orders, SHOP.form],
		});
		const item = action(container, SHOP.form) as HTMLElement;
		expect(text(item)).toContain("Return request");
		expect(text(item)).toContain("Form · 3 fields");
		expect(runNow(container, SHOP.form)).toBeNull();
		const file = item.querySelector("[data-action-file]") as HTMLElement;
		expect(text(file)).toContain(
			"This form takes a file. Open it on the service page.",
		);
		const page = byRole("link", "Service page", file);
		expect(page.getAttribute("href")).toContain("tab=endpoint");
	});

	test("a service without a page says that a file can't be sent to it", async () => {
		const { container } = await openShop({ events: [SHOP.form] });
		const item = action(container, SHOP.form) as HTMLElement;
		expect(text(item.querySelector("[data-action-file]"))).toBe(
			"This form takes a file. Files can only be sent from a service page, and this service has none.",
		);
		expect(runNow(container, SHOP.form)).toBeNull();
		expect(queryByRole("link", "Service page", container)).toBeNull();
	});

	test("a stopped service says only that it can't run", async () => {
		const { container } = await openShop({
			events: [SHOP.orders, SHOP.form],
			arrange: stopShop,
		});
		expect(lines(container, SHOP.form)).toEqual([
			"Can't be run while the service is not running.",
		]);
		expect(text(action(container, SHOP.form))).toContain("Form");
		expect(runNow(container, SHOP.form)).toBeNull();
	});
});

test("a service without quick actions or forms shows no such block", async () => {
	const { container } = await openShop({ events: [SHOP.once] });
	expect(block(container)).toBeNull();
});

describe("Service › Status · a run you started", () => {
	test("Run now… runs it in the sheet; the Status tab keeps it under Runs you started", async () => {
		const view = await open({ overlays: true });
		const { container } = view;
		view.fake.agent(QUICK_REPLY.device).runs.script = {
			queuedReads: 0,
			runningReads: 0,
			end: { output: { reply: "Thanks" } },
		};
		await click(runNow(container) as HTMLElement);
		await view.settle();
		const sheet = byRole("dialog");
		await click(byRole("button", "Run", sheet));
		for (let round = 0; round < 160; round++) {
			if (document.querySelector("[data-run-phase=ended]")) break;
			await advance(25);
			await view.settle();
		}
		expect(text(sheet)).toContain("The run succeeded.");
		await act(async () => useOverlayStore.getState().close());
		await view.settle();
		const started = container.querySelector<HTMLElement>(
			"#service-runs-you-started",
		);
		expect(text(started)).toContain("Quick reply");
		expect(text(started)).toContain("Succeeded");
		// The output stays on the device: this computer keeps none of it.
		expect(text(started)).not.toContain("Thanks");
	});
});

describe("Service · the actions menu", () => {
	const menuOf = async (container: HTMLElement, service: string) => {
		await click(byRole("button", `More actions for ${service}`, container));
		return inPortal("menu");
	};

	test("Run {event} now… for each quick action and form opens the sheet", async () => {
		const { container } = await open();
		const menu = await menuOf(container, QUICK_REPLY.service);
		const item = byRole("menuitem", /^Run Quick reply now…/, menu);
		await click(item);
		expect(useOverlayStore.getState().overlay).toEqual({
			kind: "run_now",
			deviceId: QUICK_REPLY.device,
			serviceId: QUICK_REPLY.service,
			eventId: QUICK_REPLY.event,
		});
	});

	test("a form that takes a file is off, with where to open it", async () => {
		const { container } = await openShop({ events: [SHOP.orders, SHOP.form] });
		const menu = await menuOf(container, SHOP.service);
		const item = byRole("menuitem", /^Run Return request now…/, menu);
		expect(item.getAttribute("aria-disabled") ?? item.dataset.disabled).toMatch(
			/true|^$/,
		);
		expect(text(item)).toContain(
			"This form takes a file. Open it on the service page.",
		);
		await click(item);
		expect(useOverlayStore.getState().overlay.kind).toBe("none");
	});

	test("a service without quick actions or forms has no Run … now", async () => {
		const { container } = await openShop({ events: [SHOP.once] });
		const menu = await menuOf(container, SHOP.service);
		expect(
			allByRole("menuitem", undefined, menu).map(text).join(" | "),
		).not.toContain("now…");
	});
});
