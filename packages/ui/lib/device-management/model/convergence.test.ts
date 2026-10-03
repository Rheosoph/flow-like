import { expect, test } from "bun:test";
import {
	type ConvergencePlacement,
	type ConvergenceRollout,
	convergence,
} from "./convergence";
import type { Convergence } from "./types";

const placement = (
	patch: Partial<ConvergencePlacement> = {},
): ConvergencePlacement => ({
	desired_state: "running",
	observed_state: "running",
	config_revision: 4,
	applied_revision: 4,
	desired_replicas: 2,
	ready_replicas: 2,
	...patch,
});

test.each<[string, Partial<ConvergencePlacement>, Convergence]>([
	["running as requested", {}, "converged"],
	[
		"single instance without replica counts",
		{ desired_replicas: undefined, ready_replicas: undefined },
		"converged",
	],
	["settings not applied yet", { applied_revision: 3 }, "converging"],
	["settings pending", { applied_revision: null }, "converging"],
	["fewer instances ready", { ready_replicas: 1 }, "converging"],
	["starting", { observed_state: "starting" }, "converging"],
	[
		"stopped while requested running",
		{ observed_state: "stopped" },
		"converging",
	],
	["backoff", { observed_state: "backoff" }, "crash_looping"],
	["failed", { observed_state: "failed" }, "crash_looping"],
	[
		"agent reports crash looping (BG7)",
		{
			observed_state: "starting",
			restarts: {
				failures: 3,
				max_restarts: 5,
				crash_looping: true,
				retry_in_seconds: 40,
				last_started_at: null,
			},
		},
		"crash_looping",
	],
	[
		"stopped by the user",
		{ desired_state: "stopped", observed_state: "stopped" },
		"stopped_by_user",
	],
	[
		"stopping on request",
		{ desired_state: "stopped", observed_state: "stopping" },
		"converging",
	],
	["unknown observed state", { observed_state: "unknown" }, "unknown"],
	["unrecognised observed state", { observed_state: "removed" }, "unknown"],
])("%s", (_, patch, expected) => {
	expect(convergence(placement(patch))).toBe(expected);
});

test.each<[ConvergenceRollout["state"], Convergence]>([
	["validating", "update_in_progress"],
	["activating", "update_in_progress"],
	["rolling_back", "update_in_progress"],
	["staged", "converged"],
	["healthy", "converged"],
	["rolled_back", "converged"],
])("rollout %s", (state, expected) => {
	expect(convergence(placement(), { state })).toBe(expected);
});

test("a failed rollback leaves the service stopped after a failed update", () => {
	const stopped = placement({
		desired_state: "stopped",
		observed_state: "stopped",
	});
	expect(
		convergence(stopped, { state: "failed", failure_code: "rollback_failed" }),
	).toBe("failed_stopped");
	expect(
		convergence(stopped, { state: "failed", failure_code: "rollback_timeout" }),
	).toBe("failed_stopped");
	expect(
		convergence(stopped, {
			state: "failed",
			failure_code: "validation_failed",
		}),
	).toBe("stopped_by_user");
});
