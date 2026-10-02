import type { SetupStep } from "../../../../lib/device-management/model/types";
import { enumLabel } from "../copy/enum-labels";
import { gateCopy } from "../copy/gate-copy";
import type { DevicesT } from "../primitives/area-context";
import type { NextState } from "./setup-context";
import {
	type PasswordIssue,
	type RepeatIssue,
	type SetupDraft,
	type TargetOption,
	modeAvailable,
	nameIssue,
} from "./setup-state";
import { nameIssueText } from "./steps/name-step";
import { passwordIssueText, repeatIssueText } from "./steps/password-step";
import type { SetupChecks } from "./use-setup-checks";
import type { SetupCreate } from "./use-setup-create";

const OK: NextState = { ok: true };
const busy = (reason: string): NextState => ({
	ok: false,
	reason,
	kind: "busy",
});
const hub = (reason: string): NextState => ({ ok: false, reason, kind: "hub" });
const field = (reason: string): NextState => ({
	ok: false,
	reason,
	field: true,
});

/** Why step 0 can't be left, in one line; undefined when every check passes. */
export function checksReason(
	t: DevicesT,
	checks: SetupChecks,
): NextState | undefined {
	const { readiness, release, gate } = checks;
	if (readiness.state === "checking")
		return busy(t("devices:setup.next.checking", "Checking the hub…"));
	if (readiness.state === "failed" || readiness.error)
		return hub(
			t(
				"devices:setup.next.checksFailed",
				"The hub checks couldn't finish. Check again.",
			),
		);
	if (readiness.refreshing)
		return busy(t("devices:setup.next.checking", "Checking the hub…"));
	const failing = readiness.data.checks.find((check) => !check.ready);
	if (failing)
		return hub(
			t(
				"devices:setup.next.notReady",
				"{{check}} isn't ready. Only the hub operator can fix it.",
				{ check: enumLabel(t, "readinessCheck", failing.id) },
			),
		);
	if (release.state === "missing")
		return hub(
			t(
				"devices:setup.check.noRelease.title",
				"This hub has no signed agent releases.",
			),
		);
	if (release.state === "rejected")
		return hub(
			t(
				"devices:setup.next.rejected",
				"The agent release can't be verified, so no package can be built.",
			),
		);
	if (release.state === "unreachable")
		return hub(
			t(
				"devices:setup.next.releaseUnreachable",
				"The agent release couldn't be loaded. Check again.",
			),
		);
	if (!gate.ok)
		return { ok: false, reason: gateCopy(t, gate).inline, kind: gate.kind };
	return release.state === "verified"
		? undefined
		: busy(t("devices:setup.next.verifying", "Verifying the agent release…"));
}

/** Checks that fail for good (not while they load): only then is step 0 forced. */
export function checksBlocked(checks: SetupChecks): boolean {
	const { readiness, release, gate } = checks;
	if (readiness.state === "failed") return true;
	if (readiness.state !== "loaded" || readiness.refreshing) return false;
	if (!readiness.data.ready) return true;
	if (release.state === "missing" || release.state === "rejected") return true;
	return !gate.ok;
}

export interface NextInput {
	t: DevicesT;
	step: SetupStep;
	draft: SetupDraft;
	checks: SetupChecks;
	option: TargetOption | undefined;
	password: string;
	firstIssue: PasswordIssue | undefined;
	secondIssue: RepeatIssue | undefined;
	create: Pick<SetupCreate, "run" | "canRetry" | "built">;
}

function platformState({ t, option, draft }: NextInput): NextState {
	if (!option?.available)
		return field(
			t(
				"devices:setup.platform.error.target",
				"Pick the platform the device runs on.",
			),
		);
	return modeAvailable(option, draft.mode)
		? OK
		: field(
				t(
					"devices:setup.platform.error.mode",
					"Pick how the device runs the agent.",
				),
			);
}

function passwordState({ t, firstIssue, secondIssue }: NextInput): NextState {
	if (firstIssue) return field(passwordIssueText(t, firstIssue));
	return secondIssue ? field(repeatIssueText(t, secondIssue)) : OK;
}

function createState(input: NextInput): NextState {
	const { t, draft, checks, create, password, firstIssue, secondIssue } = input;
	if (draft.created) return OK;
	if (create.run.status === "running")
		return busy(
			t(
				"devices:setup.next.creating",
				"Creating the package. Keep this window open until it's ready.",
			),
		);
	const retry = create.run.status === "failed" && create.canRetry;
	if (!retry && (!password || firstIssue || secondIssue))
		return {
			ok: false,
			kind: "locked",
			reason: t(
				"devices:setup.next.passwordGone",
				"Enter the device password again first. It's never stored, so it's gone after the page reloaded.",
			),
		};
	return checksReason(t, checks) ?? OK;
}

function saveState({ t, draft, create }: NextInput): NextState {
	if (!create.built)
		return draft.acknowledged
			? OK
			: busy(
					t(
						"devices:setup.next.acknowledge",
						"Confirm that you have the package, or cancel this setup.",
					),
				);
	if (draft.created?.outcome !== "saved" && !draft.keySaved)
		return {
			ok: false,
			kind: "nokeys",
			reason: t(
				"devices:setup.next.keyFirst",
				"Download the key backup file first. Your account has no copy of these keys.",
			),
		};
	return draft.packageSaved
		? OK
		: busy(
				t(
					"devices:setup.next.packageFirst",
					"Download the setup package first. It can't be downloaded again after you leave this setup.",
				),
			);
}

function nameState({ t, draft }: NextInput): NextState {
	const issue = nameIssue(draft.name);
	return issue ? field(nameIssueText(t, issue)) : OK;
}

function waitState({ t, draft }: NextInput): NextState {
	return draft.checkedInAt === undefined
		? busy(
				t("devices:setup.next.opensLater", "Opens once {{name}} checks in.", {
					name: draft.name,
				}),
			)
		: OK;
}

/** Check · Name · Platform · Password · Create · Save · Start · Waiting. */
const BY_STEP: Record<SetupStep, (input: NextInput) => NextState> = {
	0: ({ t, checks }) => checksReason(t, checks) ?? OK,
	1: nameState,
	2: platformState,
	3: passwordState,
	4: createState,
	5: saveState,
	6: () => OK,
	7: waitState,
};

/** Whether the step's primary may run right now (R7: a gated primary sends nothing). */
export function nextState(input: NextInput): NextState {
	return BY_STEP[input.step](input);
}
