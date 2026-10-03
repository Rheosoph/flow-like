import { afterAll, afterEach, describe, expect, test } from "bun:test";
import type {
	DeployPlan,
	DeployResult,
	PlanDevice,
} from "../../../../lib/device-management/model/deploy-plan";
import type {
	DeployRunEvent,
	DeployRunState,
} from "../../../../lib/device-management/model/deploy-run";
import { byRole, installDom } from "../testing/dom-harness";

const dom = installDom();
const { cleanupDevices, mountDevices, preloadDevices } = await import(
	"../testing/mount-devices"
);
await preloadDevices();
const { DeploySummary } = await import("./deploy-summary");
const { deploySteps } = await import("./deploy-copy");
const { checkPlan, makePlan, resolvePlan } = await import(
	"../../../../lib/device-management/model/deploy-plan"
);
const { createDeployRun, reduceDeployRun } = await import(
	"../../../../lib/device-management/model/deploy-run"
);
const { NOW0, PLAN_DEVICES, VISITOR_PLAN_APP } = await import(
	"../../../../lib/device-management/model/__fixtures__/apps"
);
const { SAMPLE_IDS } = await import(
	"../../../../lib/device-management/model/__fixtures__/sample-fleet"
);

const EDGE = SAMPLE_IDS.edge;
const STUDIO = SAMPLE_IDS.studio;
const DEVICES: Record<string, PlanDevice> = {
	[EDGE]: { ...PLAN_DEVICES["edge-berlin-01"], id: EDGE },
	[STUDIO]: { ...PLAN_DEVICES["studio-mac-mini"], id: STUDIO },
};
const FACTS = {
	app: VISITOR_PLAN_APP,
	devices: DEVICES,
	platform: "desktop" as const,
	now: NOW0,
};

afterEach(async () => {
	await cleanupDevices();
	await dom.cleanup();
});
afterAll(dom.restore);

const text = (root: ParentNode) =>
	(root as HTMLElement).textContent?.replace(/\s+/g, " ") ?? "";

function planOf(deviceIds: string[]): DeployPlan {
	const draft = makePlan({
		scope: { kind: "app", appId: VISITOR_PLAN_APP.id },
		route: {
			appId: VISITOR_PLAN_APP.id,
			deviceIds,
			eventId: "evt_visitor_page",
		},
		app: VISITOR_PLAN_APP,
		deploymentId: "dep-summary",
		now: NOW0,
	});
	return resolvePlan(draft, FACTS);
}

const target = (deviceId: string) => `${deviceId}/check-in-page`;

/** A run of the plan's devices, one at a time, after the given events. */
function runOf(
	deviceIds: string[],
	events: DeployRunEvent[] = [],
): DeployRunState {
	const idle = createDeployRun({
		id: "run",
		rows: deviceIds.map((deviceId) => ({
			target: target(deviceId),
			deviceId,
			serviceId: "check-in-page",
			phases: ["create", "start"],
		})),
		order: "one",
		stopOnFail: false,
	});
	return [{ type: "start", at: NOW0 } as const, ...events].reduce(
		reduceDeployRun,
		idle,
	);
}

const done = (deviceId: string): DeployRunEvent => ({
	type: "done",
	target: target(deviceId),
	at: NOW0 + 5,
});
const failed = (deviceId: string): DeployRunEvent => ({
	type: "fail",
	target: target(deviceId),
	at: NOW0 + 5,
	error: { phase: "create", code: "invalid" },
});

interface Shown {
	deviceIds: string[];
	run?: DeployRunState | null;
	outcome?: DeployResult["outcome"];
	deployed?: boolean;
}

/** The summary as the frame shows it on Rollout; returns its text. */
async function rolloutSummary(shown: Shown): Promise<string> {
	const plan = planOf(shown.deviceIds);
	const view = await mountDevices(
		<DeploySummary
			plan={plan}
			check={checkPlan(plan, FACTS)}
			steps={deploySteps("online")}
			current="rollout"
			reached={7}
			prefilled={new Set()}
			limitsOnly={false}
			deployed={shown.deployed ?? false}
			run={shown.run ?? null}
			{...(shown.outcome ? { outcome: shown.outcome } : {})}
			goTo={() => {}}
		/>,
	);
	const said = text(byRole("region", "Your choices", view.container));
	await view.unmount();
	return said;
}

describe("This deploy on Rollout (APP §3.4)", () => {
	test("several devices: the board's own count, then how it ended", async () => {
		const both = [EDGE, STUDIO];
		const running = await rolloutSummary({
			deviceIds: both,
			run: runOf(both),
		});
		expect(running).toContain("Rollout0 of 2 done");
		expect(running).toContain(
			"Running on the devices now. Your choices are locked until it finishes.",
		);
		expect(running).not.toContain("After you deploy");

		const partial = await rolloutSummary({
			deviceIds: both,
			run: runOf(both, [failed(EDGE), done(STUDIO)]),
		});
		expect(partial).toContain("RolloutFailed on 1");
		expect(partial).toContain(
			"Finished. Change anything to start a new deploy from the same choices.",
		);

		const all = await rolloutSummary({
			deviceIds: both,
			run: runOf(both, [done(EDGE), done(STUDIO)]),
		});
		expect(all).toContain("RolloutDone");
	});

	test("one device: Running, Done or Failed, never Deployed for a run that failed", async () => {
		const one = [EDGE];
		const running = await rolloutSummary({ deviceIds: one, run: runOf(one) });
		expect(running).toContain("RolloutRunning");
		expect(running).toContain(
			"Applying on edge-berlin-01 now. Your choices are locked until it finishes.",
		);

		const ended = await rolloutSummary({
			deviceIds: one,
			run: runOf(one, [failed(EDGE)]),
			deployed: true,
		});
		expect(ended).toContain("RolloutFailed");
		expect(ended).not.toContain("Deployed");
		expect(ended).toContain(
			"It didn't finish on edge-berlin-01. Your choices are kept for a retry.",
		);

		const good = await rolloutSummary({
			deviceIds: one,
			run: runOf(one, [done(EDGE)]),
			deployed: true,
		});
		expect(good).toContain("RolloutDone");
		expect(good).toMatch(/Finished on edge-berlin-01 at \d/);
	});

	test("without a run in this window: how the last one ended as it was kept, else nothing is claimed", async () => {
		const one = [EDGE];
		expect(
			await rolloutSummary({ deviceIds: one, outcome: "none", deployed: true }),
		).toContain("RolloutFailed");
		expect(
			await rolloutSummary({ deviceIds: one, outcome: "all", deployed: true }),
		).toContain("RolloutDone");
		expect(await rolloutSummary({ deviceIds: one })).toContain(
			"RolloutAfter you deploy",
		);
	});
});
