import { describe, expect, test } from "bun:test";
import {
	type DeploymentEvent,
	type InstalledProject,
	type PlacementConfiguration,
	canCheckDeploymentStartup,
} from "../../../../lib/device-management/deployment";
import {
	CRM_PLAN_APP,
	NOW0,
	PLAN_DEVICES,
	VISITOR_CATALOG,
	VISITOR_PLAN_APP,
} from "../../../../lib/device-management/model/__fixtures__/apps";
import {
	type DeployDraft,
	type PlanApp,
	makePlan,
	resolvePlan,
} from "../../../../lib/device-management/model/deploy-plan";
import {
	SAFE_UPDATE_TIMINGS,
	type UpdateFacts,
	blocksNewUpdate,
	isBehind,
	isFastPath,
	safeUpdateBlockers,
	secretCount,
	startsAfter,
	targetPhases,
	timingIssues,
	updateKind,
	updateStrategy,
	uploadsNothing,
} from "./update-path";

const EDGE = "edge-berlin-01";

function planOf(
	app: PlanApp,
	route: { deviceIds: string[]; serviceId?: string; mode?: "new" | "update" },
	change: Partial<DeployDraft> = {},
) {
	const draft = {
		...makePlan({
			scope: { kind: "app", appId: app.id },
			route: { appId: app.id, ...route },
			app,
			deploymentId: "dep-update-path",
			now: NOW0,
		}),
		...change,
	};
	return resolvePlan(draft, {
		app,
		devices: PLAN_DEVICES,
		platform: "desktop",
		now: NOW0,
	});
}

const hosted = { id: "evt_page", name: "Visitor page", hosted: true };
const background = { id: "evt_nightly", name: "Nightly sync", hosted: false };

function existing(
	change: Partial<NonNullable<UpdateFacts["existing"]>> = {},
	grant: unknown = { grant_id: "g", authz_version: 1 },
): NonNullable<UpdateFacts["existing"]> {
	return {
		desired_state: "running",
		rollout_sources: ["offline", "online"],
		config: { source: "online", resource_grant: grant as never },
		...change,
	};
}

describe("update strategy (APP §3.14)", () => {
	test("auto is a safe update when the service runs, the agent supports its mode, it has cloud access and every event can be checked", () => {
		expect(
			updateStrategy("auto", {
				source: "online",
				existing: existing(),
				events: [hosted, { ...background, rollout_supported: true }],
			}),
		).toEqual({ strategy: "safe", known: true, reasons: [] });
	});

	test("an older agent that names no update modes checks offline copies only", () => {
		const older = existing({ rollout_sources: undefined });
		expect(
			updateStrategy("auto", {
				source: "online",
				existing: older,
				events: [hosted],
			}),
		).toEqual({
			strategy: "quick",
			known: true,
			reasons: [{ code: "agent_unsupported" }],
		});
		expect(
			updateStrategy("auto", {
				source: "offline",
				existing: {
					...older,
					config: { source: "offline", resource_grant: null },
				},
				events: [hosted],
			}).strategy,
		).toBe("safe");
	});

	test("each blocker is named in the device's order", () => {
		const facts: UpdateFacts = {
			source: "online",
			existing: existing({ desired_state: "stopped" }, null),
			events: [hosted, background],
		};
		expect(safeUpdateBlockers(facts)).toEqual([
			{ code: "stopped" },
			{ code: "no_cloud_access" },
			{ code: "event_unchecked", params: { event: "Nightly sync", count: 1 } },
		]);
		expect(
			safeUpdateBlockers({ ...facts, existing: existing(), events: [] }),
		).toEqual([{ code: "no_events" }]);
	});

	test("a chosen quick update and an unread service say so", () => {
		expect(
			updateStrategy("quick", {
				source: "online",
				existing: existing(),
				events: [hosted],
			}),
		).toEqual({
			strategy: "quick",
			known: true,
			reasons: [{ code: "chosen_quick" }],
		});
		expect(
			updateStrategy("auto", { source: "online", events: [hosted] }),
		).toEqual({
			strategy: "quick",
			known: false,
			reasons: [{ code: "not_loaded" }],
		});
	});

	test("agrees with the device rule the wiring uses for every combination", () => {
		const events: DeploymentEvent[] = VISITOR_CATALOG.events;
		for (const desired of ["running", "stopped"] as const)
			for (const sources of [undefined, ["offline"], ["offline", "online"]])
				for (const grant of [null, { grant_id: "g", authz_version: 1 }])
					for (const supported of [true, false]) {
						const list = events.map((event) => ({
							...event,
							rollout_supported: supported,
						}));
						const placement = {
							placement_id: "visitor-checkin",
							project_id: "app_visitor_checkin",
							deployment_id: "dep",
							config_revision: 3,
							desired_state: desired,
							rollout_sources: sources,
							config: { source: "online", resource_grant: grant },
						} as unknown as PlacementConfiguration;
						const installed = {
							project_id: "app_visitor_checkin",
							project_path: "/p",
							source: "online",
						} satisfies InstalledProject;
						expect(
							updateStrategy("auto", {
								source: "online",
								existing: placement,
								events: list,
							}).strategy === "safe",
						).toBe(canCheckDeploymentStartup(installed, placement, list));
					}
	});
});

describe("update phases", () => {
	const update = planOf(CRM_PLAN_APP, {
		deviceIds: [EDGE],
		serviceId: "nightly-sync",
	});
	const [target] = update.targets;
	const [service] = target?.services ?? [];
	if (!target || !service) throw new Error("The fixture plan has no target.");

	test("a safe update stages, checks and switches; nothing is stopped by hand", () => {
		expect(
			targetPhases(update, service, {
				strategy: "safe",
				secrets: true,
				wasRunning: true,
				startStopped: false,
			}),
		).toEqual([
			"upload",
			"install",
			"prepare_update",
			"secrets",
			"check_new",
			"switch",
		]);
	});

	test("a quick update starts again only what ran before, or what you asked to start", () => {
		const quick = { strategy: "quick" as const, secrets: false };
		expect(
			targetPhases(update, service, {
				...quick,
				wasRunning: true,
				startStopped: false,
			}),
		).toEqual(["upload", "install", "stop", "start"]);
		expect(
			targetPhases(update, service, {
				...quick,
				wasRunning: false,
				startStopped: false,
			}),
		).toEqual(["upload", "install", "stop"]);
		expect(
			targetPhases(update, service, {
				...quick,
				wasRunning: false,
				startStopped: true,
			}),
		).toEqual(["upload", "install", "stop", "start"]);
	});

	test("an update that keeps the version uploads and prepares nothing", () => {
		const keep = planOf(
			CRM_PLAN_APP,
			{ deviceIds: [EDGE], serviceId: "nightly-sync" },
			{ version: "keep" },
		);
		const kept = keep.targets[0]?.services[0];
		if (!kept) throw new Error("The fixture plan has no service.");
		expect(uploadsNothing(keep, kept)).toBe(true);
		expect(updateKind(keep)).toBe("settings");
		expect(
			targetPhases(keep, kept, {
				strategy: "safe",
				secrets: false,
				wasRunning: true,
				startStopped: false,
			}),
		).toEqual(["prepare_update", "check_new", "switch"]);
		expect(uploadsNothing(update, service)).toBe(false);
		expect(updateKind(update)).toBe("reupload");
	});

	test("online updates re-pin, new services always upload", () => {
		const fresh = planOf(VISITOR_PLAN_APP, { deviceIds: [EDGE] });
		const created = fresh.targets[0]?.services[0];
		if (!created) throw new Error("The fixture plan has no service.");
		expect(updateKind(fresh)).toBe("repin");
		expect(
			uploadsNothing(
				{ ...fresh, draft: { ...fresh.draft, version: "keep" } },
				created,
			),
		).toBe(false);
	});
});

describe("update helpers", () => {
	test("whether the service runs afterwards", () => {
		const base = {
			strategy: "quick" as const,
			start: true,
			startStopped: false,
		};
		expect(startsAfter({ ...base, kind: "new", wasRunning: false })).toBe(true);
		expect(
			startsAfter({ ...base, kind: "new", wasRunning: false, start: false }),
		).toBe(false);
		expect(
			startsAfter({
				...base,
				kind: "update",
				strategy: "safe",
				wasRunning: true,
			}),
		).toBe(true);
		expect(startsAfter({ ...base, kind: "update", wasRunning: true })).toBe(
			true,
		);
		expect(startsAfter({ ...base, kind: "update", wasRunning: false })).toBe(
			false,
		);
		expect(
			startsAfter({
				...base,
				kind: "add",
				wasRunning: false,
				startStopped: true,
			}),
		).toBe(true);
	});

	test("safe-update timings stay within 2–60 s and 10–600 s", () => {
		expect(timingIssues(SAFE_UPDATE_TIMINGS)).toEqual([]);
		expect(timingIssues({ stabilizeSeconds: 1, deadlineSeconds: 601 })).toEqual(
			["stabilize", "deadline"],
		);
		expect(
			timingIssues({ stabilizeSeconds: 2.5, deadlineSeconds: 600 }),
		).toEqual(["stabilize"]);
	});

	test("the version-only update of several services is the fast path", () => {
		const everywhere = planOf(CRM_PLAN_APP, {
			deviceIds: [EDGE],
			mode: "update",
		});
		expect(isFastPath(everywhere.draft)).toBe(true);
		expect(isFastPath({ ...everywhere.draft, version: "keep" })).toBe(false);
		expect(
			isFastPath(
				planOf(CRM_PLAN_APP, { deviceIds: [EDGE], serviceId: "nightly-sync" })
					.draft,
			),
		).toBe(false);
	});

	test("secrets to save: typed secret values plus a new access token", () => {
		const draftSecrets = { var_host_token: "s3cret" };
		const plan = planOf(
			VISITOR_PLAN_APP,
			{ deviceIds: [EDGE] },
			{ secrets: draftSecrets },
		);
		const [first] = plan.targets;
		const hostedService = first?.services.find((value) =>
			value.events.includes("evt_visitor_page"),
		);
		if (!first || !hostedService)
			throw new Error("The fixture plan has no hosted service.");
		expect(secretCount(plan, first, hostedService)).toBe(2);
		const kept = {
			...plan,
			draft: {
				...plan.draft,
				secrets: {},
				endpoint: { ...plan.draft.endpoint, token: "keep" as const },
			},
		};
		expect(secretCount(kept, first, hostedService)).toBe(0);
	});

	test("a staged or running update blocks a new one; drift is per service", () => {
		for (const state of ["staged", "validating", "activating", "rolling_back"])
			expect(blocksNewUpdate(state)).toBe(true);
		for (const state of ["healthy", "rolled_back", "failed", "cancelled", null])
			expect(blocksNewUpdate(state)).toBe(false);
		expect(isBehind({ appVersion: { hash: "a" } }, "b")).toBe(true);
		expect(isBehind({ appVersion: { hash: "b" } }, "b")).toBe(false);
		expect(isBehind({ appVersion: null }, "b")).toBe(false);
	});
});
