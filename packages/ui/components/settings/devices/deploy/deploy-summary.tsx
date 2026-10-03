"use client";

import { useTranslation } from "@flow-like/locales";
import { ChevronUp, ClipboardList, Lock } from "lucide-react";
import { type ReactNode, useState } from "react";
import { isBotTokenKey } from "../../../../lib/device-management/bot-config";
import type {
	DeployPlan,
	DeployResult,
	PlanCheck,
} from "../../../../lib/device-management/model/deploy-plan";
import {
	type DeployRunState,
	deployRunResult,
} from "../../../../lib/device-management/model/deploy-run";
import type { DeployStepId } from "../../../../lib/device-management/model/types";
import { humanFileSize } from "../../../../lib/utils";
import { formatMoney } from "../copy/attention-copy";
import { type DevicesT, useAreaTime } from "../primitives/area-context";
import { DvSheet } from "../primitives/dv-sheet";
import { fleetRolloutChip } from "../primitives/fleet-rollout";
import { FreshnessStamp } from "../primitives/freshness-stamp";
import { cx } from "../primitives/tone";
import {
	WizardSummary,
	type WizardSummaryItem,
	type WizardSummaryState,
} from "../primitives/wizard";
import { stepTitle } from "./deploy-copy";
import { eventVariables } from "./deploy-facts";
import type { DeployPrepared } from "./step-props";

/* "This deploy" (APP §3.4): one line per step, said the way the step's choices read when done. */

export interface DeploySummaryInput {
	plan: DeployPlan;
	check: PlanCheck;
	steps: readonly DeployStepId[];
	current: DeployStepId;
	/** Index of the furthest step visited. */
	reached: number;
	/** Steps the entry filled in. */
	prefilled: ReadonlySet<DeployStepId>;
	limitsOnly: boolean;
	versionLabel?: string;
	prepared?: DeployPrepared | null;
	deployed: boolean;
	/** This plan's run once it started in this window (or was picked up after a reload). */
	run?: DeployRunState | null;
	/** How the plan's last run ended, kept with the saved progress. */
	outcome?: DeployResult["outcome"];
	goTo(step: DeployStepId): void;
}

interface ValueContext extends DeploySummaryInput {
	t: DevicesT;
	locale: string;
}

const Muted = ({ children }: Readonly<{ children: ReactNode }>) => (
	<span className="text-muted-foreground">{children}</span>
);
const Mono = ({ children }: Readonly<{ children: ReactNode }>) => (
	<span className="font-mono">{children}</span>
);

const joined = (parts: readonly (string | false | undefined)[]) =>
	parts.filter(Boolean).join(" · ");

function versionText({ t, plan, versionLabel }: ValueContext): string {
	if (plan.draft.entry === "update" && plan.draft.version === "keep")
		return t("devices:deploy.summary.keepsVersion", "keeps each version");
	return versionLabel ?? t("devices:deploy.summary.newest", "newest version");
}

function whatValue(c: ValueContext): ReactNode {
	const { t, plan } = c;
	if (!plan.app)
		return (
			<Muted>{t("devices:deploy.summary.nothing", "Nothing chosen yet")}</Muted>
		);
	const events = new Set(plan.services.flatMap((service) => service.events));
	const services = (count: number) =>
		t("devices:deploy.summary.services", {
			count,
			defaultValue_one: "{{count, number}} service",
			defaultValue_other: "{{count, number}} services",
		});
	const counts =
		plan.draft.entry === "update"
			? [services(plan.targets.length)]
			: [
					t("devices:deploy.summary.events", {
						count: events.size,
						defaultValue_one: "{{count, number}} event",
						defaultValue_other: "{{count, number}} events",
					}),
					services(plan.services.length),
				];
	return joined([plan.app.name, versionText(c), ...counts]);
}

function howValue({ t, plan, check }: ValueContext): ReactNode {
	if (!plan.app)
		return (
			<Muted>{t("devices:deploy.summary.afterApp", "After the app")}</Muted>
		);
	if (check.issues.some((issue) => issue.code === "local_only_web"))
		return (
			<span className="text-critical">
				{t("devices:deploy.summary.needsDesktop", "Needs the desktop app")}
			</span>
		);
	return plan.mode === "offline"
		? t("devices:deploy.summary.offline", "Offline copy · data on the device")
		: t("devices:deploy.summary.online", "Runs online · data in the cloud");
}

function whereValue({ t, plan }: ValueContext): ReactNode {
	if (!plan.targets.length)
		return (
			<Muted>{t("devices:deploy.summary.noDevices", "No devices yet")}</Muted>
		);
	return plan.targets.map((target, index) => (
		<span key={target.deviceId}>
			{index ? ", " : ""}
			<Mono>{target.name}</Mono>
			{target.locked ? (
				<Lock
					aria-label={t("devices:deploy.summary.locked", "locked")}
					className="ml-1 inline size-3 text-locked"
				/>
			) : null}
		</span>
	));
}

function settingsCounts(plan: DeployPlan) {
	const variables = eventVariables(
		plan.app,
		plan.services.flatMap((service) => service.events),
	);
	const { draft } = plan;
	const perDevice = (id: string, secret: boolean) =>
		draft.targets.some(
			(target) => (secret ? target.over.secrets : target.over.vars)?.[id],
		);
	let values = 0;
	let differ = 0;
	let secrets = 0;
	// A bot's token has no app default: it counts once it is set, and never as a default.
	let tokensUnset = 0;
	for (const variable of variables) {
		const own = perDevice(variable.id, variable.secret);
		if (variable.secret) {
			if (draft.secrets[variable.id] || own) secrets += 1;
			else if (isBotTokenKey(variable.id)) tokensUnset += 1;
			continue;
		}
		if (draft.vars[variable.id] !== undefined || own) values += 1;
		if (own) differ += 1;
	}
	return {
		all: variables.length - tokensUnset,
		values,
		differ,
		secrets,
		tokensUnset,
	};
}

function settingsValue({ t, plan }: ValueContext): ReactNode {
	if (!plan.app)
		return (
			<Muted>{t("devices:deploy.summary.afterApp", "After the app")}</Muted>
		);
	const { all, values, differ, secrets, tokensUnset } = settingsCounts(plan);
	if (!all && !tokensUnset)
		return (
			<Muted>
				{plan.app.variables
					? t("devices:deploy.summary.noSettings", "This app has no settings")
					: t(
							"devices:deploy.summary.settingsUnknown",
							"Known once the version is prepared",
						)}
			</Muted>
		);
	const defaults = all - values - secrets;
	return joined([
		tokensUnset > 0 &&
			t("devices:deploy.summary.botTokens", {
				count: tokensUnset,
				defaultValue_one: "{{count, number}} bot token to set",
				defaultValue_other: "{{count, number}} bot tokens to set",
			}),
		values > 0 &&
			t("devices:deploy.summary.values", {
				count: values,
				defaultValue_one: "{{count, number}} value",
				defaultValue_other: "{{count, number}} values",
			}),
		differ > 0 &&
			t("devices:deploy.summary.differ", {
				count: differ,
				defaultValue_one: "{{count, number}} differs by device",
				defaultValue_other: "{{count, number}} differ by device",
			}),
		secrets > 0 &&
			t("devices:deploy.summary.secrets", {
				count: secrets,
				defaultValue_one: "{{count, number}} secret",
				defaultValue_other: "{{count, number}} secrets",
			}),
		defaults > 0 &&
			t("devices:deploy.summary.defaults", {
				count: defaults,
				defaultValue_one: "{{count, number}} app default",
				defaultValue_other: "{{count, number}} app defaults",
			}),
	]);
}

function isolationText({ t, plan }: ValueContext): string {
	const { targets, draft } = plan;
	if (!draft.isolation)
		return t(
			"devices:deploy.summary.keepsLimits",
			"keeps each service's limits",
		);
	if (!targets.length)
		return draft.isolation.profile === "trusted_process"
			? t("devices:deploy.summary.asAgent", "runs as the agent")
			: t("devices:deploy.summary.sandboxWhere", "sandboxed where possible");
	return t(
		"devices:deploy.summary.sandboxedOn",
		"sandboxed on {{count, number}} of {{total, number}}",
		{
			count: targets.filter((target) => !target.runsAsAgent).length,
			total: targets.length,
		},
	);
}

function endpointValue(c: ValueContext): ReactNode {
	const { t, plan } = c;
	if (!plan.app)
		return (
			<Muted>{t("devices:deploy.summary.afterApp", "After the app")}</Muted>
		);
	const instances = t("devices:deploy.summary.instances", {
		count: Math.max(1, ...plan.services.map((row) => row.maxInstances)),
		defaultValue_one: "{{count, number}} instance",
		defaultValue_other: "{{count, number}} instances",
	});
	const { host, port } = plan.draft.endpoint;
	const address = c.limitsOnly ? (
		t("devices:deploy.summary.noEndpoint", "No endpoint")
	) : host === null || port === null ? (
		t("devices:deploy.summary.keepsAddress", "Keeps each address")
	) : (
		<Mono>
			{host}:{port}
		</Mono>
	);
	return (
		<>
			{address} · {instances} · {isolationText(c)}
		</>
	);
}

/** Targets that get a new service; an update entry creates none, even while its services are still unread. */
function creates(plan: DeployPlan): number {
	if (plan.draft.entry === "update") return 0;
	return plan.targets.filter((target) =>
		target.services.some((service) => service.kind === "new"),
	).length;
}

function accessValue({ t, plan, locale }: ValueContext): ReactNode {
	if (!plan.app)
		return (
			<Muted>{t("devices:deploy.summary.afterApp", "After the app")}</Muted>
		);
	if (!plan.targets.length)
		return (
			<Muted>
				{t("devices:deploy.summary.afterDevices", "After the devices")}
			</Muted>
		);
	const created = creates(plan);
	const kept = plan.targets.length - created;
	const { spending } = plan.draft;
	return joined([
		created > 0 &&
			t("devices:deploy.summary.approvals", {
				count: created,
				defaultValue_one: "{{count, number}} approval",
				defaultValue_other: "{{count, number}} approvals",
			}),
		created > 0 &&
			spending !== null &&
			t("devices:deploy.summary.spendEach", "{{amount}} each", {
				amount: formatMoney(spending.limitMicros, locale),
			}),
		kept > 0 &&
			t("devices:deploy.summary.kept", "{{count, number}} kept", {
				count: kept,
			}),
		!!plan.draft.writes &&
			t("devices:deploy.summary.writesOn", "write buffering on"),
	]);
}

function copyValue({ t, plan, prepared }: ValueContext): ReactNode {
	if (!plan.app)
		return (
			<Muted>{t("devices:deploy.summary.afterApp", "After the app")}</Muted>
		);
	if (!plan.targets.length)
		return (
			<Muted>
				{t("devices:deploy.summary.afterDevices", "After the devices")}
			</Muted>
		);
	if (plan.draft.version === "keep")
		return t("devices:deploy.summary.nothingToSend", "Nothing to send");
	const count = plan.targets.length;
	return prepared
		? t("devices:deploy.summary.sizeToDevices", {
				count,
				size: humanFileSize(prepared.artifact.descriptor.total_bytes),
				defaultValue_one: "{{size}} to {{count, number}} device",
				defaultValue_other: "{{size}} to {{count, number}} devices",
			})
		: t("devices:deploy.summary.copyToDevices", {
				count,
				defaultValue_one: "The copy goes to {{count, number}} device",
				defaultValue_other: "The copy goes to {{count, number}} devices",
			});
}

function reviewValue({ t, plan }: ValueContext): ReactNode {
	const { draft } = plan;
	const how =
		draft.entry === "update"
			? draft.strategy === "quick"
				? t("devices:deploy.summary.quick", "quick update")
				: t("devices:deploy.summary.safe", "safe update where possible")
			: draft.start
				? t("devices:deploy.summary.start", "start after deploy")
				: t("devices:deploy.summary.stopped", "stays stopped");
	if (plan.targets.length < 2)
		return how.charAt(0).toUpperCase() + how.slice(1);
	const order = {
		one: t("devices:deploy.summary.orderOne", "One device at a time"),
		all: t("devices:deploy.summary.orderAll", "All at once"),
		first: t(
			"devices:deploy.summary.orderFirst",
			"First device, then the rest",
		),
	}[draft.order];
	return `${order} · ${how}`;
}

/** The run of this plan, once there is one to speak of. */
const startedRun = (run: DeployRunState | null | undefined) =>
	run && run.status !== "idle" ? run : null;

const ENDED: Record<DeployResult["outcome"], (t: DevicesT) => string> = {
	all: (t) => t("devices:deploy.summary.runDone", "Done"),
	partial: (t) => t("devices:deploy.summary.runFailed", "Failed"),
	none: (t) => t("devices:deploy.summary.runFailed", "Failed"),
};

/** One device: Running, Done or Failed; several: the board's own count ("1 of 2 done", "Failed on 1"). */
function runValue(t: DevicesT, run: DeployRunState): string {
	const result = deployRunResult(run);
	if (result?.outcome === "all") return ENDED.all(t);
	if (run.rows.length > 1)
		return fleetRolloutChip(
			t,
			run.rows.map((row) => ({
				state:
					row.state === "failed" && row.error?.rolledBack
						? ("rolled_back" as const)
						: row.state,
			})),
		).text;
	return result
		? ENDED[result.outcome](t)
		: t("devices:deploy.summary.runRunning", "Running");
}

function rolloutValue({ t, deployed, run, outcome }: ValueContext): ReactNode {
	const started = startedRun(run);
	if (started) return runValue(t, started);
	// No run in this window (a reload): how the last one ended, as far as it was kept.
	if (outcome) return ENDED[outcome](t);
	return deployed ? (
		t("devices:deploy.summary.finished", "Finished")
	) : (
		<Muted>{t("devices:deploy.summary.afterDeploy", "After you deploy")}</Muted>
	);
}

const VALUES: Record<DeployStepId, (c: ValueContext) => ReactNode> = {
	what: whatValue,
	how: howValue,
	where: whereValue,
	settings: settingsValue,
	endpoint: endpointValue,
	access_cost: accessValue,
	copy_upload: copyValue,
	review: reviewValue,
	rollout: rolloutValue,
};

function errorsAt(check: PlanCheck, step: DeployStepId): number {
	return check.issues.filter(
		(issue) => issue.step === step && issue.severity === "error",
	).length;
}

function itemState(
	input: DeploySummaryInput,
	step: DeployStepId,
	index: number,
): WizardSummaryState {
	if (step === input.current) return "current";
	if (input.current === "rollout") return "done";
	if (step === "rollout") return input.deployed ? "done" : "todo";
	const visited = index <= input.reached || input.prefilled.has(step);
	if (!visited) return "todo";
	return index < input.reached && errorsAt(input.check, step) ? "err" : "done";
}

/** Steps you may jump to: visited or prefilled ones, never Rollout, none while a run is shown. */
export function canVisit(
	input: Pick<
		DeploySummaryInput,
		"steps" | "current" | "reached" | "prefilled"
	>,
	step: DeployStepId,
): boolean {
	if (input.current === "rollout" || step === "rollout") return false;
	if (step === input.current) return false;
	const index = input.steps.indexOf(step);
	const here = input.steps.indexOf(input.current);
	return index <= Math.max(input.reached, here) || input.prefilled.has(step);
}

function summaryItems(context: ValueContext): WizardSummaryItem[] {
	return context.steps.map((step, index) => {
		const state = itemState(context, step, index);
		return {
			id: step,
			label: stepTitle(context.t, step, context.limitsOnly),
			value: VALUES[step](context),
			state,
			...(state === "err" ? { errors: errorsAt(context.check, step) } : {}),
			...(canVisit(context, step)
				? { onSelect: () => context.goTo(step) }
				: {}),
		};
	});
}

function footText(t: DevicesT, plan: DeployPlan): string {
	const [first] = plan.targets;
	if (plan.targets.length > 1)
		return t(
			"devices:deploy.summary.footMany",
			"Nothing changes on any device until you deploy.",
		);
	return first
		? t(
				"devices:deploy.summary.footOne",
				"Nothing changes on {{device}} until you deploy.",
				{ device: first.name },
			)
		: t(
				"devices:deploy.summary.footNone",
				"Nothing is sent anywhere until you deploy.",
			);
}

/** On Rollout the note says where the run stands instead of what deploying will do. */
function runFootText(
	t: DevicesT,
	time: ReturnType<typeof useAreaTime>,
	input: DeploySummaryInput,
): string | undefined {
	const run = startedRun(input.run);
	if (!run) return undefined;
	const result = deployRunResult(run);
	const [only] = input.plan.targets;
	if (run.rows.length > 1 || !only)
		return result
			? t(
					"devices:deploy.summary.footFinished",
					"Finished. Change anything to start a new deploy from the same choices.",
				)
			: t(
					"devices:deploy.summary.footRunning",
					"Running on the devices now. Your choices are locked until it finishes.",
				);
	const device = only.name;
	if (!result)
		return t(
			"devices:deploy.summary.footRunningOne",
			"Applying on {{device}} now. Your choices are locked until it finishes.",
			{ device },
		);
	return result.outcome === "all"
		? t(
				"devices:deploy.summary.footDoneOne",
				"Finished on {{device}} at {{time}}.",
				{
					device,
					time: time.clock(result.at),
				},
			)
		: t(
				"devices:deploy.summary.footFailedOne",
				"It didn't finish on {{device}}. Your choices are kept for a retry.",
				{ device },
			);
}

/** The side summary with its done lines; reachable steps are buttons. */
export function DeploySummary({
	className,
	inSheet = false,
	...input
}: Readonly<
	DeploySummaryInput & {
		className?: string;
		/** The sheet names it already, so only the stamp heads the list. */
		inSheet?: boolean;
	}
>) {
	const { t } = useTranslation("devices");
	const time = useAreaTime();
	const { locale } = time;
	const title = t("devices:deploy.summary.title", "This deploy");
	return (
		<WizardSummary
			className={className}
			items={summaryItems({ ...input, t, locale })}
			title={
				<>
					{inSheet ? null : (
						<span className="flex items-center gap-2">
							<ClipboardList aria-hidden className="size-4 text-ink-2" />
							{title}
						</span>
					)}
					<FreshnessStamp
						source="local"
						age="current"
						text={t("devices:deploy.summary.keptHere", "kept in this window")}
						className={inSheet ? "flex font-normal" : "mt-1 flex font-normal"}
					/>
				</>
			}
			foot={
				input.current === "rollout"
					? runFootText(t, time, input)
					: footText(t, input.plan)
			}
		/>
	);
}

/** Below 960 px of container width: one sticky line that opens the summary as a sheet. */
export function DeploySummaryBar({
	className,
	...input
}: Readonly<DeploySummaryInput & { className?: string }>) {
	const { t } = useTranslation("devices");
	const [open, setOpen] = useState(false);
	const count = input.plan.targets.length;
	const goTo = (step: DeployStepId) => {
		setOpen(false);
		input.goTo(step);
	};
	return (
		<>
			<button
				type="button"
				aria-haspopup="dialog"
				data-deploy-summary-bar=""
				onClick={() => setOpen(true)}
				className={cx(
					"w-full items-center gap-2 rounded-lg border border-border bg-card px-3 py-2 text-left text-ui hover:bg-row-hover focus-visible:outline-2 focus-visible:outline-offset-2 focus-visible:outline-ring",
					className,
				)}
			>
				<ClipboardList aria-hidden className="size-4 shrink-0 text-ink-2" />
				<span className="min-w-0 flex-1 truncate">
					<b className="font-semibold">
						{t("devices:deploy.summary.title", "This deploy")}
					</b>
					{t(
						"devices:deploy.summary.barStep",
						" · Step {{n, number}} of {{total, number}} · ",
						{
							n: input.steps.indexOf(input.current) + 1,
							total: input.steps.length,
						},
					)}
					{count
						? t("devices:deploy.summary.barDevices", {
								count,
								defaultValue_one: "{{count, number}} device",
								defaultValue_other: "{{count, number}} devices",
							})
						: t("devices:deploy.summary.barNoDevices", "no devices yet")}
				</span>
				<ChevronUp
					aria-hidden
					className="size-4 shrink-0 text-muted-foreground"
				/>
			</button>
			<DvSheet
				open={open}
				onOpenChange={setOpen}
				icon={ClipboardList}
				title={t("devices:deploy.summary.title", "This deploy")}
				bodyClassName="p-0"
			>
				<DeploySummary
					{...input}
					goTo={goTo}
					inSheet
					className="rounded-none border-0"
				/>
			</DvSheet>
		</>
	);
}
