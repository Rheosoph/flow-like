import { afterAll, afterEach, describe, expect, test } from "bun:test";
import { act } from "react";
import type { ManagementResponse } from "../../../../lib/device-management/types";
import {
	click,
	clickByText,
	inPortal,
	installDom,
	queryByRole,
	typeInto,
} from "../testing/dom-harness";
import type { FakeWorkspace } from "../testing/fake-workspace";
import type { MountDevicesOptions } from "../testing/mount-devices";
import type * as Hooks from "./index";

const dom = installDom();
const { cleanupDevices, mountDevices, preloadDevices } = await import(
	"../testing/mount-devices"
);
await preloadDevices();
const hooks = await import("./index");
const { InlineResult } = await import("../primitives/inline-result");
const { ApiResponseError } = await import("../../../../lib/api-error");
const { deviceKeys } = await import(
	"../../../../lib/device-management/hub/queries"
);
const { SAMPLE_IDS, SAMPLE_NOW } = await import(
	"../../../../lib/device-management/model/__fixtures__/sample-fleet"
);

const EDGE = SAMPLE_IDS.edge;
const SERVICE = "support-bot";
const KEY = `service:${EDGE}/${SERVICE}`;
const STOP = { type: "stop", placement_id: SERVICE, expected_revision: 7 };
const ROWS = {
	what: "Its 2 instances stop.",
	who: "The service stops answering.",
	when: "Immediately.",
	undo: { reversible: true, text: "Start runs the same settings again." },
};

afterEach(async () => {
	await cleanupDevices();
	await dom.cleanup();
});
afterAll(dom.restore);

type Outcome = Hooks.DeviceActionOutcome<ManagementResponse>;
type Request = Partial<Hooks.DeviceActionRequest<ManagementResponse>>;

function Controls({
	request,
	outcomes,
}: Readonly<{ request: Request; outcomes: Outcome[] }>) {
	const actions = hooks.useDeviceAction();
	const results = hooks.useInlineResults(request.resultKey ?? KEY);
	const run = () =>
		void actions
			.run<ManagementResponse>({
				action: "stop",
				deviceId: EDGE,
				target: { placementId: SERVICE },
				label: "Stop support-bot",
				consequence: ROWS,
				resultKey: KEY,
				call: (context) => context.request(STOP),
				activity: { kind: "command", serviceId: SERVICE },
				...request,
			})
			.then((outcome) => outcomes.push(outcome));
	return (
		<div>
			<button
				type="button"
				data-trigger=""
				aria-busy={actions.pending(request.resultKey ?? KEY)}
				onClick={run}
			>
				Stop…
			</button>
			{results.map((result) => (
				<InlineResult
					key={result.id}
					tone={result.tone}
					onDismiss={result.dismiss}
				>
					{result.text}
				</InlineResult>
			))}
		</div>
	);
}

/** The golden fleet: edge-berlin-01 is unlocked and connected, support-bot runs two instances. */
async function mount(
	request: Request | ((fake: FakeWorkspace) => Request) = {},
	options: MountDevicesOptions = {},
) {
	const outcomes: Outcome[] = [];
	const view = await mountDevices(
		({ fake }) => (
			<Controls
				request={typeof request === "function" ? request(fake) : request}
				outcomes={outcomes}
			/>
		),
		options,
	);
	const { api, workspace } = view.fake;
	return {
		view,
		outcomes,
		api,
		agent: api.agent(EDGE),
		trigger: view.container.querySelector("[data-trigger]") as HTMLElement,
		result: () => view.container.querySelector("[data-result]"),
		confirmButton: () =>
			inPortal("alertdialog").querySelector("[data-confirm]") as HTMLElement,
		/** The stop commands the device received. */
		stops: () =>
			api.commands
				.filter(([device, type]) => device === EDGE && type === "stop")
				.map(([, , command]) => command),
		/** The tray items of this service. */
		tray: () =>
			workspace.activity
				.list()
				.filter((item) => item.target.serviceId === SERVICE),
	};
}

describe("gate → confirm → one call", () => {
	test("a failing gate sends nothing and opens no confirm", async () => {
		const { view, api, outcomes, trigger, tray } = await mount(
			{},
			{ unlock: "none" },
		);
		const before = [api.calls.length, api.commands.length];
		await click(trigger);
		await view.settle();
		expect(outcomes).toMatchObject([{ status: "gated", gate: { gate: "G7" } }]);
		expect(queryByRole("alertdialog")).toBeNull();
		expect([api.calls.length, api.commands.length]).toEqual(before);
		expect(tray()).toEqual([]);
	});

	test("the confirm shows the consequence rows; Cancel sends nothing and leaves no trace", async () => {
		const { view, api, outcomes, trigger, result, stops, tray } = await mount();
		const writes = api.writes().length;
		await click(trigger);
		const sheet = inPortal("alertdialog");
		expect(sheet.textContent).toContain("Stop support-bot?");
		for (const kind of ["what", "who", "when", "undo"])
			expect(sheet.querySelector(`[data-kind="${kind}"]`)).not.toBeNull();
		expect(sheet.textContent).toContain("Its 2 instances stop.");

		await clickByText("Cancel", sheet);
		await view.settle();
		expect(outcomes).toEqual([{ status: "cancelled" }]);
		expect(stops()).toEqual([]);
		expect(api.writes().length).toBe(writes);
		expect(tray()).toEqual([]);
		expect(result()).toBeNull();
		expect(trigger.getAttribute("aria-busy")).toBe("false");
	});

	test("Confirm sends exactly one command, tracks it in the tray and leaves an inline result until dismissed", async () => {
		const { view, outcomes, trigger, result, confirmButton, stops, tray } =
			await mount();
		await click(trigger);
		await click(confirmButton());
		await view.settle();
		expect(stops()).toEqual([STOP]);
		expect(outcomes).toMatchObject([
			{ status: "done", result: { state: "completed" } },
		]);
		expect(queryByRole("alertdialog")).toBeNull();
		expect(tray()).toMatchObject([
			{
				kind: "command",
				state: "done",
				target: { deviceId: EDGE, serviceId: SERVICE },
				label: { code: "command", params: { action: "stop" } },
			},
		]);
		expect(result()?.getAttribute("data-result")).toBe("good");
		expect(result()?.textContent).toContain("Stop support-bot: done.");
		expect(trigger.getAttribute("aria-busy")).toBe("false");

		await clickByText("Dismiss");
		expect(result()).toBeNull();
	});

	test("a double click on the control opens one confirm and sends one command", async () => {
		const { view, outcomes, trigger, confirmButton, stops } = await mount();
		await click(trigger);
		await click(trigger);
		await view.settle();
		expect(outcomes).toEqual([{ status: "busy" }]);
		expect(document.querySelectorAll('[role="alertdialog"]').length).toBe(1);

		const confirm = confirmButton();
		await act(async () => {
			confirm.click();
			confirm.click();
		});
		await view.settle();
		expect(stops().length).toBe(1);
		expect(outcomes.map((outcome) => outcome.status)).toEqual(["busy", "done"]);
	});

	test("the control is busy from the click until the device answered", async () => {
		const { view, agent, trigger, result, confirmButton, stops, tray } =
			await mount();
		const answer = agent.hold("stop");
		await click(trigger);
		expect(trigger.getAttribute("aria-busy")).toBe("true");
		await click(confirmButton());
		await view.settle();
		expect(stops().length).toBe(1);
		expect(trigger.getAttribute("aria-busy")).toBe("true");
		expect(result()?.getAttribute("data-result")).toBe("info");
		expect(tray()).toMatchObject([{ kind: "command", state: "active" }]);

		await act(async () => answer());
		await view.settle();
		expect(trigger.getAttribute("aria-busy")).toBe("false");
		expect(result()?.getAttribute("data-result")).toBe("good");
	});
});

describe("confirmation strength", () => {
	test("check: the confirm stays disabled until the acknowledgement is ticked", async () => {
		const { view, trigger, confirmButton, stops } = await mount({
			strength: "check",
			confirm: { checkLabel: "Interrupt the running services now" },
		});
		await click(trigger);
		expect(confirmButton().getAttribute("aria-disabled")).toBe("true");
		await click(confirmButton());
		await view.settle();
		expect(stops()).toEqual([]);

		await clickByText("Interrupt the running services now");
		expect(confirmButton().getAttribute("aria-disabled")).toBeNull();
		await click(confirmButton());
		await view.settle();
		expect(stops().length).toBe(1);
	});

	test("typed: only the exact text enables the confirm", async () => {
		const { view, trigger, confirmButton, stops } = await mount({
			strength: "typed",
			confirm: { typed: SERVICE, tone: "danger" },
		});
		await click(trigger);
		const input = inPortal("alertdialog").querySelector("input") as Element;
		await typeInto(input, "support-");
		expect(confirmButton().getAttribute("aria-disabled")).toBe("true");
		await click(confirmButton());
		await view.settle();
		expect(stops()).toEqual([]);
		await typeInto(input, SERVICE);
		expect(confirmButton().getAttribute("aria-disabled")).toBeNull();
		await click(confirmButton());
		await view.settle();
		expect(stops().length).toBe(1);
	});

	test("without consequence rows the screen confirmed inline: the call goes out at once", async () => {
		const { view, trigger, outcomes, stops } = await mount({
			consequence: undefined,
		});
		await click(trigger);
		await view.settle();
		expect(queryByRole("alertdialog")).toBeNull();
		expect(stops().length).toBe(1);
		expect(outcomes[0]?.status).toBe("done");
	});
});

describe("outcomes", () => {
	test("a definitive device rejection shows the device's reason and frees the control", async () => {
		const {
			view,
			agent,
			outcomes,
			trigger,
			result,
			confirmButton,
			stops,
			tray,
		} = await mount();
		const accept = agent.reject(
			"stop",
			"unauthorized",
			"Grant reader-7 lacks stop on this placement.",
		);
		await click(trigger);
		await click(confirmButton());
		await view.settle();

		expect(outcomes).toMatchObject([
			{
				status: "rejected",
				rejection: { code: "unauthorized" },
				failure: { code: "rejected_unauthorized" },
			},
		]);
		expect(result()?.getAttribute("data-result")).toBe("critical");
		expect(result()?.textContent).toContain(
			"Grant reader-7 lacks stop on this placement.",
		);
		expect(tray()).toMatchObject([{ kind: "command", state: "failed" }]);
		expect(trigger.getAttribute("aria-busy")).toBe("false");

		accept();
		await click(trigger);
		await click(confirmButton());
		await view.settle();
		expect(stops().length).toBe(2);
		expect(outcomes[1]?.status).toBe("done");
		expect(result()?.getAttribute("data-result")).toBe("good");
	});

	test("no reply: one tray item with the operation handle, and the result says it may have run", async () => {
		const { view, agent, outcomes, trigger, result, confirmButton, tray } =
			await mount();
		agent.dropNext("stop");
		await click(trigger);
		await click(confirmButton());
		await view.settle();

		expect(outcomes).toMatchObject([{ status: "unknown" }]);
		expect(tray()).toMatchObject([
			{
				kind: "command",
				state: "unknown",
				resume: {
					type: "operation",
					operationId: expect.any(String),
					command: "stop",
				},
			},
		]);
		expect(result()?.getAttribute("data-result")).toBe("unknown");
		expect(result()?.textContent).toContain("no reply received");
		expect(trigger.getAttribute("aria-busy")).toBe("false");
	});

	test("a hub failure is reported in plain words and nothing is tracked for an untracked action", async () => {
		const path = `devices/${EDGE}`;
		const { view, api, outcomes, trigger, result } = await mount({
			action: "rename_device",
			consequence: undefined,
			activity: undefined,
			label: "Rename edge-berlin-01",
			call: async (
				context: Hooks.DeviceActionContext,
			): Promise<ManagementResponse> => {
				const { api: hub, profile } = context.workspace.deps;
				await hub.patch(profile, path, { display_name: "Edge" });
				return { operation_id: "", state: "completed", result: {} };
			},
		});
		const tray = view.fake.workspace.activity.list().length;
		api.fail(
			{ method: "PATCH", path },
			new ApiResponseError({ status: 500, message: "boom" }),
		);
		await click(trigger);
		await view.settle();
		expect(outcomes).toMatchObject([{ status: "failed" }]);
		expect(api.sent("PATCH", path)).toEqual([
			["PATCH", path, { display_name: "Edge" }],
		]);
		expect(result()?.textContent).toContain(
			"Rename edge-berlin-01 didn't run.",
		);
		expect(result()?.textContent).toContain("The hub reported an error.");
		expect(view.fake.workspace.activity.list().length).toBe(tray);
	});

	test("a finished action invalidates the queries it names", async () => {
		const { view, api, outcomes, trigger } = await mount((fake) => ({
			consequence: undefined,
			invalidate: [deviceKeys.list(fake.workspace.scopeKey)],
		}));
		const lists = () => api.sent("GET", "devices").length;
		const before = lists();
		await click(trigger);
		await view.settle();
		expect(outcomes[0]?.status).toBe("done");
		expect(lists()).toBe(before + 1);
	});

	test("a tracked action that settles later keeps the tray item waiting and the result in progress", async () => {
		let finish: ((value: "done") => void) | undefined;
		const { view, trigger, result, tray } = await mount({
			consequence: undefined,
			activity: {
				kind: "command",
				serviceId: SERVICE,
				resume: (response) => ({
					type: "operation",
					operationId: response.operation_id,
					command: "stop",
					issuedAt: SAMPLE_NOW,
				}),
				settle: () =>
					new Promise((resolve) => {
						finish = resolve;
					}),
			},
		});
		await click(trigger);
		await view.settle();
		expect(tray()).toMatchObject([
			{ state: "waiting", resume: { type: "operation", command: "stop" } },
		]);
		expect(result()?.getAttribute("data-result")).toBe("info");
		expect(result()?.textContent).toContain("in progress");
		expect(trigger.getAttribute("aria-busy")).toBe("false");

		await act(async () => finish?.("done"));
		await view.settle();
		expect(tray()).toMatchObject([{ state: "done" }]);
		expect(result()?.getAttribute("data-result")).toBe("good");
	});
});
