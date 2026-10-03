import {
	Box,
	CircleCheck,
	History,
	type LucideIcon,
	OctagonX,
	Power,
	Rocket,
	RotateCcw,
	Terminal,
	Users,
} from "lucide-react";
import type { ReactNode } from "react";
import type { AgentOperation } from "../../../../lib/device-management/agent-reads";
import type { DeploymentRolloutStatus } from "../../../../lib/device-management/deployment";
import { enumLabel } from "../copy/enum-labels";
import type { DevicesT } from "../primitives/area-context";
import { IdRef } from "../primitives/id-ref";
import type { TimelineEntry, TimelineTone } from "../primitives/timeline";
import type {
	CommandState,
	HistoryPauseReason,
	InstanceState,
	TimelineFact,
} from "./timeline-model";
import type { ObserveTarget, PersonNames } from "./use-observe-target";

/*
 * The sentences of the timeline (SPEC §4.29). Every key carries the
 * `devices:` prefix: these are free functions, so the extractor can't tell the
 * namespace from a hook.
 */

type Fact<T extends TimelineFact["type"]> = Extract<TimelineFact, { type: T }>;

/** What a sentence needs besides its fact. */
export interface Sentences {
	t: DevicesT;
	target: ObserveTarget;
	/** A service's name as it appears inside a sentence (a link on the device page). */
	service(id: string): ReactNode;
	/** Commands sent from this computer, by command ID: "Stop support-bot". */
	titles: ReadonlyMap<string, string>;
	/** Who sent which command (BG13); undefined when the device doesn't say. */
	operations: ReadonlyMap<string, AgentOperation> | undefined;
	people: PersonNames;
}

const INSTANCE_TONE: Partial<Record<InstanceState, TimelineTone>> = {
	running: "good",
	backoff: "critical",
	failed: "critical",
};

const COMMAND_TONE: Record<CommandState, TimelineTone> = {
	accepted: "info",
	completed: "good",
	failed: "critical",
	unknown: "warning",
};

const UPDATE_LOOK: Record<
	DeploymentRolloutStatus["state"],
	{ icon: LucideIcon; tone?: TimelineTone }
> = {
	staged: { icon: Rocket, tone: "info" },
	validating: { icon: Rocket, tone: "info" },
	activating: { icon: Rocket, tone: "info" },
	healthy: { icon: CircleCheck, tone: "good" },
	rolling_back: { icon: RotateCcw, tone: "warning" },
	rolled_back: { icon: RotateCcw, tone: "warning" },
	failed: { icon: OctagonX, tone: "critical" },
	cancelled: { icon: Rocket },
};

/** Where a sentence names the service: translators keep the mark, the name is put in its place. */
const SERVICE_MARK = "<1/>";

/**
 * Puts the service's name (a link on the device page) into a translated
 * sentence. The keys stay literal `t()` calls so the extractor finds them.
 */
function withService(
	s: Sentences,
	sentence: string,
	service: string,
): ReactNode {
	const [before, ...rest] = sentence.split(SERVICE_MARK);
	return (
		<>
			{before}
			{s.service(service)}
			{rest.join(SERVICE_MARK)}
		</>
	);
}

function instanceSentence(t: DevicesT, fact: Fact<"instance">): string {
	const values = { slot: fact.slot, settings: fact.settings ?? 0 };
	if (fact.state === "starting" && fact.settings !== undefined)
		return t(
			"devices:observe.timeline.instance.startingWith",
			"Instance #{{slot, number}} of <1/> is starting with settings v{{settings, number}}.",
			values,
		);
	const texts: Record<InstanceState, string> = {
		starting: t(
			"devices:observe.timeline.instance.starting",
			"Instance #{{slot, number}} of <1/> is starting.",
			values,
		),
		running: t(
			"devices:observe.timeline.instance.running",
			"Instance #{{slot, number}} of <1/> is running.",
			values,
		),
		stopping: t(
			"devices:observe.timeline.instance.stopping",
			"Instance #{{slot, number}} of <1/> is stopping.",
			values,
		),
		stopped: t(
			"devices:observe.timeline.instance.stopped",
			"Instance #{{slot, number}} of <1/> stopped.",
			values,
		),
		backoff: t(
			"devices:observe.timeline.instance.backoff",
			"Instance #{{slot, number}} of <1/> crashed and is waiting before it restarts.",
			values,
		),
		failed: t(
			"devices:observe.timeline.instance.failed",
			"Instance #{{slot, number}} of <1/> crashed and wasn't restarted.",
			values,
		),
		removed: t(
			"devices:observe.timeline.instance.removed",
			"Instance #{{slot, number}} of <1/> was removed.",
			values,
		),
	};
	return texts[fact.state];
}

function serviceCommand(t: DevicesT, state: CommandState): string {
	const texts: Record<CommandState, string> = {
		accepted: t(
			"devices:observe.timeline.command.acceptedFor",
			"The device recorded a command for <1/>.",
		),
		completed: t(
			"devices:observe.timeline.command.doneFor",
			"A command for <1/> finished.",
		),
		failed: t(
			"devices:observe.timeline.command.failedFor",
			"A command for <1/> failed.",
		),
		unknown: t(
			"devices:observe.timeline.command.unknownFor",
			"The result of a command for <1/> isn't known.",
		),
	};
	return texts[state];
}

function updateSentence(
	t: DevicesT,
	state: DeploymentRolloutStatus["state"],
): string {
	const texts: Record<DeploymentRolloutStatus["state"], string> = {
		staged: t(
			"devices:observe.timeline.update.staged",
			"Safe update staged for <1/>. The current version keeps running until it is activated.",
		),
		validating: t(
			"devices:observe.timeline.update.validating",
			"Safe update of <1/>: checking the new version.",
		),
		activating: t(
			"devices:observe.timeline.update.activating",
			"Safe update of <1/>: switching over.",
		),
		healthy: t(
			"devices:observe.timeline.update.healthy",
			"Safe update finished: <1/> runs the new version.",
		),
		rolling_back: t(
			"devices:observe.timeline.update.rollingBack",
			"Update of <1/> is rolling back to the previous version.",
		),
		rolled_back: t(
			"devices:observe.timeline.update.rolledBack",
			"Update of <1/> rolled back. The previous version is running again.",
		),
		failed: t(
			"devices:observe.timeline.update.failed",
			"Update of <1/> failed.",
		),
		cancelled: t(
			"devices:observe.timeline.update.cancelled",
			"Update of <1/> was discarded.",
		),
	};
	return texts[state];
}

function namedCommand(t: DevicesT, command: string, state: CommandState) {
	const texts: Record<CommandState, string> = {
		accepted: t(
			"devices:observe.timeline.command.namedAccepted",
			"{{command}}: the device recorded it.",
			{ command },
		),
		completed: t(
			"devices:observe.timeline.command.namedDone",
			"{{command}}: done.",
			{ command },
		),
		failed: t(
			"devices:observe.timeline.command.namedFailed",
			"{{command}}: failed.",
			{ command },
		),
		unknown: t(
			"devices:observe.timeline.command.namedUnknown",
			"{{command}}: the result isn't known.",
			{ command },
		),
	};
	return texts[state];
}

function plainCommand(t: DevicesT, state: CommandState) {
	const texts: Record<CommandState, string> = {
		accepted: t(
			"devices:observe.timeline.command.accepted",
			"The device recorded a command.",
		),
		completed: t(
			"devices:observe.timeline.command.done",
			"A command finished.",
		),
		failed: t("devices:observe.timeline.command.failed", "A command failed."),
		unknown: t(
			"devices:observe.timeline.command.unknown",
			"A command's result isn't known.",
		),
	};
	return texts[state];
}

function commandText(s: Sentences, fact: Fact<"command">): ReactNode {
	const title = s.titles.get(fact.operationId);
	if (title) return namedCommand(s.t, title, fact.state);
	return fact.service
		? withService(s, serviceCommand(s.t, fact.state), fact.service)
		: plainCommand(s.t, fact.state);
}

/** Why recording paused, as the end of a sentence ("… because the readers list expired"). */
export function pauseReason(
	t: DevicesT,
	reason: HistoryPauseReason | null,
): string {
	const reasons: Record<HistoryPauseReason, string> = {
		roster_expired: t(
			"devices:observe.history.reason.rosterExpired",
			"the readers list expired",
		),
		rules_changed: t(
			"devices:observe.history.reason.rulesChanged",
			"the access rules changed",
		),
		rules_expired: t(
			"devices:observe.history.reason.rulesExpired",
			"the access rules expired",
		),
		quota_reached: t(
			"devices:observe.history.reason.quotaReached",
			"the history storage of your plan is full",
		),
		tier_without_history: t(
			"devices:observe.history.reason.tierWithoutHistory",
			"your plan doesn't store history",
		),
		outbox_full: t(
			"devices:observe.history.reason.outboxFull",
			"uploads to the hub aren't getting through",
		),
	};
	return reason
		? reasons[reason]
		: t("devices:observe.history.reason.unknown", "the device didn't say why");
}

function historyText(s: Sentences, fact: Fact<"history_paused">): ReactNode {
	const { t } = s;
	const values = {
		kind: enumLabel(t, "archiveKind", fact.kind).toLowerCase(),
		reason: pauseReason(t, fact.reason),
	};
	if (fact.scope === "device")
		return t(
			"devices:observe.timeline.history.device",
			"Retained {{kind}} for the whole device stopped recording: {{reason}}.",
			values,
		);
	return withService(
		s,
		t(
			"devices:observe.timeline.history.service",
			"Retained {{kind}} for <1/> stopped recording: {{reason}}.",
			values,
		),
		fact.scope,
	);
}

function otherActor(s: Sentences, actor: AgentOperation["actor"]): string {
	const { t } = s;
	const name = actor.user_id ? s.people(actor.user_id) : undefined;
	if (name)
		return t("devices:observe.timeline.byPerson", "by {{name}}", { name });
	return actor.role === "owner"
		? t("devices:observe.timeline.byOwner", "by the owner")
		: t("devices:observe.timeline.byShared", "by someone with shared access");
}

/** "by you", "by Mira Novak": from this computer's own commands, else from the device's record (BG13). */
function actorText(s: Sentences, fact: Fact<"command">): string | undefined {
	const byYou = s.t("devices:observe.timeline.byYou", "by you");
	if (s.titles.has(fact.operationId)) return byYou;
	const actor = s.operations?.get(fact.operationId)?.actor;
	if (!actor) return undefined;
	const mine =
		actor.user_id === s.target.me || (actor.role === "owner" && s.target.owner);
	return mine ? byYou : otherActor(s, actor);
}

function Meta({ parts }: Readonly<{ parts: readonly ReactNode[] }>) {
	const shown = parts.filter((part) => part !== undefined && part !== null);
	if (!shown.length) return null;
	return (
		<span className="inline-flex flex-wrap items-center gap-x-1.5 gap-y-0.5">
			{shown.map((part, index) => (
				// biome-ignore lint/suspicious/noArrayIndexKey: parts are positional
				<span key={index} className="inline-flex items-center gap-1.5">
					{part}
					{index < shown.length - 1 ? <span aria-hidden>·</span> : null}
				</span>
			))}
		</span>
	);
}

type Body = Omit<TimelineEntry, "id" | "at">;

function instanceEntry(s: Sentences, fact: Fact<"instance">): Body {
	const { t } = s;
	const crashed = fact.state === "backoff" || fact.state === "failed";
	const settings =
		fact.settings !== undefined && fact.state !== "starting"
			? t(
					"devices:observe.timeline.settings",
					"settings v{{version, number}}",
					{ version: fact.settings },
				)
			: undefined;
	return {
		kind: "instance",
		icon: crashed ? OctagonX : Box,
		tone: INSTANCE_TONE[fact.state],
		text: withService(s, instanceSentence(t, fact), fact.service),
		meta: (
			<Meta
				parts={[
					fact.process === undefined
						? undefined
						: t("devices:observe.timeline.process", "process {{id}}", {
								id: String(fact.process),
							}),
					settings,
				]}
			/>
		),
	};
}

function commandEntry(s: Sentences, fact: Fact<"command">): Body {
	const { t } = s;
	return {
		kind: "command",
		icon: Terminal,
		tone: COMMAND_TONE[fact.state],
		text: commandText(s, fact),
		meta: (
			<Meta
				parts={[
					actorText(s, fact),
					<IdRef
						key="id"
						id={fact.operationId}
						label={t("devices:observe.timeline.command.id", "command")}
						copyLabel={t(
							"devices:observe.timeline.command.copyId",
							"Copy command ID",
						)}
					/>,
				]}
			/>
		),
	};
}

function updateEntry(s: Sentences, fact: Fact<"update">): Body {
	const look = UPDATE_LOOK[fact.state];
	return {
		kind: "update",
		icon: look.icon,
		tone: look.tone,
		text: withService(s, updateSentence(s.t, fact.state), fact.service),
		meta:
			fact.from === undefined ? undefined : (
				<Meta
					parts={[
						s.t(
							"devices:observe.timeline.update.from",
							"from settings v{{version, number}}",
							{ version: fact.from },
						),
					]}
				/>
			),
	};
}

function accessEntry(s: Sentences, fact: Fact<"access">): Body {
	const { t, target } = s;
	return {
		kind: "access",
		icon: Users,
		tone: fact.applied ? "good" : "info",
		text: fact.applied
			? t("devices:observe.timeline.accessApplied", {
					count: fact.people,
					version: fact.version,
					defaultValue_one:
						"Access rules v{{version, number}} saved and applied by the device: {{count, number}} person.",
					defaultValue_other:
						"Access rules v{{version, number}} saved and applied by the device: {{count, number}} people.",
				})
			: t(
					"devices:observe.timeline.accessWaiting",
					"Access rules v{{version, number}} saved. Waiting for {{device}} to apply them.",
					{ version: fact.version, device: target.name },
				),
	};
}

const ENTRY: {
	[T in TimelineFact["type"]]: (s: Sentences, fact: Fact<T>) => Body;
} = {
	instance: instanceEntry,
	command: commandEntry,
	update: updateEntry,
	access: accessEntry,
	boot: (s) => ({
		kind: "device",
		icon: Power,
		text: s.t("devices:observe.timeline.boot", "{{device}} started.", {
			device: s.target.name,
		}),
	}),
	agent_start: (s) => ({
		kind: "device",
		icon: Power,
		text: s.t(
			"devices:observe.timeline.agentStart",
			"The agent on {{device}} started.",
			{ device: s.target.name },
		),
	}),
	history_paused: (s, fact) => ({
		kind: "device",
		icon: History,
		tone: "warning",
		text: historyText(s, fact),
	}),
};

/** One fact as a timeline entry: icon, tone, sentence and its small print. */
export function entryOf(s: Sentences, fact: TimelineFact): TimelineEntry {
	const build = ENTRY[fact.type] as (s: Sentences, fact: TimelineFact) => Body;
	return { id: fact.id, at: fact.at, ...build(s, fact) };
}
