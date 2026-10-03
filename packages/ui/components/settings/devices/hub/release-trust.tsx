"use client";

import { useTranslation } from "@flow-like/locales";
import {
	ArrowUp,
	BadgeCheck,
	CircleCheck,
	List,
	LoaderCircle,
	LockOpen,
	type LucideIcon,
	OctagonX,
	PackageCheck,
	PackageX,
	RefreshCw,
	TriangleAlert,
} from "lucide-react";
import { type ReactNode, useState } from "react";
import type { HubStandalone } from "../../../../lib/device-management/hub/endpoints";
import {
	deviceName,
	keysLocked,
} from "../../../../lib/device-management/model/device-view";
import type { DeviceViewModel } from "../../../../lib/device-management/model/types";
import type {
	ReleaseFacts,
	ReleaseTarget,
	VerifiedRelease,
} from "../../../../lib/device-package";
import { enumLabel } from "../copy/enum-labels";
import { useAreaTime } from "../primitives/area-context";
import { Block } from "../primitives/block";
import { dayText } from "../primitives/day";
import { DvButton } from "../primitives/dv-button";
import { CellSub, DvTable, Td, Th, Tr } from "../primitives/dv-table";
import { FreshnessStamp } from "../primitives/freshness-stamp";
import { IdRef } from "../primitives/id-ref";
import { InlineResult } from "../primitives/inline-result";
import { KeyValueList, KvRow } from "../primitives/key-value-list";
import { PresenceGlyph } from "../primitives/presence-glyph";
import { StateView } from "../primitives/state-view";
import { StatusChip } from "../primitives/status-chip";
import {
	TONE_SOLID,
	TONE_SURFACE,
	TONE_TEXT,
	type Tone,
	cx,
} from "../primitives/tone";
import { useRouteLink } from "../routing/use-devices-route";
import { stampOf } from "../shell/attention-popover";
import {
	type HubSupportRead,
	type ReleaseTrustRead,
	hubErrorCopy,
	useDeviceRows,
	useDeviceViews,
	useHubSupport,
	useOverlay,
	useReleaseTrust,
} from "../workspace";
import {
	Fingerprint,
	Hint,
	HubReadStamp,
	SectionHead,
	Tech,
	UrlLine,
	useLocale,
} from "./hub-parts";
import {
	type ReleaseVerdict,
	releaseCheckFailure,
	releaseVerdictOf,
	usableRelease,
} from "./hub-view";
import { checkCopy } from "./readiness-list";
import {
	type ShownVerdict,
	isLastDay,
	releaseCheckText,
	releaseFetchCause,
	releaseLeftText,
	verdictSentence,
} from "./release-copy";
import { bytesText, useHubRecord, useReverify } from "./use-hub-facts";

type ReleaseTrustConfig = NonNullable<HubStandalone["release_trust"]>;
/** A verdict about a list whose own dates are known. */
type DatedVerdict = Extract<
	ReleaseVerdict,
	{ kind: "ok" | "ends_soon" | "expired" }
>;

const TARGETS: readonly { target: ReleaseTarget; docker: string | null }[] = [
	{ target: "x86_64-unknown-linux-gnu", docker: "linux/amd64" },
	{ target: "aarch64-unknown-linux-gnu", docker: "linux/arm64" },
	{ target: "aarch64-apple-darwin", docker: null },
	{ target: "x86_64-apple-darwin", docker: null },
];
/** Above this a setup package downloads the agent on the device instead of carrying it. */
const PACKAGE_BINARY_MAX = 268_435_456;
/** A cell without a value shows a dash in the table and drops out of the stacked card. */
const NO_VALUE = "empty:before:content-['–']";
const DEVICES_SHOWN = 6;
const DEVICES_STEP = 20;

/* The verdict: what the release means for the viewer, in one sentence. */

const VERDICT_LOOK: Record<
	ShownVerdict["kind"],
	{ tone?: Tone; icon: LucideIcon }
> = {
	ok: { tone: "good", icon: BadgeCheck },
	ends_soon: { tone: "warning", icon: TriangleAlert },
	expired: { tone: "critical", icon: OctagonX },
	failed: { tone: "critical", icon: OctagonX },
	unfetched: { tone: "warning", icon: TriangleAlert },
	checking: { icon: LoaderCircle },
};

function VerdictLine({ verdict }: Readonly<{ verdict: ShownVerdict }>) {
	const { t } = useTranslation("devices");
	const time = useAreaTime();
	const { tone, icon: Icon } = VERDICT_LOOK[verdict.kind];
	return (
		<p
			role={tone === "critical" ? "alert" : "status"}
			data-verdict={verdict.kind}
			data-tone={tone ?? "neutral"}
			title={verdict.kind === "failed" ? verdict.detail : undefined}
			className={cx(
				"flex items-start gap-2.5 border-b px-4 py-3 text-sm/5 text-foreground",
				tone ? TONE_SURFACE[tone] : "border-hairline bg-surface-sunken",
			)}
		>
			<Icon
				aria-hidden
				className={cx(
					"mt-0.5 size-4 shrink-0",
					tone ? TONE_TEXT[tone] : "animate-spin text-muted-foreground",
				)}
			/>
			<span className="max-w-[88ch] min-w-0 text-pretty">
				{verdictSentence(t, time, verdict)}
			</span>
		</p>
	);
}

/* The current release: version, validity window, trust facts. */

const TRACK_TONE: Record<DatedVerdict["kind"], Tone | undefined> = {
	ok: undefined,
	ends_soon: "warning",
	expired: "critical",
};

function ValidityTrack({ verdict }: Readonly<{ verdict: DatedVerdict }>) {
	const { t } = useTranslation("devices");
	const time = useAreaTime();
	const { facts, kind } = verdict;
	const span = Math.max(1, facts.expires_at - facts.issued_at);
	const elapsed = Math.max(
		0,
		Math.min(1, (time.nowS - facts.issued_at) / span),
	);
	const percent = `${(elapsed * 100).toFixed(1)}%`;
	const dates = {
		from: time.at(facts.issued_at),
		until: time.at(facts.expires_at),
	};
	const tone = TRACK_TONE[kind];
	return (
		<div className="flex w-full max-w-140 min-w-0 flex-col gap-1.5 justify-self-end @max-[1080px]/devices:max-w-none @max-[1080px]/devices:justify-self-stretch">
			<div
				role="img"
				aria-label={
					verdict.kind === "ends_soon" &&
					!isLastDay(facts.expires_at, time.nowS)
						? t("hub.release.window", {
								...dates,
								count: verdict.daysLeft,
								defaultValue_one:
									"Valid from {{from}} to {{until}}; {{count, number}} day left",
								defaultValue_other:
									"Valid from {{from}} to {{until}}; {{count, number}} days left",
							})
						: t(
								"hub.release.windowDates",
								"Valid from {{from}} to {{until}}",
								dates,
							)
				}
				className="relative h-1.5 rounded-[3px] bg-muted"
			>
				<i
					className={cx(
						"absolute inset-y-0 left-0 rounded-l-[3px]",
						tone ? TONE_SOLID[tone] : "bg-border-strong",
					)}
					style={{ width: percent }}
				/>
				<b
					className="absolute -top-1 -ml-px h-3.5 w-0.5 rounded-[1px] bg-foreground"
					style={{ left: percent }}
				/>
			</div>
			<div className="flex justify-between gap-2 text-xs text-muted-foreground">
				<span title={time.abs(facts.issued_at)}>
					{t("hub.release.issued", "Issued {{when}}", { when: dates.from })}
				</span>
				{tone ? (
					<b className={cx("font-medium", TONE_TEXT[tone])}>
						{kind === "expired"
							? t("hub.release.expired", "Expired")
							: releaseLeftText(t, facts.expires_at, time.nowS)}
					</b>
				) : null}
				<span title={time.abs(facts.expires_at)}>
					{kind === "expired"
						? t("hub.release.ranOut", "Ran out {{when}}", {
								when: dates.until,
							})
						: t("hub.release.expires", "Valid until {{when}}", {
								when: dates.until,
							})}
				</span>
			</div>
		</div>
	);
}

function Hero({ verdict }: Readonly<{ verdict: DatedVerdict }>) {
	const { t } = useTranslation("devices");
	const { facts } = verdict;
	return (
		<div className="grid grid-cols-[auto_minmax(0,1fr)] items-center gap-x-8 gap-y-3 border-b border-hairline px-4 py-3.5 @max-[1080px]/devices:grid-cols-1">
			<div className="flex flex-wrap items-baseline gap-x-2.5 gap-y-1">
				<span className="font-mono text-2xl/[30px] font-semibold tracking-[-0.01em] @max-[480px]/devices:text-[21px]/[26px]">
					{facts.release_version}
				</span>
				<span className="text-ui text-muted-foreground">
					{t("hub.release.number", "release number {{sequence}}", {
						sequence: facts.sequence,
					})}
				</span>
				{verdict.kind === "expired" ? (
					<StatusChip tone="critical" icon={OctagonX} className="self-center">
						{t("hub.release.expired", "Expired")}
					</StatusChip>
				) : (
					<StatusChip tone="good" icon={BadgeCheck} className="self-center">
						{t("hub.release.verified", "Verified")}
					</StatusChip>
				)}
			</div>
			<ValidityTrack verdict={verdict} />
		</div>
	);
}

function KvHint({ children }: Readonly<{ children: ReactNode }>) {
	return <Hint className="mt-0.5">{children}</Hint>;
}

/** Older releases a hub still accepts while its minimum is behind the current release: said to the one who can raise it. */
function MinimumBehind({
	minimum,
	sequence,
}: Readonly<{ minimum: number; sequence: number }>) {
	const { t } = useTranslation("devices");
	const below = sequence - 1;
	return (
		<KvHint>
			{below === minimum
				? t(
						"hub.release.minimumBehindOne",
						"For the hub operator: release {{minimum}} is still accepted. Raise the minimum to {{sequence}} so that only the current release is.",
						{ minimum, sequence },
					)
				: t(
						"hub.release.minimumBehind",
						"For the hub operator: releases {{minimum}} to {{below}} are still accepted. Raise the minimum to {{sequence}} so that only the current release is.",
						{ minimum, below, sequence },
					)}
		</KvHint>
	);
}

function ValidRow({
	facts,
	usable,
}: Readonly<{ facts: ReleaseFacts; usable: boolean }>) {
	const { t } = useTranslation("devices");
	const time = useAreaTime();
	return (
		<KvRow label={t("hub.release.valid", "Valid")}>
			<span title={time.abs(facts.issued_at)}>{time.at(facts.issued_at)}</span>
			{" → "}
			<span title={time.abs(facts.expires_at)}>
				{time.at(facts.expires_at)}
			</span>
			<Tech>{`issued_at ${facts.issued_at} · expires_at ${facts.expires_at}`}</Tech>
			{usable ? (
				<KvHint>
					{t(
						"hub.release.validHint",
						"For the hub operator: publish or renew a release before {{date}}. Running devices are not affected.",
						{ date: dayText(time, facts.expires_at) },
					)}
				</KvHint>
			) : null}
		</KvRow>
	);
}

function TrustFacts({
	trust,
	verdict,
	verifiedAt,
}: Readonly<{
	trust: ReleaseTrustConfig;
	verdict: ShownVerdict;
	/** Unix seconds of the verification on this computer. */
	verifiedAt?: number;
}>) {
	const { t } = useTranslation("devices");
	const time = useAreaTime();
	const release = usableRelease(verdict);
	const facts = "facts" in verdict ? verdict.facts : undefined;
	return (
		<KeyValueList>
			<KvRow label={t("hub.release.publishedAt", "Published at")}>
				<UrlLine
					url={trust.manifest_url}
					copyLabel={t("hub.release.copyAddress", "Copy release address")}
				/>
				<KvHint>
					{t(
						"hub.release.publishedHint",
						"Where this app and your devices fetch the signed release list.",
					)}
				</KvHint>
			</KvRow>
			<KvRow label={t("hub.release.keys", "Trusted signing keys")}>
				<span className="flex flex-wrap items-center gap-x-1.5 gap-y-1">
					<span className="tabular-nums">{trust.public_keys.length}</span>
					{trust.public_keys.map((key) => (
						<IdRef
							key={key}
							id={key}
							copyLabel={t("hub.release.copyKey", "Copy signing key")}
						/>
					))}
				</span>
				<KvHint>
					{t(
						"hub.release.keysHint",
						"The hub operator pins these keys. A release signed by any other key is refused.",
					)}
				</KvHint>
			</KvRow>
			{facts ? (
				<KvRow label={t("hub.release.sequence", "Release number")}>
					<span className="tabular-nums">{facts.sequence}</span>
					<Tech>sequence</Tech>
					<KvHint>
						{t(
							"hub.release.sequenceHint",
							"Release numbers only go up. Devices refuse a lower one.",
						)}
					</KvHint>
				</KvRow>
			) : null}
			<KvRow label={t("hub.release.minimum", "Minimum release number")}>
				<span className="tabular-nums">{trust.minimum_sequence}</span>
				<Tech>minimum_sequence</Tech>
				<KvHint>
					{t(
						"hub.release.minimumHint",
						"The lowest release number this hub accepts.",
					)}
				</KvHint>
				{release && release.manifest.sequence > trust.minimum_sequence ? (
					<MinimumBehind
						minimum={trust.minimum_sequence}
						sequence={release.manifest.sequence}
					/>
				) : null}
			</KvRow>
			{facts ? <ValidRow facts={facts} usable={!!release} /> : null}
			{release ? (
				<KvRow label={t("hub.release.verification", "Verification")}>
					<StatusChip tone="good" icon={BadgeCheck} className="mr-1.5">
						{t("hub.release.verified", "Verified")}
					</StatusChip>
					{t(
						"hub.release.verifiedBy",
						"Signed by a trusted key · release number {{sequence}} is at or above the minimum {{minimum}} · within its validity window",
						{
							sequence: release.manifest.sequence,
							minimum: trust.minimum_sequence,
						},
					)}
					{verifiedAt === undefined ? null : (
						<span
							data-provenance=""
							className="ml-1.5 text-xs text-muted-foreground"
						>
							{t(
								"hub.release.checkedHere",
								"(checked on this computer at {{time}})",
								{ time: time.clock(verifiedAt) },
							)}
						</span>
					)}
					<KvHint>
						{t("hub.release.signedBy", "Signing key fingerprint:")}{" "}
						<Fingerprint
							value={release.signerFingerprint}
							copyLabel={t(
								"hub.release.copyFingerprint",
								"Copy signing key fingerprint",
							)}
						/>
					</KvHint>
					<KvHint>
						{t(
							"hub.release.downloadsHint",
							"Downloads are checked against each platform's size and SHA-256 before use.",
						)}
						<Tech>{`state_schema_version ${release.manifest.state_schema_version}`}</Tech>
					</KvHint>
				</KvRow>
			) : null}
		</KeyValueList>
	);
}

/* Your devices: which agent each runs, older ones first. */

type AgentRank = 0 | 1 | 2;

function agentRank(view: DeviceViewModel, current?: string): AgentRank {
	if (!view.agent) return 1;
	return current !== undefined && view.agent.version !== current ? 0 : 2;
}

/** The version cell of a device whose agent version isn't known here: the way to read it, never "Unknown". */
function UnreadVersion({ view }: Readonly<{ view: DeviceViewModel }>) {
	const { t } = useTranslation("devices");
	const overlay = useOverlay();
	if (view.presence.kind === "never")
		return (
			<span className="font-sans font-normal text-muted-foreground">–</span>
		);
	if (keysLocked(view.keys))
		return (
			<DvButton
				size="xs"
				variant="link"
				icon={LockOpen}
				data-act="unlock"
				className="font-sans"
				onClick={() => overlay.openUnlock(view.row.device_id)}
			>
				{t("hub.devices.locked", "Unlock to read")}
			</DvButton>
		);
	return (
		<span className="font-sans font-normal text-muted-foreground">
			{t("hub.devices.notRead", "Not read yet")}
		</span>
	);
}

interface AgentCompare {
	/** The version of the usable release; absent while there is none. */
	current?: string;
	/** The release is still being verified: nothing is said against it yet. */
	pending?: boolean;
}

function DeviceAgentRow({
	view,
	current,
	pending,
}: Readonly<{ view: DeviceViewModel } & AgentCompare>) {
	const { t } = useTranslation("devices");
	const time = useAreaTime();
	const link = useRouteLink();
	const name = deviceName(view.row);
	const { agent, presence } = view;
	let status: ReactNode = null;
	if (agent && current !== undefined && agent.version === current)
		status = (
			<span className={cx("inline-flex items-center gap-1", TONE_TEXT.good)}>
				<CircleCheck aria-hidden className="size-3" />
				{t("hub.devices.current", "On the current release")}
			</span>
		);
	else if (agent && current !== undefined)
		status = (
			<>
				<span
					className={cx("inline-flex items-center gap-1", TONE_TEXT.warning)}
				>
					<ArrowUp aria-hidden className="size-3" />
					{t("hub.devices.available", "{{version}} available", {
						version: current,
					})}
				</span>
				{presence.kind === "offline" && presence.since !== undefined ? (
					<span title={time.abs(presence.since)}>
						{t(
							"hub.devices.offline",
							"Offline since {{when}}, so it can't update yet",
							{ when: time.at(presence.since) },
						)}
					</span>
				) : null}
			</>
		);
	else if (agent && !pending)
		status = (
			<span>
				{t(
					"hub.devices.keeps",
					"Keeps this agent until the hub has a verified release again",
				)}
			</span>
		);
	else if (!agent && presence.kind === "never")
		status = <span>{t("hub.devices.never", "Hasn't checked in yet.")}</span>;
	return (
		<li
			data-device={view.row.device_id}
			className="grid grid-cols-[minmax(0,1fr)_auto] items-baseline gap-x-3 gap-y-0.5 border-t border-hairline py-2 text-ui first:border-t-0 first:pt-0"
		>
			<span className="inline-flex min-w-0 items-center gap-1.5">
				<PresenceGlyph
					kind={presence.kind}
					label={enumLabel(t, "presence", presence.kind)}
				/>
				<a
					{...link({
						screen: "device",
						deviceId: view.row.device_id,
						tab: "settings",
					})}
					title={name}
					className="truncate font-mono font-medium text-foreground hover:underline"
				>
					{name}
				</a>
			</span>
			<span className="text-right font-mono text-[12.5px] font-medium whitespace-nowrap tabular-nums">
				{agent ? agent.version : <UnreadVersion view={view} />}
			</span>
			{status || agent ? (
				<span className="col-span-full flex flex-wrap items-center gap-x-2.5 gap-y-0.5 text-xs text-muted-foreground">
					{status}
					{agent ? <FreshnessStamp {...stampOf(agent.source)} compact /> : null}
				</span>
			) : null}
		</li>
	);
}

/** The list needs `GET /devices`; until it answers this is "not loaded", never "no devices" (R6). */
function DeviceAgents(compare: Readonly<AgentCompare>) {
	const { t } = useTranslation("devices");
	const devices = useDeviceRows();
	if (devices.rows) return <DeviceAgentList {...compare} />;
	return (
		<div data-hub="devices" className="flex min-w-0 flex-col gap-3">
			<SectionHead title={t("hub.devices.title", "Your devices")} />
			{devices.loading ? (
				<StateView
					kind="loading"
					title={t("hub.devices.loading", "Loading your devices")}
				/>
			) : (
				<StateView
					kind="notloaded"
					title={t(
						"hub.devices.notLoaded",
						"Your devices haven't been read: the hub didn't answer.",
					)}
					text={devices.error ? hubErrorCopy(t, devices.error.code) : undefined}
				/>
			)}
		</div>
	);
}

function DeviceAgentList({ current, pending }: Readonly<AgentCompare>) {
	const { t } = useTranslation("devices");
	const link = useRouteLink();
	const views = useDeviceViews();
	const [shown, setShown] = useState(DEVICES_SHOWN);
	const list = views
		.filter((view) => view.row.status !== "revoked")
		.sort(
			(a, b) =>
				agentRank(a, current) - agentRank(b, current) ||
				deviceName(a.row).localeCompare(deviceName(b.row)),
		);
	const outdated = list.filter((view) => agentRank(view, current) === 0).length;
	const visible = list.slice(0, shown);
	const left = list.length - visible.length;
	return (
		<div data-hub="devices" className="flex min-w-0 flex-col gap-3">
			<SectionHead
				title={t("hub.devices.title", "Your devices")}
				note={
					current === undefined
						? t("hub.devices.summaryPlain", "{{total, number}} active", {
								total: list.length,
							})
						: outdated
							? t(
									"hub.devices.summaryOutdated",
									"{{total, number}} active · {{outdated, number}} on an older agent, listed first",
									{ total: list.length, outdated },
								)
							: t(
									"hub.devices.summary",
									"{{total, number}} active · none on an older agent",
									{ total: list.length },
								)
				}
			/>
			{list.length ? (
				<ul className="flex flex-col">
					{visible.map((view) => (
						<DeviceAgentRow
							key={view.row.device_id}
							view={view}
							current={current}
							pending={pending}
						/>
					))}
				</ul>
			) : (
				<StateView
					kind="empty"
					title={t("hub.devices.empty", "No devices on this account yet")}
				/>
			)}
			{left > 0 || shown > DEVICES_SHOWN ? (
				<div className="flex flex-wrap items-center gap-2">
					{left > 0 ? (
						<>
							<DvButton
								size="sm"
								onClick={() => setShown(shown + DEVICES_STEP)}
							>
								{t("hub.devices.more", "Show {{count, number}} more", {
									count: Math.min(DEVICES_STEP, left),
								})}
							</DvButton>
							<span className="text-xs text-muted-foreground">
								{t(
									"hub.devices.shown",
									"{{shown, number}} of {{total, number}} shown",
									{ shown: visible.length, total: list.length },
								)}
							</span>
						</>
					) : null}
					{shown > DEVICES_SHOWN ? (
						<DvButton
							size="sm"
							variant="ghost"
							onClick={() => setShown(DEVICES_SHOWN)}
						>
							{t("hub.devices.fewer", "Show fewer")}
						</DvButton>
					) : null}
					{list.length > DEVICES_SHOWN + DEVICES_STEP ? (
						<DvButton size="sm" variant="ghost" icon={List} asChild>
							<a {...link({ screen: "fleet", view: "devices" })}>
								{t("hub.devices.all", "All devices on Fleet overview")}
							</a>
						</DvButton>
					) : null}
				</div>
			) : null}
			<Hint>
				{t(
					"hub.devices.hint",
					"Revoked devices aren't listed. Each agent version comes from where it was last read.",
				)}
			</Hint>
		</div>
	);
}

/* Platforms of the current release. */

function Platforms({ release }: Readonly<{ release: VerifiedRelease }>) {
	const { t } = useTranslation("devices");
	const locale = useLocale();
	const { manifest } = release;
	const image = manifest.container;
	const labels = {
		platform: t("hub.platforms.platform", "Platform"),
		packages: t("hub.platforms.packages", "In setup packages"),
		size: t("hub.platforms.size", "Size"),
		hash: t("hub.platforms.hash", "SHA-256"),
	};
	return (
		<>
			<SectionHead
				className="px-4 pt-3 pb-2"
				title={t("hub.platforms.title", "Platforms in release {{version}}", {
					version: manifest.release_version,
				})}
				note={
					image
						? t(
								"hub.platforms.countDocker",
								"{{count, number}} of {{total, number}} platforms + Docker image",
								{ count: manifest.artifacts.length, total: TARGETS.length },
							)
						: t(
								"hub.platforms.count",
								"{{count, number}} of {{total, number}} platforms",
								{ count: manifest.artifacts.length, total: TARGETS.length },
							)
				}
			/>
			<DvTable
				label={t("hub.platforms.title", "Platforms in release {{version}}", {
					version: manifest.release_version,
				})}
				cols={["27%", "37%", "12%", "24%"]}
				head={
					<tr>
						<Th>{labels.platform}</Th>
						<Th>{labels.packages}</Th>
						<Th numeric>{labels.size}</Th>
						<Th>{labels.hash}</Th>
					</tr>
				}
			>
				{TARGETS.map(({ target, docker }) => {
					const artifact = manifest.artifacts.find(
						(item) => item.target === target,
					);
					const name = (
						<Td label={labels.platform} kind="name">
							<b className="font-semibold">{enumLabel(t, "target", target)}</b>
							<CellSub>
								{enumLabel(t, "targetShort", target)}
								<Tech>{target}</Tech>
							</CellSub>
						</Td>
					);
					if (!artifact)
						return (
							<Tr key={target} dim data-target={target}>
								{name}
								<Td label={labels.packages}>
									{t("hub.platforms.missing", "Not in this release")}
									<CellSub>
										{t(
											"hub.platforms.missingHint",
											"Devices of this kind can't be set up until a release includes them.",
										)}
									</CellSub>
								</Td>
								<Td label={labels.size} kind="num" className={NO_VALUE} />
								<Td label={labels.hash} className={NO_VALUE} />
							</Tr>
						);
					const composed =
						!!image && !!docker && image.platforms.includes(docker);
					return (
						<Tr key={target} data-target={target}>
							{name}
							<Td label={labels.packages}>
								{composed
									? t("hub.platforms.both", "Run directly or Docker Compose")
									: t("hub.platforms.direct", "Run directly")}
								<CellSub>
									{artifact.size > PACKAGE_BINARY_MAX
										? t(
												"hub.platforms.downloaded",
												"Over 256 MiB, so the package downloads it on the device and checks its hash there.",
											)
										: t(
												"hub.platforms.included",
												"Included in the setup package.",
											)}
								</CellSub>
							</Td>
							<Td label={labels.size} kind="num">
								{bytesText(locale, artifact.size)}
							</Td>
							<Td label={labels.hash}>
								<IdRef
									id={artifact.sha256}
									copyLabel={t("hub.platforms.copyHash", "Copy SHA-256")}
								/>
							</Td>
						</Tr>
					);
				})}
			</DvTable>
			{image ? (
				<div className="border-t border-hairline px-4 py-3">
					<KeyValueList>
						<KvRow label={t("hub.platforms.image", "Docker image")}>
							<UrlLine
								url={image.image}
								copyLabel={t("hub.platforms.copyImage", "Copy Docker image")}
							/>
							<KvHint>
								{t(
									"hub.platforms.imageHint",
									"Pinned by its digest. Docker Compose setups pull exactly this image.",
								)}
							</KvHint>
						</KvRow>
					</KeyValueList>
				</div>
			) : null}
		</>
	);
}

/* States without a verified release. */

function MissingRelease() {
	const { t } = useTranslation("devices");
	const none = (text: string) => (
		<span className="text-muted-foreground">{text}</span>
	);
	return (
		<div className="flex flex-col gap-3 px-4 py-3">
			<StateView
				kind="gate"
				icon={PackageX}
				title={t(
					"hub.release.missing.title",
					"This hub has no signed agent releases, so agents can't be updated remotely.",
				)}
				text={t(
					"hub.release.missing.text",
					"New devices can't be set up either: every setup package carries a verified agent release. Devices already set up keep running the agent they have.",
				)}
			/>
			<div
				data-fix=""
				className="flex flex-col gap-1 rounded-sm border border-hairline bg-surface-sunken px-2.5 py-2 text-ink-2"
			>
				<p className="text-ui">
					<b className="font-semibold text-foreground">
						{t("hub.checks.fixFor", "Fix, for the hub operator:")}
					</b>{" "}
					{checkCopy(t, "release").fix}
				</p>
				<p className="text-xs text-muted-foreground">
					{t(
						"hub.release.missing.then",
						"Then select Check again under Hub status checks. Nothing needs to happen on your devices first.",
					)}
				</p>
			</div>
			<KeyValueList>
				<KvRow label={t("hub.release.publishedAt", "Published at")}>
					{none(t("hub.release.missing.address", "Not configured on this hub"))}
				</KvRow>
				<KvRow label={t("hub.release.keys", "Trusted signing keys")}>
					{none(t("hub.release.missing.keys", "None configured"))}
				</KvRow>
				<KvRow label={t("hub.release.minimum", "Minimum release number")}>
					{none(t("hub.release.missing.minimum", "Not configured"))}
				</KvRow>
			</KeyValueList>
		</div>
	);
}

/** What "Verify again" came back with: verified, refused by a check, or not fetched. */
function VerifyOutcome({
	verify,
	minimum,
}: Readonly<{ verify: ReturnType<typeof useReverify>; minimum: number }>) {
	const { t } = useTranslation("devices");
	const time = useAreaTime();
	const { outcome } = verify;
	if (!outcome) return null;
	const at = time.clock(outcome.at);
	if (outcome.ok)
		return (
			<InlineResult tone="good" onDismiss={verify.dismiss}>
				{t(
					"hub.release.result.verified",
					"Verified again at {{time}}: signed by a trusted key, release number {{sequence}} is at or above the minimum {{minimum}}, and valid until {{until}}.",
					{
						time: at,
						sequence: outcome.data.manifest.sequence,
						minimum,
						until: time.at(outcome.data.manifest.expires_at),
					},
				)}
			</InlineResult>
		);
	const refused = releaseCheckFailure(outcome.error);
	return refused ? (
		<InlineResult tone="critical" onDismiss={verify.dismiss}>
			<span title={refused.detail}>
				{t(
					"hub.release.result.refused",
					"Checked at {{time}}. The release failed a check: {{check}}.",
					{ time: at, check: releaseCheckText(t, refused.check) },
				)}
			</span>
		</InlineResult>
	) : (
		<InlineResult tone="warning" onDismiss={verify.dismiss}>
			{t(
				"hub.release.result.failed",
				"Couldn't verify at {{time}}: {{cause}}",
				{
					time: at,
					cause: releaseFetchCause(t, outcome.error),
				},
			)}
		</InlineResult>
	);
}

/** A fresh fetch and verification of the release list, with its outcome next to the button (R9). */
function VerifyAgain({ minimum }: Readonly<{ minimum: number }>) {
	const { t } = useTranslation("devices");
	const verify = useReverify();
	return (
		<>
			<div className="flex flex-wrap items-center gap-2">
				<DvButton
					size="sm"
					icon={RefreshCw}
					busy={verify.busy}
					onClick={() => {
						void verify.run();
					}}
				>
					{verify.busy
						? t("hub.release.verifyingShort", "Verifying…")
						: t("hub.release.verifyAgain", "Verify again")}
				</DvButton>
			</div>
			<VerifyOutcome verify={verify} minimum={minimum} />
		</>
	);
}

function ConfiguredRelease({
	trust,
	verdict,
	verifiedAt,
}: Readonly<{
	trust: ReleaseTrustConfig;
	verdict: ShownVerdict;
	/** Unix seconds of the verification the verdict rests on. */
	verifiedAt?: number;
}>) {
	const release = usableRelease(verdict);
	return (
		<>
			<VerdictLine verdict={verdict} />
			{verdict.kind === "ok" ||
			verdict.kind === "ends_soon" ||
			verdict.kind === "expired" ? (
				<Hero verdict={verdict} />
			) : null}
			<div className="grid grid-cols-[minmax(0,1.25fr)_minmax(0,1fr)] border-b border-hairline @max-[900px]/devices:grid-cols-1">
				<div className="flex min-w-0 flex-col gap-3 px-4 py-3">
					<TrustFacts trust={trust} verdict={verdict} verifiedAt={verifiedAt} />
					<VerifyAgain minimum={trust.minimum_sequence} />
				</div>
				<div className="min-w-0 border-l border-hairline px-4 py-3 @max-[900px]/devices:border-t @max-[900px]/devices:border-l-0">
					<DeviceAgents
						current={release?.manifest.release_version}
						pending={verdict.kind === "checking"}
					/>
				</div>
			</div>
			{release ? <Platforms release={release} /> : null}
		</>
	);
}

function ReleaseBody({
	record,
	verdict,
	read,
	hub,
}: Readonly<{
	record: HubStandalone | undefined;
	verdict: ReleaseVerdict;
	read: ReleaseTrustRead;
	hub: HubSupportRead;
}>) {
	const { t } = useTranslation("devices");
	if (hub.support.state === "checking")
		return (
			<div className="px-4 py-3">
				<StateView
					kind="loading"
					title={t("hub.release.loading", "Loading the release")}
				/>
			</div>
		);
	const trust = record?.release_trust;
	if (!record || verdict.kind === "waiting")
		return (
			<div className="px-4 py-3">
				<StateView
					kind="notloaded"
					title={t(
						"hub.release.notLoaded",
						"The release settings haven't been read: the hub didn't answer.",
					)}
				/>
			</div>
		);
	if (!trust || verdict.kind === "missing")
		return (
			<>
				<MissingRelease />
				<div className="border-t border-hairline px-4 py-3">
					<DeviceAgents />
				</div>
			</>
		);
	return (
		<ConfiguredRelease
			trust={trust}
			verdict={verdict}
			verifiedAt={read.freshness.at ?? read.freshness.dataFrom}
		/>
	);
}

/** SPEC §5.10 block 4: what the hub's agent release means right now, where releases are published and which keys sign them. */
export function ReleaseTrust() {
	const { t } = useTranslation("devices");
	const time = useAreaTime();
	const hub = useHubSupport();
	const { record } = useHubRecord();
	const read = useReleaseTrust();
	const verdict = releaseVerdictOf(record, read, time.nowS);
	// The block shows the list this computer verified, or else only the hub's settings.
	const shown = "release" in verdict && !!verdict.release;
	return (
		<Block
			id="releases"
			icon={verdict.kind === "missing" ? PackageX : PackageCheck}
			title={t("hub.release.title", "Signed agent releases")}
			stamp={
				<HubReadStamp
					freshness={shown ? read.freshness : hub.freshness}
					loading={hub.support.state === "checking"}
				/>
			}
			flush
			className="scroll-mt-4"
			foot={
				record?.release_trust ? (
					<span>
						{t(
							"hub.release.foot",
							"The hub names where releases are published and which keys sign them. This app verifies every release itself before it builds a setup package or updates an agent.",
						)}
					</span>
				) : undefined
			}
		>
			<ReleaseBody record={record} verdict={verdict} read={read} hub={hub} />
		</Block>
	);
}
