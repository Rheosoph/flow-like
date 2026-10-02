"use client";

import { Trans, useTranslation } from "@flow-like/locales";
import {
	BadgeCheck,
	Hourglass,
	Laptop,
	LoaderCircle,
	OctagonX,
	PackageCheck,
	RefreshCw,
	Server,
	ShieldCheck,
	TriangleAlert,
} from "lucide-react";
import { type Ref, useState } from "react";
import type { GateFailure } from "../../../../../lib/device-management/model/types";
import { enumLabel } from "../../copy/enum-labels";
import { gateCopy } from "../../copy/gate-copy";
import { HubLimitsUsage, ReadinessList } from "../../hub/readiness-list";
import { type DevicesT, useAreaTime } from "../../primitives/area-context";
import { Banner } from "../../primitives/banner";
import { Block } from "../../primitives/block";
import { DvButton } from "../../primitives/dv-button";
import { FreshnessStamp } from "../../primitives/freshness-stamp";
import { InlineResult } from "../../primitives/inline-result";
import { KeyValueList, KvRow } from "../../primitives/key-value-list";
import { StateView } from "../../primitives/state-view";
import { StatusChip } from "../../primitives/status-chip";
import { useCopy } from "../../primitives/use-copy";
import { WizardStepHeader } from "../../primitives/wizard";
import { useDeviceWorkspace, useLocalSummary } from "../../workspace";
import { useSetup } from "../setup-context";
import { KeyFingerprint, Mono, TechnicalDetails } from "../setup-parts";
import { STEP_COUNT } from "../setup-state";
import type { ReleaseCheck, SetupChecks } from "../use-setup-checks";

/** Why a release failed verification, in the words of IA §6.2 N5; the verifier's sentence stays behind "Details". */
export function releaseReason(t: DevicesT, detail: string): string {
	if (/signature/i.test(detail))
		return t(
			"devices:setup.check.reason.signature",
			"the signature doesn't match",
		);
	if (/expired/i.test(detail))
		return t("devices:setup.check.reason.expired", "it has expired");
	if (/older than the configured minimum/i.test(detail))
		return t(
			"devices:setup.check.reason.sequence",
			"it's older than the hub's minimum release",
		);
	if (/digest|size/i.test(detail))
		return t(
			"devices:setup.check.reason.digest",
			"its size or checksum doesn't match",
		);
	return t("devices:setup.check.reason.invalid", "it isn't a valid release");
}

/** Machine-oriented on purpose (SPEC §3.10): what a hub operator needs, never translated. */
function diagnosticsOf(host: string, checks: SetupChecks): string {
	const { readiness, release } = checks;
	const at =
		readiness.state === "loaded"
			? new Date(readiness.checkedAt).toISOString()
			: new Date().toISOString();
	const rows =
		readiness.state === "loaded"
			? readiness.data.checks.map(
					(check) =>
						`${check.id}: ${check.ready ? "ready" : "NOT READY"} · ${check.message}`,
				)
			: [`readiness: ${readiness.state}`];
	const releaseLine =
		release.state === "verified"
			? `release: ${release.release.manifest.release_version} (#${release.release.manifest.sequence}) · verified`
			: release.state === "rejected"
				? `release: rejected · ${release.detail}`
				: `release: ${release.state}`;
	const trust = checks.config
		? [
				`manifest: ${checks.config.manifestUrl}`,
				`minimum sequence: ${checks.config.minimumSequence}`,
			]
		: [];
	return [
		`Hub readiness · ${host} · ${at}`,
		...rows,
		releaseLine,
		...trust,
	].join("\n");
}

function HubActions({ diagnostics }: Readonly<{ diagnostics: string }>) {
	const { t } = useTranslation("devices");
	const { leaveLink } = useSetup();
	const { copied, copy } = useCopy();
	return (
		<>
			<DvButton size="sm" icon={Server} asChild>
				<a {...leaveLink({ screen: "hub" })}>
					{t("setup.check.openHub", "Open hub status")}
				</a>
			</DvButton>
			<DvButton
				size="sm"
				variant="ghost"
				onClick={() => {
					void copy(diagnostics);
				}}
			>
				{copied
					? t("setup.details.copied", "Copied")
					: t("setup.details.copy", "Copy diagnostics")}
			</DvButton>
		</>
	);
}

/** The hub's limits: unused setup packages, packages per day, or devices. */
function LimitBanner({ gate }: Readonly<{ gate: GateFailure }>) {
	const { t } = useTranslation("devices");
	const setup = useSetup();
	const { code, params } = gate.copy;
	const title = gateCopy(t, gate).title;
	const pendingLink = (
		<DvButton size="sm" icon={Hourglass} asChild>
			<a {...setup.leaveLink({ screen: "fleet", view: "devices" })}>
				{t("setup.check.openPending", "Open Pending setups")}
			</a>
		</DvButton>
	);
	if (code === "pending_setup_limit_reached" && params?.period === "day")
		return (
			<Banner tone="warning" title={title}>
				{t(
					"setup.check.limit.daily",
					"The limit counts packages from the last 24 hours, cancelled ones too. Nothing on your side needs fixing; try again later.",
				)}
			</Banner>
		);
	if (code === "pending_setup_limit_reached")
		return (
			<Banner tone="warning" title={title} actions={pendingLink}>
				{t(
					"setup.check.limit.pending",
					"Cancel one in Pending setups to continue, or start one of those devices, then check again here. Cancelling frees its slot at once.",
				)}
			</Banner>
		);
	if (code === "device_limit_reached")
		return (
			<Banner tone="warning" title={title} actions={pendingLink}>
				{t(
					"setup.check.limit.devices",
					"Unused setup packages count too. Revoke a device you no longer use, or cancel a pending setup, then check again here.",
				)}
			</Banner>
		);
	return <Banner tone="warning" title={title} />;
}

/** The one page-level reason step 0 can't continue (hub, release or limits), or nothing. */
function CheckBanner() {
	const { t } = useTranslation("devices");
	const setup = useSetup();
	const { checks, host } = setup;
	const { readiness, release, gate } = checks;
	const diagnostics = diagnosticsOf(host, checks);

	if (readiness.state === "failed")
		return (
			<Banner
				tone="warning"
				title={t("setup.check.failed.title", "The hub checks couldn't finish.")}
				actions={
					<DvButton
						size="sm"
						icon={RefreshCw}
						onClick={() => {
							void checks.recheck();
						}}
					>
						{t("setup.check.again", "Check again")}
					</DvButton>
				}
			>
				{t(
					"setup.check.failed.text",
					"Check your connection and that the hub is up to date, then check again. Nothing was created.",
				)}
			</Banner>
		);
	if (readiness.state === "loaded" && !readiness.data.ready) {
		const failing = readiness.data.checks.find((check) => !check.ready);
		return (
			<Banner
				tone="warning"
				title={t(
					"setup.check.notReady.title",
					"Ask your hub operator. Only they can fix this.",
				)}
				actions={<HubActions diagnostics={diagnostics} />}
			>
				{t(
					"setup.check.notReady.text",
					"Send them the diagnostics, or open Hub status to see what {{check}} needs. Then check again here.",
					{
						check: failing
							? enumLabel(t, "readinessCheck", failing.id)
							: t("setup.check.notReady.theHub", "the hub"),
					},
				)}
			</Banner>
		);
	}
	if (release.state === "missing")
		return (
			<Banner
				tone="warning"
				title={t(
					"setup.check.noRelease.title",
					"This hub has no signed agent releases.",
				)}
				actions={<HubActions diagnostics={diagnostics} />}
			>
				{t(
					"setup.check.noRelease.text",
					"Without them no setup package can be built. Ask your hub operator to publish signed releases.",
				)}
			</Banner>
		);
	if (release.state === "rejected")
		return (
			<Banner
				tone="critical"
				title={t(
					"setup.check.rejected.title",
					"The hub's agent release couldn't be verified: {{reason}}.",
					{ reason: releaseReason(t, release.detail) },
				)}
				actions={<HubActions diagnostics={diagnostics} />}
			>
				{t(
					"setup.check.rejected.text",
					"Ask your hub operator. The app won't package an agent it can't trace to a key they pinned, so nothing on this computer can work around it.",
				)}
			</Banner>
		);
	if (!gate.ok && readiness.state === "loaded")
		return <LimitBanner gate={gate} />;
	return null;
}

function ReleaseChip({ release }: Readonly<{ release: ReleaseCheck }>) {
	const { t } = useTranslation("devices");
	if (release.state === "verified")
		return (
			<StatusChip tone="good" icon={BadgeCheck}>
				{enumLabel(t, "releaseVerification", "verified")}
			</StatusChip>
		);
	if (release.state === "rejected")
		return (
			<StatusChip tone="critical" icon={OctagonX}>
				{enumLabel(t, "releaseVerification", "rejected", {
					reason: releaseReason(t, release.detail),
				})}
			</StatusChip>
		);
	if (release.state === "checking")
		return (
			<StatusChip tone="info" icon={LoaderCircle} spin>
				{t("setup.check.release.verifying", "Verifying")}
			</StatusChip>
		);
	return null;
}

function ReleaseFacts({
	release,
}: Readonly<{ release: Extract<ReleaseCheck, { state: "verified" }> }>) {
	const { t } = useTranslation("devices");
	const time = useAreaTime();
	const { manifest, signerFingerprint, targets } = release.release;
	const packages = [
		...targets.map((target) => enumLabel(t, "targetShort", target)),
		...(manifest.container
			? [t("setup.check.release.docker", "Docker image for Linux")]
			: []),
	];
	return (
		<KeyValueList>
			<KvRow label={t("setup.check.release.agent", "Agent")}>
				<Trans
					t={t}
					i18nKey="setup.check.release.version"
					defaults="<1>{{version}}</1> · release #{{sequence, number}}"
					values={{
						version: manifest.release_version,
						sequence: manifest.sequence,
					}}
					components={{ 1: <Mono /> }}
				/>
			</KvRow>
			<KvRow
				label={t("setup.check.release.signedBy", "Signed by")}
				provenance={t(
					"setup.check.release.pinned",
					"a key the hub operator pinned",
				)}
			>
				<KeyFingerprint
					value={signerFingerprint}
					copyLabel={t(
						"setup.check.release.copyFingerprint",
						"Copy signing key fingerprint",
					)}
				/>
			</KvRow>
			<KvRow label={t("setup.check.release.valid", "Valid")}>
				{t(
					"setup.check.release.validity",
					"issued {{issued}} · expires {{expires}} ({{left}})",
					{
						issued: time.at(manifest.issued_at),
						expires: time.at(manifest.expires_at),
						left: time.ago(manifest.expires_at),
					},
				)}
			</KvRow>
			<KvRow label={t("setup.check.release.packages", "Packages")}>
				{packages.join(" · ")}
			</KvRow>
			<KvRow
				label={t("setup.check.release.after", "After setup")}
				provenance={t("setup.check.release.guard", "rollback guard")}
			>
				{t(
					"setup.check.release.floor",
					"The device refuses any release older than #{{sequence, number}}.",
					{ sequence: manifest.sequence },
				)}
			</KvRow>
		</KeyValueList>
	);
}

/** No release to show yet: being verified, or not taken before the hub checks pass. */
function ReleasePending({ verifying }: Readonly<{ verifying: boolean }>) {
	const { t } = useTranslation("devices");
	return verifying ? (
		<StateView
			kind="loading"
			title={t("setup.check.release.loading", "Verifying the agent release…")}
		/>
	) : (
		<StateView
			kind="notloaded"
			title={t(
				"setup.check.release.waiting",
				"Verified once the hub checks pass",
			)}
			text={t(
				"setup.check.release.waitingText",
				"The setup only uses a release it verified after the hub said it can register devices.",
			)}
		/>
	);
}

/** The release server gave no answer: nothing is wrong with the release itself. */
function ReleaseUnreachable() {
	const { t } = useTranslation("devices");
	const { checks } = useSetup();
	return (
		<StateView
			kind="error"
			title={t(
				"setup.check.release.unreachable",
				"The agent release couldn't be loaded",
			)}
			text={t(
				"setup.check.release.unreachableText",
				"The release server didn't answer. Check again in a moment; nothing was created.",
			)}
			actions={
				<DvButton
					size="sm"
					icon={RefreshCw}
					onClick={() => {
						void checks.recheck();
					}}
				>
					{t("setup.check.again", "Check again")}
				</DvButton>
			}
		/>
	);
}

/** No trusted release: the hub publishes none, or the one it publishes failed verification. */
function ReleaseUntrusted({ detail }: Readonly<{ detail?: string }>) {
	const { t } = useTranslation("devices");
	const { checks, host } = useSetup();
	if (detail === undefined)
		return (
			<StateView
				kind="unsupported"
				title={t(
					"setup.check.noRelease.title",
					"This hub has no signed agent releases.",
				)}
				text={t(
					"setup.check.noRelease.text",
					"Without them no setup package can be built. Ask your hub operator to publish signed releases.",
				)}
			/>
		);
	return (
		<>
			<StateView
				kind="error"
				title={t("setup.check.release.rejected", "This release is not trusted")}
				text={t(
					"setup.check.release.rejectedText",
					"It can't be traced to a key the hub operator pinned, so no package is built from it.",
				)}
			/>
			<TechnicalDetails
				detail={detail}
				diagnostics={diagnosticsOf(host, checks)}
			/>
		</>
	);
}

function ReleaseBody({ release }: Readonly<{ release: ReleaseCheck }>) {
	switch (release.state) {
		case "verified":
			return <ReleaseFacts release={release} />;
		case "rejected":
			return <ReleaseUntrusted detail={release.detail} />;
		case "missing":
			return <ReleaseUntrusted />;
		case "unreachable":
			return <ReleaseUnreachable />;
		default:
			return <ReleasePending verifying={release.state === "checking"} />;
	}
}

function ReleaseBlock() {
	const { t } = useTranslation("devices");
	const time = useAreaTime();
	const { release } = useSetup().checks;
	return (
		<Block
			icon={PackageCheck}
			title={t("setup.check.release.title", "Agent release")}
			summary={<ReleaseChip release={release} />}
			stamp={
				release.state === "verified" ? (
					<FreshnessStamp
						source="local"
						age="current"
						observedAt={Math.floor(release.verifiedAt / 1000)}
						text={t("setup.check.release.verifiedAgo", "verified {{ago}}", {
							ago: time.ago(Math.floor(release.verifiedAt / 1000)),
						})}
					/>
				) : (
					<FreshnessStamp source="local" age="notloaded" />
				)
			}
		>
			<ReleaseBody release={release} />
		</Block>
	);
}

type KeepResult = "granted" | "refused";

/** Where the new device's keys will live, and whether this browser keeps them. */
function KeysHereBlock() {
	const { t } = useTranslation("devices");
	const time = useAreaTime();
	const workspace = useDeviceWorkspace();
	const local = useLocalSummary();
	const [busy, setBusy] = useState(false);
	const [result, setResult] = useState<{ kind: KeepResult; at: number }>();
	const desktop = local.platform === "desktop";
	const safe = desktop || local.persistence === "persisted";

	const keep = async () => {
		setBusy(true);
		try {
			const persistence = await workspace.local.requestPersistence();
			setResult({
				kind: persistence === "persisted" ? "granted" : "refused",
				at: time.nowS,
			});
		} finally {
			setBusy(false);
		}
	};

	return (
		<Block
			icon={Laptop}
			title={t("setup.check.keys.title", "Keys on this computer")}
			stamp={
				<FreshnessStamp
					source="local"
					age="current"
					text={t("setup.check.keys.stamp", "stored here")}
				/>
			}
			bodyClassName="items-start"
		>
			{safe ? (
				<StatusChip tone="good" icon={ShieldCheck}>
					{desktop
						? t("setup.check.keys.desktop", "Desktop app · kept safely")
						: t("setup.check.keys.web", "Web · kept safely")}
				</StatusChip>
			) : (
				<StatusChip tone="warning" icon={TriangleAlert}>
					{enumLabel(t, "storage", "denied")}
				</StatusChip>
			)}
			<p className="text-ui">
				{desktop
					? t(
							"setup.check.keys.desktopText",
							"The new device's keys are stored in this app on this computer. Other browsers, profiles, hubs and accounts can't see them.",
						)
					: safe
						? t(
								"setup.check.keys.webText",
								"This browser granted persistent storage, so it won't delete the new device's keys on its own.",
							)
						: t(
								"setup.check.keys.riskText",
								"This site hasn't been granted persistent storage. Clearing site data or running low on disk can delete the new device's keys from this browser.",
							)}
			</p>
			{safe ? null : (
				<>
					<p className="text-xs text-muted-foreground">
						{t(
							"setup.check.keys.riskHint",
							"Setup still works. Keep the account backup on and save the key backup file in the Save step.",
						)}
					</p>
					<DvButton
						size="sm"
						icon={ShieldCheck}
						busy={busy}
						onClick={() => {
							void keep();
						}}
					>
						{t("setup.check.keys.keep", "Keep keys safely")}
					</DvButton>
				</>
			)}
			{result ? (
				<InlineResult
					tone={result.kind === "granted" ? "good" : "warning"}
					onDismiss={() => setResult(undefined)}
				>
					{result.kind === "granted"
						? t(
								"setup.check.keys.granted",
								"The browser granted persistent storage at {{time}}. Keys made here are kept safely.",
								{ time: time.clock(result.at) },
							)
						: t(
								"setup.check.keys.refused",
								"The browser didn't grant persistent storage at {{time}}. Keep the account backup on.",
								{ time: time.clock(result.at) },
							)}
				</InlineResult>
			) : null}
		</Block>
	);
}

/** Step 0: can this hub register a device, is the agent genuine, is there room for one more. */
export function CheckStep({
	headingRef,
}: Readonly<{ headingRef?: Ref<HTMLHeadingElement> }>) {
	const { t } = useTranslation("devices");
	const { host } = useSetup();
	return (
		<>
			<WizardStepHeader
				headingRef={headingRef}
				step={1}
				total={STEP_COUNT}
				title={t("setup.check.title", "Check the hub")}
				lede={
					<Trans
						t={t}
						i18nKey="setup.check.lede"
						defaults="Before anything is created, the app checks that <1/> can register devices, that the agent release is genuine, and that you have room for one more device."
						components={{
							1: <span className="whitespace-nowrap">{host}</span>,
						}}
					/>
				}
			/>
			<CheckBanner />
			<ReadinessList compact />
			<ReleaseBlock />
			<div className="grid gap-4 @[640px]/setup:grid-cols-2">
				<HubLimitsUsage compact />
				<KeysHereBlock />
			</div>
		</>
	);
}
