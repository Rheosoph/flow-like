import type { DeploymentRolloutStatus } from "../deployment";
import type { Convergence, PlacementStatusPlus } from "./types";

const KNOWN_OBSERVED = new Set([
	"starting",
	"running",
	"stopping",
	"backoff",
	"stopped",
	"failed",
]);
const CRASHED = new Set(["backoff", "failed"]);
const ROLLOUT_RUNNING = new Set<DeploymentRolloutStatus["state"]>([
	"validating",
	"activating",
	"rolling_back",
]);
const ROLLBACK_FAILED = new Set(["rollback_timeout", "rollback_failed"]);

export type ConvergencePlacement = Pick<
	PlacementStatusPlus,
	| "desired_state"
	| "observed_state"
	| "config_revision"
	| "applied_revision"
	| "desired_replicas"
	| "ready_replicas"
	| "restarts"
	| "replicas"
>;

export type ConvergenceRollout = Pick<
	DeploymentRolloutStatus,
	"state" | "failure_code"
>;

/** IA §1.2 E1 "Derived for the redesign". */
export function convergence(
	placement: ConvergencePlacement,
	rollout?: ConvergenceRollout | null,
): Convergence {
	if (rollout && ROLLOUT_RUNNING.has(rollout.state))
		return "update_in_progress";
	const { desired_state: desired, observed_state: observed } = placement;
	if (!KNOWN_OBSERVED.has(observed)) return "unknown";
	if (desired === "stopped") return stoppedState(observed, rollout);
	if (desired !== "running") return "unknown";
	return runningState(placement);
}

const stoppedState = (
	observed: string,
	rollout: ConvergenceRollout | null | undefined,
): Convergence => {
	if (observed !== "stopped") return "converging";
	const rollbackFailed =
		rollout?.state === "failed" &&
		ROLLBACK_FAILED.has(rollout.failure_code ?? "");
	return rollbackFailed ? "failed_stopped" : "stopped_by_user";
};

const crashLooping = (placement: ConvergencePlacement): boolean =>
	CRASHED.has(placement.observed_state) ||
	placement.restarts?.crash_looping === true ||
	(placement.replicas ?? []).some((replica) => replica.restarts?.crash_looping);

const replicasReady = (placement: ConvergencePlacement) => {
	const requested = placement.desired_replicas ?? 1;
	return (placement.ready_replicas ?? requested) >= requested;
};

const runningState = (placement: ConvergencePlacement): Convergence => {
	if (crashLooping(placement)) return "crash_looping";
	const applied =
		placement.observed_state === "running" &&
		placement.applied_revision === placement.config_revision;
	return applied && replicasReady(placement) ? "converged" : "converging";
};
