"use client";

import { useTranslation } from "@flow-like/locales";
import {
	BadgeCheck,
	CircleCheck,
	ClipboardCopy,
	LoaderCircle,
	type LucideIcon,
	OctagonX,
	PackageX,
	Plus,
	RefreshCw,
	Server,
	TriangleAlert,
} from "lucide-react";
import { type ReactNode, useState } from "react";
import type {
	DevicesRoute,
	HubDeviceSupport,
	HubErrorCode,
} from "../../../../lib/device-management/model/types";
import { gateCopy } from "../copy/gate-copy";
import {
	type AreaTime,
	type DevicesT,
	useAreaTime,
} from "../primitives/area-context";
import { Banner } from "../primitives/banner";
import { Block, PageHeader } from "../primitives/block";
import { dayText } from "../primitives/day";
import { DvButton } from "../primitives/dv-button";
import { type Gate, GatedAction } from "../primitives/gate-notice";
import { Headline } from "../primitives/headline";
import { InlineResult } from "../primitives/inline-result";
import { KeyValueList, KvRow } from "../primitives/key-value-list";
import { StatusChip } from "../primitives/status-chip";
import { type ChipTone, TONE_TEXT, type Tone, cx } from "../primitives/tone";
import { useDevicesRoute, useRouteLink } from "../routing/use-devices-route";
import type { ScreenProps } from "../screen-props";
import { useDeviceWorkspace } from "../workspace";
import { DiagnosticsResult, useDiagnostics } from "./diagnostics";
import { FeaturesBlock } from "./features-block";
import { HistoryBlock } from "./history-block";
import { Hint, HubReadStamp, Tech, UrlLine } from "./hub-parts";
import {
	type HubView,
	type ReleaseVerdict,
	type ReleaseVerdictKind,
	firstFailing,
	jumpTo,
	planName,
	useHubView,
	useReleaseVerdict,
} from "./hub-view";
import { HubLimitsUsage } from "./limits-usage";
import { ReadinessList, checkCopy } from "./readiness-list";
import { isLastDay } from "./release-copy";
import { ReleaseTrust } from "./release-trust";
import type { DeviceSlots } from "./use-hub-facts";

const FLEET_HOME: DevicesRoute = { screen: "fleet", view: "devices" };
const SETUP: DevicesRoute = { screen: "setup" };

type HubState = HubDeviceSupport["state"];

/* Header: the gated Set up a device. */

const setupGate = (t: DevicesT, view: HubView) => {
	const { setup, summary } = view;
	if (setup.ok) return undefined;
	const failing = firstFailing(view);
	const gate: Gate = { kind: setup.kind, reason: gateCopy(t, setup).inline };
	if (setup.copy.code !== "readiness_failing" || !summary || !failing)
		return gate;
	return {
		...gate,
		reason: t(
			"devices:hub.setup.needsChecks",
			"Setup needs all {{total, number}} checks to pass. {{check}} fails.",
			{ total: summary.total, check: checkCopy(t, failing).label },
		),
	};
};

/*
 * Phone width: the actions take the whole row with the primary action first
 * and full width. The page header primitive keeps its row, so the header's
 * last child (that row) is turned into a column from here.
 */
const PHONE_HEADER =
	"@max-[720px]/devices:[&>div:last-child]:flex-col @max-[720px]/devices:[&>div:last-child]:items-stretch";
const PHONE_ACTIONS =
	"@max-[720px]/devices:w-full @max-[720px]/devices:justify-start";
const PHONE_PRIMARY =
	"@max-[720px]/devices:order-first @max-[720px]/devices:w-full";

function SetupAction({ view }: Readonly<{ view: HubView }>) {
	const { t } = useTranslation("devices");
	const link = useRouteLink();
	const gate = setupGate(t, view);
	const label = t("hub.setup.action", "Set up a device");
	if (gate)
		return (
			<GatedAction
				gate={gate}
				className={cx(
					"items-end @max-[720px]/devices:items-stretch",
					PHONE_PRIMARY,
				)}
			>
				<DvButton variant="primary" icon={Plus}>
					{label}
				</DvButton>
			</GatedAction>
		);
	return (
		<DvButton variant="primary" icon={Plus} asChild className={PHONE_PRIMARY}>
			<a {...link(SETUP)}>{label}</a>
		</DvButton>
	);
}

/* The conclusion sentence. */

interface HeadlineText {
	lead: string;
	rest: string;
}

/** Before the checks can say anything: what the hub itself answered. */
const STATE_HEADLINE: Record<
	Exclude<HubState, "on">,
	(t: DevicesT, host: string) => HeadlineText
> = {
	checking: (t, host) => ({
		lead: t("devices:hub.headline.checking", "Checking {{host}}…", { host }),
		rest: t(
			"devices:hub.headline.checkingRest",
			"Device support, the hub status checks and the agent release appear in a moment. Nothing on this page changes your devices.",
		),
	}),
	unreachable: (t, host) => ({
		lead: t(
			"devices:hub.headline.unreachable",
			"It isn't known yet whether {{host}} is ready for devices.",
			{ host },
		),
		rest: t(
			"devices:hub.headline.unreachableRest",
			"Your devices keep running; nothing on them depends on this computer reaching the hub. The app keeps retrying on its own.",
		),
	}),
	off: (t, host) => ({
		lead: t("devices:hub.headline.off", "Devices are off on {{host}}.", {
			host,
		}),
		rest: t(
			"devices:hub.headline.offRest",
			"Ask the hub operator to turn on device support. Apps, flows and everything else in Flow-Like keep working.",
		),
	}),
};

const noChecksHeadline = (t: DevicesT, view: HubView) => ({
	lead: t("devices:hub.headline.supports", "{{host}} supports devices.", {
		host: view.host,
	}),
	rest: view.readiness.loading
		? t(
				"devices:hub.headline.supportsChecking",
				"The hub status checks are running…",
			)
		: t(
				"devices:hub.headline.supportsNoChecks",
				"The hub status checks couldn't run, so it isn't known yet whether setup works. Select Check again below.",
			),
});

/** A hub that is on but can't set up devices: the first failing check and what it blocks. */
const failingHeadline = (t: DevicesT, view: HubView) => {
	const { summary, host } = view;
	const failing = firstFailing(view);
	if (!summary || !failing) return undefined;
	const copy = checkCopy(t, failing);
	return {
		lead: t(
			"devices:hub.headline.notReady",
			"{{host}} can't set up devices right now: {{check}} fails.",
			{ host, check: copy.label },
		),
		rest: t(
			"devices:hub.headline.notReadyRest",
			"{{passed, number}} of {{total, number}} checks pass. {{blocks}} Only the hub operator can fix it.",
			{ passed: summary.passed, total: summary.total, blocks: copy.blocks },
		),
	};
};

const beingVerified = (t: DevicesT, all: string) =>
	t(
		"devices:hub.headline.verifying",
		"{{all}}. The agent release is being verified.",
		{ all },
	);

const notVerified = (t: DevicesT, all: string) =>
	t(
		"devices:hub.headline.unverified",
		"{{all}}, but the agent release couldn't be verified, so setup can't create a package.",
		{ all },
	);

/** "All 6 checks pass" continued by what the agent release adds to it; `until` is the day the release ends. */
const RELEASE_REST: Record<
	ReleaseVerdictKind,
	(t: DevicesT, all: string, until: string) => string
> = {
	ok: (t, all, until) =>
		t(
			"devices:hub.headline.verified",
			"{{all}} and the current agent release is verified until {{until}}.",
			{ all, until },
		),
	ends_soon: (t, all, until) =>
		t(
			"devices:hub.headline.endsSoon",
			"{{all}}, but the agent release runs out on {{until}}.",
			{ all, until },
		),
	expired: (t, all, until) =>
		t(
			"devices:hub.headline.expired",
			"{{all}}, but the agent release ran out on {{until}}, so setup can't create a package.",
			{ all, until },
		),
	checking: beingVerified,
	waiting: beingVerified,
	missing: (t, all) =>
		t(
			"devices:hub.headline.noRelease",
			"{{all}}, but the hub has no signed agent releases, so setup can't create a package.",
			{ all },
		),
	failed: notVerified,
	unfetched: notVerified,
};

const slotsRest = (t: DevicesT, slots: DeviceSlots | undefined) => {
	if (!slots) return "";
	const counts = { used: slots.used, max: slots.max };
	if (slots.over)
		return t(
			"devices:hub.headline.over",
			"You're over your device limit, though: {{used, number}} of {{max, number}}, so setting up another device fails.",
			counts,
		);
	if (slots.full)
		return t(
			"devices:hub.headline.full",
			"You're at your device limit, though: {{used, number}} of {{max, number}}, so setting up another device fails.",
			counts,
		);
	if (slots.near)
		return t(
			"devices:hub.headline.near",
			"You're using {{used, number}} of {{max, number}} device slots.",
			counts,
		);
	return "";
};

const readyLead = (t: DevicesT, view: HubView, time: AreaTime) => {
	const { hub, host } = view;
	const from = hub.support.error ? hub.freshness.dataFrom : undefined;
	if (from === undefined)
		return t("devices:hub.headline.ready", "{{host}} is ready for devices.", {
			host,
		});
	return t(
		"devices:hub.headline.wasReady",
		"{{host}} was ready for devices at {{time}}, the last time the app reached it.",
		{ host, time: time.clock(from, false) },
	);
};

/** Passing checks make the hub ready only with a usable release; until then the lead doesn't say "ready". */
const releaseLead = (
	t: DevicesT,
	view: HubView,
	verdict: ReleaseVerdict,
	time: AreaTime,
) => {
	if (verdict.kind === "ok" || verdict.kind === "ends_soon")
		return readyLead(t, view, time);
	if (verdict.kind === "checking" || verdict.kind === "waiting")
		return t("devices:hub.headline.checking", "Checking {{host}}…", {
			host: view.host,
		});
	return t(
		"devices:hub.headline.releaseBlocks",
		"{{host}} can't set up devices right now.",
		{ host: view.host },
	);
};

const readyHeadline = (
	t: DevicesT,
	view: HubView,
	verdict: ReleaseVerdict,
	time: AreaTime,
) => {
	const total = view.summary ? view.summary.total : 0;
	const all = t(
		"devices:hub.headline.allPass",
		"All {{total, number}} checks pass",
		{ total },
	);
	const until =
		"facts" in verdict && verdict.facts
			? dayText(time, verdict.facts.expires_at)
			: "";
	const release = RELEASE_REST[verdict.kind](t, all, until);
	return {
		lead: releaseLead(t, view, verdict, time),
		rest: [release, slotsRest(t, view.slots)].filter(Boolean).join(" "),
	};
};

function useHeadline(view: HubView): HeadlineText {
	const { t } = useTranslation("devices");
	const time = useAreaTime();
	const verdict = useReleaseVerdict(view);
	const { state } = view.hub.support;
	if (state !== "on") return STATE_HEADLINE[state](t, view.host);
	if (!view.summary) return noChecksHeadline(t, view);
	return failingHeadline(t, view) ?? readyHeadline(t, view, verdict, time);
}

/* Block: Hub. Four answers at a glance, each jumping to its block. */

function SummaryCell({
	to,
	label,
	value,
	sub,
	verdict,
}: Readonly<{
	to: string;
	label: string;
	value: ReactNode;
	sub: ReactNode;
	/** The release verdict this cell states. */
	verdict?: ReleaseVerdictKind;
}>) {
	return (
		<button
			type="button"
			data-jump={to}
			data-verdict={verdict}
			onClick={() => jumpTo(to)}
			className="flex min-w-0 cursor-pointer flex-col gap-1 bg-card px-4 py-3 text-left hover:bg-row-hover focus-visible:outline-2 focus-visible:-outline-offset-2 focus-visible:outline-ring @max-[480px]/devices:px-3 @max-[480px]/devices:py-2.5"
		>
			<span className="text-label font-semibold tracking-[0.06em] text-muted-foreground uppercase">
				{label}
			</span>
			<span className="flex min-h-6 flex-wrap items-center gap-1.5 text-base/[22px] font-semibold text-foreground">
				{value}
			</span>
			<span className="text-xs text-muted-foreground">{sub}</span>
		</button>
	);
}

const DASH = <span className="font-normal text-muted-foreground">–</span>;

const SUPPORT_CHIP: Record<
	HubState,
	{ tone: ChipTone; icon: LucideIcon; spin?: boolean }
> = {
	checking: { tone: "info", icon: LoaderCircle, spin: true },
	unreachable: { tone: "critical", icon: OctagonX },
	off: { tone: "critical", icon: OctagonX },
	on: { tone: "good", icon: CircleCheck },
};

const supportWord = (t: DevicesT, state: HubState) => {
	const words: Record<HubState, string> = {
		checking: t("devices:hub.sum.checking", "Checking…"),
		unreachable: t("devices:hub.sum.unreachable", "Hub unreachable"),
		off: t("devices:hub.sum.off", "Off on this hub"),
		on: t("devices:hub.sum.on", "On"),
	};
	return words[state];
};

const supportSub = (t: DevicesT, view: HubView, time: AreaTime) => {
	const { hub, host } = view;
	const from = hub.support.error ? hub.freshness.dataFrom : undefined;
	const subs: Record<HubState, string> = {
		checking: t("devices:hub.sum.asking", "Asking {{host}}", { host }),
		unreachable: t(
			"devices:hub.sum.unreachableSub",
			"Its device settings couldn't be read",
		),
		off: t("devices:hub.sum.offSub", "The hub operator turned it off"),
		on:
			from === undefined
				? t("devices:hub.sum.onSub", "Devices can be set up and managed")
				: t("devices:hub.sum.lastKnown", "Last known · read at {{time}}", {
						time: time.clock(from, false),
					}),
	};
	return subs[hub.support.state];
};

function SupportCell({ view }: Readonly<{ view: HubView }>) {
	const { t } = useTranslation("devices");
	const time = useAreaTime();
	const { state } = view.hub.support;
	const chip = SUPPORT_CHIP[state];
	return (
		<SummaryCell
			to="features"
			label={t("hub.sum.support", "Device support")}
			value={
				<StatusChip tone={chip.tone} icon={chip.icon} spin={chip.spin}>
					{supportWord(t, state)}
				</StatusChip>
			}
			sub={supportSub(t, view, time)}
		/>
	);
}

const noChecksSub = (t: DevicesT, view: HubView) => {
	if (view.readiness.loading || view.hub.support.state === "checking")
		return t("devices:hub.sum.waiting", "Waiting for the hub");
	return t("devices:hub.sum.checksNotRun", "Not run yet");
};

const checksSub = (t: DevicesT, view: HubView, time: AreaTime) => {
	const failing = firstFailing(view);
	if (failing)
		return t("devices:hub.sum.fails", "{{check}} fails", {
			check: checkCopy(t, failing).label,
		});
	const { freshness } = view.readiness;
	const at = freshness.at ?? freshness.dataFrom;
	if (at === undefined) return null;
	return t("devices:hub.sum.checkedAt", "checked {{time}}", {
		time: time.clock(at, false),
	});
};

function ChecksCell({ view }: Readonly<{ view: HubView }>) {
	const { t } = useTranslation("devices");
	const time = useAreaTime();
	const { summary } = view;
	const label = t("hub.sum.checks", "Status checks");
	if (!summary)
		return (
			<SummaryCell
				to="readiness"
				label={label}
				value={DASH}
				sub={noChecksSub(t, view)}
			/>
		);
	const Icon = summary.failing.length ? OctagonX : CircleCheck;
	const tone = summary.failing.length ? TONE_TEXT.critical : TONE_TEXT.good;
	return (
		<SummaryCell
			to="readiness"
			label={label}
			value={
				<>
					<Icon aria-hidden className={cx("size-4", tone)} />
					<span className="tabular-nums">
						{t("hub.sum.passCount", "{{passed, number}} of {{total, number}}", {
							passed: summary.passed,
							total: summary.total,
						})}
					</span>
					{t("hub.sum.pass", "pass")}
				</>
			}
			sub={checksSub(t, view, time)}
		/>
	);
}

interface ReleaseLook {
	/** The agent the release names, once a genuinely signed list is in hand. */
	version?: string;
	/** Icon, its tone and the word beside it; without them the cell shows a dash. */
	mark?: { icon: LucideIcon; tone: Tone; word: string };
	sub: string;
}

/** The agent release at a glance; the icon is green only for a verified release with time left. */
const releaseLook = (
	t: DevicesT,
	time: AreaTime,
	verdict: ReleaseVerdict,
): ReleaseLook => {
	const notVerified = t("devices:hub.sum.unverified", "Not verified");
	const waits = t(
		"devices:hub.sum.unverifiedSub",
		"Setup and agent updates wait",
	);
	switch (verdict.kind) {
		case "ok":
			return {
				version: verdict.facts.release_version,
				mark: {
					icon: BadgeCheck,
					tone: "good",
					word: t("devices:hub.sum.verified", "Verified"),
				},
				sub: t("devices:hub.sum.untilDate", "until {{when}}", {
					when: dayText(time, verdict.facts.expires_at),
				}),
			};
		case "ends_soon":
			return {
				version: verdict.facts.release_version,
				mark: {
					icon: TriangleAlert,
					tone: "warning",
					word: t("devices:hub.sum.endsSoon", "Ends soon"),
				},
				sub: isLastDay(verdict.facts.expires_at, time.nowS)
					? t(
							"devices:hub.sum.untilLastDay",
							"until {{when}} · less than a day left",
							{ when: time.at(verdict.facts.expires_at) },
						)
					: t("devices:hub.sum.until", {
							when: dayText(time, verdict.facts.expires_at),
							count: verdict.daysLeft,
							defaultValue_one: "until {{when}} · {{count, number}} day left",
							defaultValue_other:
								"until {{when}} · {{count, number}} days left",
						}),
			};
		case "expired":
			return {
				version: verdict.facts.release_version,
				mark: {
					icon: OctagonX,
					tone: "critical",
					word: t("devices:hub.release.expired", "Expired"),
				},
				sub: t("devices:hub.sum.expiredSub", "ran out on {{when}}", {
					when: dayText(time, verdict.facts.expires_at),
				}),
			};
		case "failed":
			return {
				mark: { icon: OctagonX, tone: "critical", word: notVerified },
				sub: waits,
			};
		case "unfetched":
			return {
				mark: { icon: TriangleAlert, tone: "warning", word: notVerified },
				sub: waits,
			};
		case "missing":
			return {
				mark: {
					icon: PackageX,
					tone: "warning",
					word: t("devices:hub.sum.none", "None"),
				},
				sub: t("devices:hub.sum.noneSub", "No signed agent releases"),
			};
		case "checking":
			return { sub: t("devices:hub.sum.verifying", "Verifying the release…") };
		default:
			return { sub: t("devices:hub.sum.waiting", "Waiting for the hub") };
	}
};

function ReleaseCell({ view }: Readonly<{ view: HubView }>) {
	const { t } = useTranslation("devices");
	const time = useAreaTime();
	const verdict = useReleaseVerdict(view);
	const { version, mark, sub } = releaseLook(t, time, verdict);
	return (
		<SummaryCell
			to="releases"
			verdict={verdict.kind}
			label={t("hub.sum.release", "Agent release")}
			value={
				mark ? (
					<>
						{version ? (
							<span className="font-mono text-[15px]">{version}</span>
						) : null}
						<mark.icon
							aria-hidden
							className={cx("size-4", TONE_TEXT[mark.tone])}
						/>
						{mark.word}
					</>
				) : (
					DASH
				)
			}
			sub={sub}
		/>
	);
}

/** Why the Hub block shows a limit without your count. */
const noUsageReason = (t: DevicesT, view: HubView) => {
	const { usage, hub } = view;
	const { state } = hub.support;
	if (state === "checking")
		return t("devices:hub.sum.waiting", "Waiting for the hub");
	if (state === "off")
		return t("devices:hub.sum.usageOff", "Counted once device support is on");
	if (state === "unreachable")
		return t("devices:hub.sum.usageUnknown", "Not known until the hub answers");
	if (usage.missingOnHub)
		return t("devices:hub.sum.noUsage", "This hub doesn't report your usage");
	if (usage.loading)
		return t("devices:hub.sum.usageLoading", "Reading your usage…");
	return t("devices:hub.sum.usageUnread", "Your usage couldn't be read");
};

function SlotsValue({ slots }: Readonly<{ slots: DeviceSlots }>) {
	const { t } = useTranslation("devices");
	return (
		<>
			<span className="tabular-nums">
				{t("hub.sum.used", "{{used, number}}", { used: slots.used })}
			</span>
			<span className="font-normal text-muted-foreground tabular-nums">
				{t("hub.sum.ofMax", "of {{max, number}}", { max: slots.max })}
			</span>
			{slots.full ? (
				<StatusChip tone="warning" icon={TriangleAlert}>
					{slots.over
						? t("hub.sum.over", "Over the limit")
						: t("hub.sum.full", "At the limit")}
				</StatusChip>
			) : null}
		</>
	);
}

function UsageCell({ view }: Readonly<{ view: HubView }>) {
	const { t } = useTranslation("devices");
	const { slots, limits } = view;
	const label = t("hub.sum.devices", "Your devices");
	if (slots)
		return (
			<SummaryCell
				to="limits"
				label={label}
				value={<SlotsValue slots={slots} />}
				sub={
					slots.pending
						? t("hub.sum.pendingIncluded", {
								count: slots.pending,
								defaultValue_one:
									"includes {{count, number}} unused setup package",
								defaultValue_other:
									"includes {{count, number}} unused setup packages",
							})
						: t("hub.sum.noPending", "no unused setup packages")
				}
			/>
		);
	return (
		<SummaryCell
			to="limits"
			label={label}
			value={
				limits.devices === undefined ? (
					DASH
				) : (
					<span className="font-normal text-muted-foreground">
						{t("hub.sum.upTo", "up to {{max, number}}", {
							max: limits.devices,
						})}
					</span>
				)
			}
			sub={noUsageReason(t, view)}
		/>
	);
}

/** The address every new setup package points a device at. */
function ApiAddress({ view }: Readonly<{ view: HubView }>) {
	const { t } = useTranslation("devices");
	const { record, origin, hub } = view;
	if (!record)
		return (
			<span className="text-muted-foreground">
				{hub.support.state === "checking"
					? t("hub.block.apiChecking", "Checking…")
					: t("hub.block.apiUnknown", "Not read yet")}
			</span>
		);
	return (
		<UrlLine
			url={record.api_base_url ?? `${origin}/api/v1`}
			copyLabel={t("hub.block.copyApi", "Copy public device API address")}
		/>
	);
}

function HubBlock({ view }: Readonly<{ view: HubView }>) {
	const { t } = useTranslation("devices");
	const { hub, origin, archive } = view;
	return (
		<Block
			id="hub"
			icon={Server}
			title={t("hub.block.title", "Hub")}
			stamp={
				<HubReadStamp
					freshness={hub.freshness}
					loading={hub.support.state === "checking"}
				/>
			}
			flush
			className="scroll-mt-4"
		>
			<div
				data-hub="summary"
				className="grid grid-cols-4 gap-px border-b border-hairline bg-hairline @max-[900px]/devices:grid-cols-2"
			>
				<SupportCell view={view} />
				<ChecksCell view={view} />
				<ReleaseCell view={view} />
				<UsageCell view={view} />
			</div>
			<div className="px-4 py-3">
				<KeyValueList>
					<KvRow label={t("hub.block.address", "Hub")}>
						<UrlLine
							url={origin}
							copyLabel={t("hub.block.copyAddress", "Copy hub address")}
						/>
					</KvRow>
					<KvRow label={t("hub.block.api", "Public device API")}>
						<ApiAddress view={view} />
						<Hint className="mt-0.5">
							{t(
								"hub.block.apiHint",
								"Every new setup package tells the device to reach the hub here.",
							)}
						</Hint>
					</KvRow>
					{archive.data ? (
						<KvRow label={t("hub.block.plan", "Your plan")}>
							{planName(archive.data.tier)}
							<Tech>{archive.data.tier}</Tech>
						</KvRow>
					) : null}
				</KeyValueList>
			</div>
		</Block>
	);
}

/* Unreachable: the one page-level condition this screen states itself (the area shows no gate here). */

const refused = (t: DevicesT, host: string) =>
	t("devices:hub.unreachable.refused", "{{host}} refused the request.", {
		host,
	});

/** Why the hub's device settings couldn't be read, as one sentence about the hub. */
const UNREACHABLE_TITLE: Record<
	HubErrorCode,
	(t: DevicesT, host: string) => string
> = {
	network: (t, host) =>
		t("devices:hub.unreachable.title", "Can't reach {{host}}.", { host }),
	timeout: (t, host) =>
		t(
			"devices:hub.unreachable.timeout",
			"Can't reach {{host}}: it didn't answer in time.",
			{ host },
		),
	server_error: (t, host) =>
		t(
			"devices:hub.unreachable.serverError",
			"{{host}} answered with a server error.",
			{ host },
		),
	rate_limited: (t, host) =>
		t(
			"devices:hub.unreachable.rateLimited",
			"{{host}} is limiting requests right now.",
			{ host },
		),
	invalid_response: (t, host) =>
		t(
			"devices:hub.unreachable.invalidResponse",
			"{{host}} sent an answer this app can't read.",
			{ host },
		),
	not_found: (t, host) =>
		t(
			"devices:hub.unreachable.notFound",
			"{{host}} doesn't say whether it supports devices.",
			{ host },
		),
	unauthorized: refused,
	forbidden: refused,
	token_restricted: refused,
};

type RetryPhase = "busy" | "tried";

/** "Retry now" with a visible outcome; an attempt made under another account is forgotten. */
function useRetry(view: HubView) {
	const scope = useDeviceWorkspace().scopeKey;
	const [attempt, setAttempt] = useState<{
		scope: string;
		phase: RetryPhase;
	}>();
	const run = async () => {
		setAttempt({ scope, phase: "busy" });
		await view.hub.retry();
		setAttempt((current) =>
			current?.scope === scope ? { scope, phase: "tried" } : current,
		);
	};
	return {
		phase: attempt?.scope === scope ? attempt.phase : undefined,
		run,
		dismiss: () => setAttempt(undefined),
	};
}

function UnreachableBanner({ view }: Readonly<{ view: HubView }>) {
	const { t } = useTranslation("devices");
	const time = useAreaTime();
	const retry = useRetry(view);
	const { hub, host } = view;
	if (hub.support.state !== "unreachable") return null;
	const code = hub.support.error ? hub.support.error.code : "network";
	const still = retry.phase === "tried" && hub.failedAt !== undefined;
	return (
		<Banner
			tone="critical"
			title={UNREACHABLE_TITLE[code](t, host)}
			actions={
				<>
					<DvButton
						size="sm"
						icon={RefreshCw}
						busy={retry.phase === "busy" || hub.checking}
						onClick={() => {
							void retry.run();
						}}
					>
						{t("hub.unreachable.retry", "Retry now")}
					</DvButton>
					{still ? (
						<InlineResult tone="warning" onDismiss={retry.dismiss}>
							{t(
								"hub.unreachable.still",
								"Still unreachable at {{time}}. The app keeps retrying on its own.",
								{ time: time.clock((hub.failedAt ?? 0) / 1000) },
							)}
						</InlineResult>
					) : null}
				</>
			}
		>
			{t(
				"hub.unreachable.text",
				"The app keeps retrying on its own. Devices may still reach the hub; this computer can't tell until it reconnects.",
			)}
		</Banner>
	);
}

/** Title, Copy diagnostics with its result, and the one primary action. */
function HubHeader({ view }: Readonly<{ view: HubView }>) {
	const { t } = useTranslation("devices");
	const { navigate, href } = useDevicesRoute();
	const diagnostics = useDiagnostics(view);
	const title = t("hub.title", "Hub status");
	return (
		<>
			<PageHeader
				crumbs={[
					{
						label: t("hub.crumb.devices", "Devices"),
						href: href(FLEET_HOME),
						onNavigate: () => navigate(FLEET_HOME),
					},
					{ label: title },
				]}
				title={title}
				sub={t(
					"hub.subtitle",
					"{{host}} · read-only · everyone signed in to this hub sees the same checks",
					{ host: view.host },
				)}
				className={PHONE_HEADER}
				actions={
					<div
						className={cx(
							"flex flex-wrap items-start justify-end gap-2",
							PHONE_ACTIONS,
						)}
					>
						<DvButton
							icon={ClipboardCopy}
							onClick={() => {
								void diagnostics.copy();
							}}
						>
							{t("hub.diagnostics.action", "Copy diagnostics")}
						</DvButton>
						<SetupAction view={view} />
					</div>
				}
			/>
			<DiagnosticsResult diagnostics={diagnostics} />
		</>
	);
}

function HubHeadline({ view }: Readonly<{ view: HubView }>) {
	const headline = useHeadline(view);
	return <Headline lead={headline.lead} rest={headline.rest} />;
}

/** N10 Hub status (SPEC §5.10, IA §6.2 N10): is this hub ready for devices, and what are its limits. Read-only. */
export function HubScreen(_props: Readonly<ScreenProps>) {
	const view = useHubView();
	return (
		<div data-screen="hub" className="flex min-w-0 flex-col gap-6">
			<UnreachableBanner view={view} />
			<HubHeader view={view} />
			<HubHeadline view={view} />
			<HubBlock view={view} />
			<div className="grid grid-cols-[minmax(0,1.6fr)_minmax(320px,1fr)] items-start gap-6 @max-[1080px]/devices:grid-cols-1">
				<ReadinessList />
				<FeaturesBlock view={view} />
			</div>
			<HubLimitsUsage />
			<ReleaseTrust />
			<HistoryBlock view={view} />
		</div>
	);
}
