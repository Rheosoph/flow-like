"use client";

import { useTranslation } from "@flow-like/locales";
import {
	ArrowUp,
	BadgeCheck,
	CircleCheck,
	List,
	OctagonX,
	PackageCheck,
	PackageX,
	RefreshCw,
	TriangleAlert,
} from "lucide-react";
import { type ReactNode, useState } from "react";
import type {
	HubError,
	HubStandalone,
} from "../../../../lib/device-management/hub/endpoints";
import { deviceName } from "../../../../lib/device-management/model/device-view";
import type {
	DeviceViewModel,
	HubErrorCode,
} from "../../../../lib/device-management/model/types";
import type {
	ReleaseTarget,
	StandaloneRelease,
	VerifiedRelease,
} from "../../../../lib/device-package";
import { enumLabel } from "../copy/enum-labels";
import { type DevicesT, useAreaTime } from "../primitives/area-context";
import { Block } from "../primitives/block";
import { DvButton } from "../primitives/dv-button";
import { CellSub, DvTable, Td, Th, Tr } from "../primitives/dv-table";
import { FreshnessStamp } from "../primitives/freshness-stamp";
import { IdRef } from "../primitives/id-ref";
import { InlineResult } from "../primitives/inline-result";
import { KeyValueList, KvRow } from "../primitives/key-value-list";
import { PresenceGlyph } from "../primitives/presence-glyph";
import { StateView } from "../primitives/state-view";
import { StatusChip } from "../primitives/status-chip";
import { TONE_TEXT, cx } from "../primitives/tone";
import { useRouteLink } from "../routing/use-devices-route";
import { stampOf } from "../shell/attention-popover";
import {
	type HubSupportRead,
	type ReleaseTrustRead,
	hubErrorCopy,
	useDeviceRows,
	useDeviceViews,
	useHubSupport,
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
import { checkCopy } from "./readiness-list";
import { bytesText, useHubRecord, useReverify } from "./use-hub-facts";

type ReleaseTrustConfig = NonNullable<HubStandalone["release_trust"]>;

const TARGETS: readonly { target: ReleaseTarget; docker: string | null }[] = [
	{ target: "x86_64-unknown-linux-gnu", docker: "linux/amd64" },
	{ target: "aarch64-unknown-linux-gnu", docker: "linux/arm64" },
	{ target: "aarch64-apple-darwin", docker: null },
	{ target: "x86_64-apple-darwin", docker: null },
];
/** Above this a setup package downloads the agent on the device instead of carrying it. */
const PACKAGE_BINARY_MAX = 268_435_456;
const DAY = 86_400;
const DEVICES_SHOWN = 6;
const DEVICES_STEP = 20;
const TRANSPORT: ReadonlySet<HubErrorCode> = new Set([
	"network",
	"timeout",
	"server_error",
	"rate_limited",
]);

/** Why a release failed verification, in plain words; the verifier's own sentence stays in the hover. */
function rejectionReason(t: DevicesT, detail: string): string {
	if (/signature/i.test(detail))
		return t(
			"devices:hub.release.reason.signature",
			"its signature doesn't match a key the hub operator pinned",
		);
	if (/expired/i.test(detail))
		return t("devices:hub.release.reason.expired", "it has expired");
	if (/older than the configured minimum/i.test(detail))
		return t(
			"devices:hub.release.reason.sequence",
			"it's older than the hub's minimum release",
		);
	return t("devices:hub.release.reason.invalid", "it isn't a valid release");
}

interface ReleaseFailure {
	/** The release list didn't arrive, as opposed to arriving and failing verification. */
	transport: boolean;
	text: string;
	detail?: string;
}

function failureOf(t: DevicesT, error: HubError): ReleaseFailure {
	if (TRANSPORT.has(error.code))
		return { transport: true, text: hubErrorCopy(t, error.code) };
	const detail =
		error.cause instanceof Error ? error.cause.message : error.message;
	return { transport: false, text: rejectionReason(t, detail), detail };
}

/** Days left of a validity window, never negative. */
const daysLeft = (expiresAt: number, nowS: number) =>
	Math.max(0, Math.ceil((expiresAt - nowS) / DAY));

/* The current release: version, validity window, trust facts. */

function ValidityTrack({
	manifest,
}: Readonly<{ manifest: StandaloneRelease }>) {
	const { t } = useTranslation("devices");
	const time = useAreaTime();
	const span = Math.max(1, manifest.expires_at - manifest.issued_at);
	const elapsed = Math.max(
		0,
		Math.min(1, (time.nowS - manifest.issued_at) / span),
	);
	const left = daysLeft(manifest.expires_at, time.nowS);
	const percent = `${(elapsed * 100).toFixed(1)}%`;
	const expired = manifest.expires_at <= time.nowS;
	return (
		<div className="flex w-full max-w-[560px] min-w-0 flex-col gap-1.5 justify-self-end @max-[1080px]/devices:max-w-none @max-[1080px]/devices:justify-self-stretch">
			<div
				role="img"
				aria-label={t(
					"hub.release.window",
					"Valid from {{from}} to {{until}}; {{count, number}} days left",
					{
						from: time.at(manifest.issued_at),
						until: time.at(manifest.expires_at),
						count: left,
					},
				)}
				className="relative h-1.5 rounded-[3px] bg-muted"
			>
				<i
					className="absolute inset-y-0 left-0 rounded-l-[3px] bg-border-strong"
					style={{ width: percent }}
				/>
				<b
					className="absolute -top-1 -ml-px h-3.5 w-0.5 rounded-[1px] bg-foreground"
					style={{ left: percent }}
				/>
			</div>
			<div className="flex justify-between gap-2 text-xs text-muted-foreground">
				<span title={time.abs(manifest.issued_at)}>
					{t("hub.release.issued", "Issued {{when}}", {
						when: time.at(manifest.issued_at),
					})}
				</span>
				<b
					className={cx(
						"font-medium",
						expired ? TONE_TEXT.critical : "text-foreground",
					)}
				>
					{expired
						? t("hub.release.expired", "Expired")
						: t("hub.release.daysLeft", {
								count: left,
								defaultValue_one: "{{count, number}} day left",
								defaultValue_other: "{{count, number}} days left",
							})}
				</b>
				<span title={time.abs(manifest.expires_at)}>
					{t("hub.release.expires", "Expires {{when}}", {
						when: time.at(manifest.expires_at),
					})}
				</span>
			</div>
		</div>
	);
}

function Hero({ release }: Readonly<{ release: VerifiedRelease }>) {
	const { t } = useTranslation("devices");
	const { manifest } = release;
	return (
		<div className="grid grid-cols-[auto_minmax(0,1fr)] items-center gap-x-8 gap-y-3 border-b border-hairline px-4 py-3.5 @max-[1080px]/devices:grid-cols-1">
			<div className="flex flex-wrap items-baseline gap-x-2.5 gap-y-1">
				<span className="font-mono text-2xl/[30px] font-semibold tracking-[-0.01em] @max-[480px]/devices:text-[21px]/[26px]">
					{manifest.release_version}
				</span>
				<span className="text-ui text-muted-foreground">
					{t("hub.release.number", "release #{{sequence}}", {
						sequence: manifest.sequence,
					})}
				</span>
				<StatusChip tone="good" icon={BadgeCheck} className="self-center">
					{t("hub.release.verified", "Verified")}
				</StatusChip>
			</div>
			<ValidityTrack manifest={manifest} />
		</div>
	);
}

function KvHint({ children }: Readonly<{ children: ReactNode }>) {
	return <Hint className="mt-0.5">{children}</Hint>;
}

function TrustFacts({
	trust,
	release,
	verifiedAt,
}: Readonly<{
	trust: ReleaseTrustConfig;
	release?: VerifiedRelease;
	/** Unix seconds of the verification on this computer. */
	verifiedAt?: number;
}>) {
	const { t } = useTranslation("devices");
	const time = useAreaTime();
	const manifest = release?.manifest;
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
			<KvRow label={t("hub.release.minimum", "Minimum release number")}>
				<span className="tabular-nums">{trust.minimum_sequence}</span>
				<Tech>minimum_sequence</Tech>
				<KvHint>
					{t(
						"hub.release.minimumHint",
						"Releases numbered below {{minimum}} are refused. Each setup package also raises its device's own minimum to the release it installs, so no device can be moved back to an older agent.",
						{ minimum: trust.minimum_sequence },
					)}
				</KvHint>
			</KvRow>
			{manifest ? (
				<KvRow label={t("hub.release.valid", "Valid")}>
					<span title={time.abs(manifest.issued_at)}>
						{time.at(manifest.issued_at)}
					</span>
					{" → "}
					<span title={time.abs(manifest.expires_at)}>
						{time.at(manifest.expires_at)}
					</span>{" "}
					<span className="text-muted-foreground">
						{t("hub.release.validMax", "· at most 30 days")}
					</span>
					<KvHint>
						{t(
							"hub.release.validHint",
							"After it expires, setup can't create packages and agents refuse it until the hub publishes a new release.",
						)}
					</KvHint>
				</KvRow>
			) : null}
			{release && manifest ? (
				<KvRow label={t("hub.release.verification", "Verification")}>
					<StatusChip tone="good" icon={BadgeCheck} className="mr-1.5">
						{t("hub.release.verified", "Verified")}
					</StatusChip>
					{t(
						"hub.release.verifiedBy",
						"Signed by a trusted key · release #{{sequence}} is at or above {{minimum}} · within its validity window",
						{
							sequence: manifest.sequence,
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
						<Tech>{`state_schema_version ${manifest.state_schema_version}`}</Tech>
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

function DeviceAgentRow({
	view,
	current,
}: Readonly<{ view: DeviceViewModel; current?: string }>) {
	const { t } = useTranslation("devices");
	const time = useAreaTime();
	const link = useRouteLink();
	const name = deviceName(view.row);
	const { agent, presence } = view;
	const locked = view.keys.state !== "unlocked";
	let status: ReactNode;
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
	else if (agent)
		status = (
			<span>
				{t(
					"hub.devices.keeps",
					"Keeps this agent until the hub has a verified release again",
				)}
			</span>
		);
	else if (presence.kind === "never")
		status = <span>{t("hub.devices.never", "Hasn't checked in yet.")}</span>;
	else
		status = (
			<span>
				{locked
					? t("hub.devices.locked", "Unlock to read its agent version")
					: t("hub.devices.notRead", "Its agent version hasn't been read yet")}
			</span>
		);
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
				{agent ? (
					agent.version
				) : (
					<span className="font-sans font-normal text-muted-foreground">
						{presence.kind === "never"
							? "–"
							: t("hub.devices.unknown", "Unknown")}
					</span>
				)}
			</span>
			<span className="col-span-full flex flex-wrap items-center gap-x-2.5 gap-y-0.5 text-xs text-muted-foreground">
				{status}
				{agent ? <FreshnessStamp {...stampOf(agent.source)} compact /> : null}
			</span>
		</li>
	);
}

/** The list needs `GET /devices`; until it answers this is "not loaded", never "no devices" (R6). */
function DeviceAgents({ current }: Readonly<{ current?: string }>) {
	const { t } = useTranslation("devices");
	const devices = useDeviceRows();
	if (devices.rows) return <DeviceAgentList current={current} />;
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

function DeviceAgentList({ current }: Readonly<{ current?: string }>) {
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
					outdated
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
								<Td label={labels.size} kind="num">
									–
								</Td>
								<Td label={labels.hash}>–</Td>
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

function FailedRelease({
	failure,
	host,
}: Readonly<{ failure: ReleaseFailure; host: string }>) {
	const { t } = useTranslation("devices");
	return failure.transport ? (
		<StateView
			kind="notloaded"
			icon={TriangleAlert}
			title={t(
				"hub.release.unfetched.title",
				"The signed release list couldn't be fetched.",
			)}
			text={t(
				"hub.release.unfetched.text",
				"{{cause}} Setup and agent updates wait until it can be verified. Devices keep running the agent they have.",
				{ cause: failure.text },
			)}
		/>
	) : (
		<StateView
			kind="error"
			icon={OctagonX}
			title={t(
				"hub.release.rejected.title",
				"The hub's agent release couldn't be verified: {{reason}}.",
				{ reason: failure.text },
			)}
			text={
				<span title={failure.detail}>
					{t(
						"hub.release.rejected.text",
						"Ask the operator of {{host}}. The app won't package or install an agent it can't trace to a key they pinned, so nothing on this computer can work around it.",
						{ host },
					)}
				</span>
			}
		/>
	);
}

/** A fresh fetch and verification of the release list, with its outcome next to the button (R9). */
function VerifyAgain({ minimum }: Readonly<{ minimum: number }>) {
	const { t } = useTranslation("devices");
	const time = useAreaTime();
	const verify = useReverify();
	const { outcome } = verify;
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
			{outcome ? (
				<InlineResult
					tone={outcome.ok ? "good" : "warning"}
					onDismiss={verify.dismiss}
				>
					{outcome.ok
						? t(
								"hub.release.result.verified",
								"Verified again at {{time}}: signed by a trusted key, release #{{sequence}} is at or above {{minimum}} and valid until {{until}}.",
								{
									time: time.clock(outcome.at),
									sequence: outcome.data.manifest.sequence,
									minimum,
									until: time.at(outcome.data.manifest.expires_at),
								},
							)
						: t(
								"hub.release.result.failed",
								"Couldn't verify at {{time}}: {{cause}}",
								{
									time: time.clock(outcome.at),
									cause: failureOf(t, outcome.error).text,
								},
							)}
				</InlineResult>
			) : null}
		</>
	);
}

/** Why there is no verified release to show yet: still verifying, not fetched, or refused. */
function Unverified({
	read,
	host,
}: Readonly<{ read: ReleaseTrustRead; host: string }>) {
	const { t } = useTranslation("devices");
	if (read.error)
		return <FailedRelease failure={failureOf(t, read.error)} host={host} />;
	return (
		<StateView
			kind="loading"
			title={t("hub.release.verifying", "Verifying the release…")}
		/>
	);
}

function ConfiguredRelease({
	trust,
	read,
	host,
}: Readonly<{
	trust: ReleaseTrustConfig;
	read: ReleaseTrustRead;
	host: string;
}>) {
	const release = read.data;
	return (
		<>
			{release ? <Hero release={release} /> : null}
			<div className="grid grid-cols-[minmax(0,1.25fr)_minmax(0,1fr)] border-b border-hairline @max-[900px]/devices:grid-cols-1">
				<div className="flex min-w-0 flex-col gap-3 px-4 py-3">
					{release ? null : <Unverified read={read} host={host} />}
					<TrustFacts
						trust={trust}
						release={release}
						verifiedAt={read.freshness.at ?? read.freshness.dataFrom}
					/>
					<VerifyAgain minimum={trust.minimum_sequence} />
				</div>
				<div className="min-w-0 border-l border-hairline px-4 py-3 @max-[900px]/devices:border-t @max-[900px]/devices:border-l-0">
					<DeviceAgents current={release?.manifest.release_version} />
				</div>
			</div>
			{release ? <Platforms release={release} /> : null}
		</>
	);
}

function ReleaseBody({
	record,
	read,
	hub,
}: Readonly<{
	record: HubStandalone | undefined;
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
	if (!record)
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
	const trust = record.release_trust;
	if (trust)
		return <ConfiguredRelease trust={trust} read={read} host={hub.host} />;
	return (
		<>
			<MissingRelease />
			<div className="border-t border-hairline px-4 py-3">
				<DeviceAgents />
			</div>
		</>
	);
}

/** SPEC §5.10 block 4: where releases are published, which keys sign them, and the current verified release. */
export function ReleaseTrust() {
	const { t } = useTranslation("devices");
	const hub = useHubSupport();
	const { record } = useHubRecord();
	const read = useReleaseTrust();
	const trust = record ? record.release_trust : undefined;
	const missing = !!record && !trust;
	return (
		<Block
			id="releases"
			icon={missing ? PackageX : PackageCheck}
			title={t("hub.release.title", "Signed agent releases")}
			stamp={
				<HubReadStamp
					freshness={trust && read.data ? read.freshness : hub.freshness}
					loading={hub.support.state === "checking"}
				/>
			}
			flush
			className="scroll-mt-4"
			foot={
				trust ? (
					<span>
						{t(
							"hub.release.foot",
							"The hub names where releases are published and which keys sign them. This app verifies every release itself before it builds a setup package or updates an agent.",
						)}
					</span>
				) : undefined
			}
		>
			<ReleaseBody record={record} read={read} hub={hub} />
		</Block>
	);
}
