"use client";

import { useTranslation } from "@flow-like/locales";
import {
	Ban,
	Boxes,
	CircleCheck,
	CircleDashed,
	CircleDot,
	Layers,
	LockKeyhole,
	type LucideIcon,
	Package,
	Plus,
	Radio,
	TriangleAlert,
} from "lucide-react";
import type { ReactNode } from "react";
import { enumLabel } from "../copy/enum-labels";
import {
	type AreaTime,
	type DevicesT,
	useAreaTime,
} from "../primitives/area-context";
import { Block } from "../primitives/block";
import { dayText } from "../primitives/day";
import { DvButton } from "../primitives/dv-button";
import { FreshnessStamp } from "../primitives/freshness-stamp";
import { StatusChip } from "../primitives/status-chip";
import type { ChipTone } from "../primitives/tone";
import { useRouteLink } from "../routing/use-devices-route";
import { stampOf } from "../shell/attention-popover";
import { HubReadStamp } from "./hub-parts";
import {
	type HubView,
	type ReleaseVerdict,
	failsCheck,
	firstFailing,
	jumpTo,
	usableRelease,
	useReleaseVerdict,
} from "./hub-view";
import { checkCopy } from "./readiness-list";

const LINK = "underline decoration-border-strong underline-offset-2";

type FeatureState = "ok" | "blocked" | "partly" | "unknown";

interface Verdict {
	state: FeatureState;
	why: string;
}

interface Feature extends Verdict {
	id: string;
	icon: LucideIcon;
	label: string;
	need: string;
	action?: ReactNode;
}

const FEATURE_CHIP: Record<FeatureState, { tone: ChipTone; icon: LucideIcon }> =
	{
		ok: { tone: "good", icon: CircleCheck },
		blocked: { tone: "warning", icon: Ban },
		partly: { tone: "info", icon: CircleDot },
		unknown: { tone: "unknown", icon: CircleDashed },
	};

interface Facts {
	t: DevicesT;
	view: HubView;
	/** The one verdict about the agent release; no rule here judges the release by itself. */
	verdict: ReleaseVerdict;
	time: AreaTime;
}

/** One reason a feature isn't simply working; `undefined` = this reason doesn't apply. */
type Rule = (facts: Facts) => Verdict | undefined;

/** The first rule that applies decides; with none, the feature works. */
const decide = (facts: Facts, rules: readonly Rule[], works: string) => {
	for (const rule of rules) {
		const verdict = rule(facts);
		if (verdict) return verdict;
	}
	return { state: "ok" as const, why: works };
};

const stateIs = (view: HubView, state: string) =>
	view.hub.support.state === state;

const whileChecking: Rule = ({ t, view }) =>
	stateIs(view, "checking")
		? {
				state: "unknown",
				why: t("devices:hub.features.asking", "Waiting for the hub's answer."),
			}
		: undefined;

const whileOff: Rule = ({ t, view }) =>
	stateIs(view, "off")
		? {
				state: "blocked",
				why: t("devices:hub.features.off", "Device support is off."),
			}
		: undefined;

const withoutChecks: Rule = ({ t, view }) => {
	if (view.summary) return undefined;
	return {
		state: "unknown",
		why: view.readiness.loading
			? t("devices:hub.features.waiting", "Waiting for the checks.")
			: t(
					"devices:hub.features.noChecks",
					"The checks haven't run, so this isn't known yet.",
				),
	};
};

const anyCheckFails: Rule = ({ t, view }) => {
	const failing = firstFailing(view);
	if (!failing) return undefined;
	return {
		state: "blocked",
		why: t("devices:hub.features.checkFails", "{{check}} fails.", {
			check: checkCopy(t, failing).label,
		}),
	};
};

const setupUnreachable: Rule = ({ t, view }) =>
	stateIs(view, "unreachable")
		? {
				state: "blocked",
				why: t(
					"devices:hub.features.setup.unreachable",
					"Setup needs the hub, and this computer can't reach it.",
				),
			}
		: undefined;

const noDeviceSlot: Rule = ({ t, view }) => {
	const { slots } = view;
	if (!slots?.full) return undefined;
	const counts = { used: slots.used, max: slots.max };
	return {
		state: "blocked",
		why: slots.over
			? t(
					"devices:hub.features.setup.over",
					"You're over your device limit: {{used, number}} of {{max, number}}.",
					counts,
				)
			: t(
					"devices:hub.features.setup.full",
					"You're at your device limit: {{used, number}} of {{max, number}}.",
					counts,
				),
	};
};

const releasePending: Rule = ({ t, verdict }) => {
	if (verdict.kind !== "checking" && verdict.kind !== "waiting")
		return undefined;
	return {
		state: "unknown",
		why: t(
			"devices:hub.features.setup.verifying",
			"The agent release is being verified.",
		),
	};
};

const setupRanOut: Rule = ({ t, verdict, time }) =>
	verdict.kind === "expired"
		? {
				state: "blocked",
				why: t(
					"devices:hub.features.setup.expired",
					"The agent release ran out on {{until}}, so no setup package can be made.",
					{ until: dayText(time, verdict.facts.expires_at) },
				),
			}
		: undefined;

const updateRanOut: Rule = ({ t, verdict, time }) =>
	verdict.kind === "expired"
		? {
				state: "blocked",
				why: t(
					"devices:hub.features.update.expired",
					"The agent release ran out on {{until}}. Devices keep the agent they have.",
					{ until: dayText(time, verdict.facts.expires_at) },
				),
			}
		: undefined;

const noReleaseToPackage: Rule = ({ t, verdict }) =>
	usableRelease(verdict)
		? undefined
		: {
				state: "blocked",
				why: t(
					"devices:hub.features.setup.unverified",
					"There is no verified agent release to put into a setup package.",
				),
			};

const liveUnreachable: Rule = ({ t, view }) =>
	stateIs(view, "unreachable")
		? {
				state: "partly",
				why: t(
					"devices:hub.features.live.unreachable",
					"Connections that are already open keep working until they renew, within 5 minutes. New ones start at the hub.",
				),
			}
		: undefined;

const noConnectionService: Rule = ({ t, view }) =>
	failsCheck(view, "signaling")
		? {
				state: "blocked",
				why: t(
					"devices:hub.features.live.blocked",
					"Connection service fails. Encrypted status still arrives.",
				),
			}
		: undefined;

const noSignedReleases: Rule = ({ t, verdict }) =>
	verdict.kind === "missing"
		? {
				state: "blocked",
				why: t(
					"devices:hub.features.update.missing",
					"This hub has no signed agent releases. Devices keep the agent they have.",
				),
			}
		: undefined;

const updateUnreachable: Rule = ({ t, view }) =>
	stateIs(view, "unreachable")
		? {
				state: "blocked",
				why: t(
					"devices:hub.features.update.unreachable",
					"Updates start over a live connection, which starts at the hub.",
				),
			}
		: undefined;

const updateNeedsLive: Rule = ({ t, view }) =>
	failsCheck(view, "signaling")
		? {
				state: "blocked",
				why: t(
					"devices:hub.features.update.noLive",
					"Updates start over a live connection, and the connection service fails.",
				),
			}
		: undefined;

const releaseUnverified: Rule = ({ t, verdict }) =>
	verdict.kind === "failed" || verdict.kind === "unfetched"
		? {
				state: "blocked",
				why: t(
					"devices:hub.features.update.unverified",
					"The agent release couldn't be verified. Devices keep the agent they have.",
				),
			}
		: undefined;

const checkInUnreachable: Rule = ({ t, view }) =>
	stateIs(view, "unreachable")
		? {
				state: "unknown",
				why: t(
					"devices:hub.features.checkin.unreachable",
					"This computer can't tell. Devices may still reach the hub.",
				),
			}
		: undefined;

const checkInOff: Rule = ({ t, view }) =>
	stateIs(view, "off")
		? {
				state: "blocked",
				why: t(
					"devices:hub.features.checkin.off",
					"Devices can't check in while device support is off.",
				),
			}
		: undefined;

/** Checks a device needs to sign in and be stored: without them it can't check in. */
const CHECK_IN_NEEDS = ["policy", "signing", "database"] as const;

const checkInBlocked: Rule = ({ t, view }) => {
	const blocking = CHECK_IN_NEEDS.find((id) => failsCheck(view, id));
	return blocking
		? { state: "blocked", why: checkCopy(t, blocking).blocks }
		: undefined;
};

const setupVerdict = (facts: Facts) => {
	const release = usableRelease(facts.verdict);
	const platforms = release
		? release.targets.map((target) => enumLabel(facts.t, "targetShort", target))
		: [];
	return decide(
		facts,
		[
			setupUnreachable,
			whileChecking,
			whileOff,
			withoutChecks,
			anyCheckFails,
			noDeviceSlot,
			releasePending,
			setupRanOut,
			noReleaseToPackage,
		],
		facts.t(
			"devices:hub.features.setup.ok",
			"Signed agent release {{version}} for {{platforms}}.",
			{
				version: release ? release.manifest.release_version : "",
				platforms: platforms.join(", "),
			},
		),
	);
};

const liveVerdict = (facts: Facts) =>
	decide(
		facts,
		[
			liveUnreachable,
			whileChecking,
			whileOff,
			withoutChecks,
			noConnectionService,
		],
		facts.t(
			"devices:hub.features.live.ok",
			"Direct, or relayed through the hub. Both are end-to-end encrypted.",
		),
	);

/** What a usable release means for updates; inside the last 30 days it says when updates stop. */
const updateWorks = ({ t, verdict, time }: Facts) => {
	const release = usableRelease(verdict);
	const values = {
		version: release ? release.manifest.release_version : "",
		until: release ? dayText(time, release.manifest.expires_at) : "",
	};
	return verdict.kind === "ends_soon"
		? t(
				"devices:hub.features.update.endsSoon",
				"Current release {{version}} · runs out on {{until}}. After that, updates wait for a new or renewed release.",
				values,
			)
		: t(
				"devices:hub.features.update.ok",
				"Current release {{version}} · verified until {{until}}.",
				values,
			);
};

const updateVerdict = (facts: Facts) =>
	decide(
		facts,
		[
			whileChecking,
			whileOff,
			noSignedReleases,
			updateUnreachable,
			withoutChecks,
			updateNeedsLive,
			releaseUnverified,
			updateRanOut,
			releasePending,
		],
		updateWorks(facts),
	);

const checkInVerdict = (facts: Facts) =>
	decide(
		facts,
		[checkInUnreachable, checkInOff, whileChecking, checkInBlocked],
		facts.t(
			"devices:hub.features.checkin.ok",
			"About once a minute. Encrypted status at least every 60 s.",
		),
	);

function useSetupFeature(facts: Facts): Feature {
	const { t } = useTranslation("devices");
	const link = useRouteLink();
	const verdict = setupVerdict(facts);
	const open = verdict.state === "ok" && facts.view.setup.ok;
	return {
		id: "setup",
		icon: Plus,
		label: t("hub.features.setup.label", "Set up new devices"),
		need: t(
			"hub.features.setup.need",
			"Needs all the checks and a verified agent release.",
		),
		...verdict,
		...(open
			? {
					action: (
						<a {...link({ screen: "setup" })} className={LINK}>
							{t("hub.setup.action", "Set up a device")}
						</a>
					),
				}
			: {}),
	};
}

function useUpdateFeature(facts: Facts): Feature {
	const { t } = useTranslation("devices");
	const verdict = updateVerdict(facts);
	return {
		id: "update",
		icon: Package,
		label: t("hub.features.update.label", "Remote agent updates"),
		need: t(
			"hub.features.update.need",
			"Needs signed agent releases and a live connection.",
		),
		...verdict,
		...(verdict.state === "ok"
			? {
					action: (
						<DvButton
							variant="link"
							size="xs"
							onClick={() => jumpTo("releases")}
						>
							{t("hub.features.update.see", "See which devices can update")}
						</DvButton>
					),
				}
			: {}),
	};
}

function useFeatures(view: HubView): Feature[] {
	const { t } = useTranslation("devices");
	const time = useAreaTime();
	const verdict = useReleaseVerdict(view);
	const facts: Facts = { t, view, verdict, time };
	const setup = useSetupFeature(facts);
	const update = useUpdateFeature(facts);
	const away = stateIs(view, "off") || stateIs(view, "unreachable");
	return [
		setup,
		{
			id: "live",
			icon: Radio,
			label: t("hub.features.live.label", "Live connections"),
			need: t(
				"hub.features.live.need",
				"Unlock, then Connect live: services, logs and commands. Needs the connection service.",
			),
			...liveVerdict(facts),
		},
		update,
		{
			id: "checkin",
			icon: LockKeyhole,
			label: t("hub.features.checkin.label", "Check-ins and encrypted status"),
			need: t("hub.features.checkin.need", "Needs device support on this hub."),
			...checkInVerdict(facts),
		},
		{
			id: "running",
			icon: Boxes,
			label: t("hub.features.running.label", "Devices already set up"),
			need: t(
				"hub.features.running.need",
				"Their services don't depend on the hub.",
			),
			state: "ok",
			why: away
				? t(
						"hub.features.running.away",
						"Services keep running as requested. Services with cloud access need the hub for new credentials every 10 minutes.",
					)
				: t(
						"hub.features.running.ok",
						"Services keep running as requested, whatever happens to the hub.",
					),
		},
	];
}

function FeatureRow({ feature }: Readonly<{ feature: Feature }>) {
	const { t } = useTranslation("devices");
	const stateText: Record<FeatureState, string> = {
		ok: t("hub.features.state.ok", "Works"),
		blocked: t("hub.features.state.blocked", "Blocked"),
		partly: t("hub.features.state.partly", "Partly"),
		unknown: t("hub.features.state.unknown", "Unknown"),
	};
	const chip = FEATURE_CHIP[feature.state];
	return (
		<li
			data-feature={feature.id}
			data-state={feature.state}
			className="grid grid-cols-[18px_minmax(0,1fr)_auto] items-start gap-x-2.5 gap-y-0.5 border-t border-hairline py-2.5 text-ui first:border-t-0 first:pt-0.5"
		>
			<feature.icon
				aria-hidden
				className="mt-px size-4 text-muted-foreground"
			/>
			<div className="min-w-0">
				<b className="font-semibold">{feature.label}</b>
				<p className="mt-0.5 text-xs text-muted-foreground">{feature.need}</p>
				<p className="mt-0.5 text-xs text-ink-2">
					{feature.why}
					{feature.action ? <> {feature.action}</> : null}
				</p>
			</div>
			<StatusChip tone={chip.tone} icon={chip.icon}>
				{stateText[feature.state]}
			</StatusChip>
		</li>
	);
}

/** "5 of 5 work", once every feature has a verdict. */
function FeatureCount({ features }: Readonly<{ features: Feature[] }>) {
	const { t } = useTranslation("devices");
	if (features.some((feature) => feature.state === "unknown")) return null;
	const working = features.filter((feature) => feature.state === "ok").length;
	const allWork = working === features.length;
	return (
		<StatusChip
			tone={allWork ? "good" : "warning"}
			icon={allWork ? CircleCheck : TriangleAlert}
		>
			{t(
				"hub.features.count",
				"{{working, number}} of {{total, number}} work",
				{ working, total: features.length },
			)}
		</StatusChip>
	);
}

/** Worked out from the checks, so the stamp names them and their time. */
function FeaturesStamp({ view }: Readonly<{ view: HubView }>) {
	const { t } = useTranslation("devices");
	const time = useAreaTime();
	const { readiness, hub } = view;
	const checkedAt = readiness.data
		? (readiness.freshness.at ?? readiness.freshness.dataFrom)
		: undefined;
	if (checkedAt === undefined)
		return (
			<HubReadStamp
				freshness={hub.freshness}
				loading={hub.support.state === "checking"}
			/>
		);
	return (
		<FreshnessStamp
			{...stampOf(readiness.freshness)}
			observedAt={checkedAt}
			text={t("hub.features.stamp", "from the checks {{ago}}", {
				ago: time.ago(checkedAt),
			})}
			noFail={hub.support.state === "off"}
		/>
	);
}

/** What the hub's state means in practice: which device features work right now, and why not. */
export function FeaturesBlock({ view }: Readonly<{ view: HubView }>) {
	const { t } = useTranslation("devices");
	const features = useFeatures(view);
	return (
		<Block
			id="features"
			icon={Layers}
			title={t("hub.features.title", "What works on this hub")}
			className="scroll-mt-4"
			summary={<FeatureCount features={features} />}
			stamp={<FeaturesStamp view={view} />}
		>
			<ul className="flex flex-col">
				{features.map((feature) => (
					<FeatureRow key={feature.id} feature={feature} />
				))}
			</ul>
		</Block>
	);
}
