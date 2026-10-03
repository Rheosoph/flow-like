"use client";

import { useTranslation } from "@flow-like/locales";
import {
	CircleCheck,
	CircleDashed,
	ListChecks,
	OctagonX,
	RefreshCw,
	TriangleAlert,
} from "lucide-react";
import { type ReactNode, useEffect, useRef } from "react";
import type { DeviceSetupReadiness } from "../../../../lib/device-management/readiness";
import { type DevicesT, useAreaTime } from "../primitives/area-context";
import { Block } from "../primitives/block";
import { Checklist, type ChecklistItem } from "../primitives/checklist";
import { DvButton } from "../primitives/dv-button";
import { type Gate, GateNotice, GatedAction } from "../primitives/gate-notice";
import { InlineResult } from "../primitives/inline-result";
import { StateView } from "../primitives/state-view";
import { StatusChip } from "../primitives/status-chip";
import { TONE_TEXT, cx } from "../primitives/tone";
import {
	type HubRead,
	type HubSupportRead,
	hubErrorCopy,
	useHubSupport,
	useReadiness,
} from "../workspace";
import { Hint, HubReadStamp, Tech } from "./hub-parts";
import {
	CHECK_IDS,
	type CheckId,
	type CheckSummary,
	type TrackedRefetch,
	summarizeChecks,
	useRecheck,
} from "./use-hub-facts";

export { HubLimitsUsage } from "./limits-usage";

type Check = DeviceSetupReadiness["checks"][number];
type Recheck = TrackedRefetch<DeviceSetupReadiness>;

export interface CheckCopy {
	label: string;
	/** What the check looks at. */
	what: string;
	ready: string;
	failed: string;
	/** What the hub operator changes. */
	fix: string;
	/** What doesn't work until then. */
	blocks: string;
}

/**
 * BG37: the hub's check results in the viewer's language. The hub sends
 * English sentences; the ids are stable, so each one maps to its own copy.
 * `compact` is the setup wizard, where the release is verified in its own block.
 */
export function checkCopy(
	t: DevicesT,
	id: CheckId,
	compact = false,
): CheckCopy {
	const copies: Record<CheckId, () => CheckCopy> = {
		policy: () => ({
			label: t("devices:hub.readiness.policy.label", "Device limits"),
			what: t(
				"devices:hub.readiness.policy.what",
				"Checks that device support is on and its limits are in range.",
			),
			ready: t(
				"devices:hub.readiness.policy.ready",
				"Device setup is turned on.",
			),
			failed: t(
				"devices:hub.readiness.policy.failed",
				"Device setup is off, or its limits are out of range.",
			),
			fix: t(
				"devices:hub.readiness.policy.fix",
				"Turn on device support in the hub settings and keep the limits in range: 1 to 1,000 devices per account, 1 to 100 unused setup packages, and a package lifetime from 1 minute to 24 hours.",
			),
			blocks: t(
				"devices:hub.readiness.policy.blocks",
				"Nobody can set up devices on this hub, and devices already set up can't check in.",
			),
		}),
		signing: () => ({
			label: t("devices:hub.readiness.signing.label", "Hub signing key"),
			what: t(
				"devices:hub.readiness.signing.what",
				"Checks that the hub can sign and verify the short-lived passes devices sign in with.",
			),
			ready: t(
				"devices:hub.readiness.signing.ready",
				"The hub can sign the passes devices sign in with.",
			),
			failed: t(
				"devices:hub.readiness.signing.failed",
				"The hub's signing keys are missing, invalid or don't belong together.",
			),
			fix: t(
				"devices:hub.readiness.signing.fix",
				"Check the hub's backend signing keys: they must be present, valid and belong together. Restart the hub after changing them.",
			),
			blocks: t(
				"devices:hub.readiness.signing.blocks",
				"Devices can't sign in, so check-ins and setup fail.",
			),
		}),
		api: () => ({
			label: t("devices:hub.readiness.api.label", "Public API address"),
			what: t(
				"devices:hub.readiness.api.what",
				"Checks that the hub names the address devices reach it on.",
			),
			ready: t(
				"devices:hub.readiness.api.ready",
				"The public device API address is set.",
			),
			failed: t(
				"devices:hub.readiness.api.failed",
				"The hub has no valid public device API address.",
			),
			fix: t(
				"devices:hub.readiness.api.fix",
				"Set the public device API address in the hub settings. It must be an HTTPS address that devices can reach.",
			),
			blocks: t(
				"devices:hub.readiness.api.blocks",
				"New setup packages can't tell a device where to find the hub.",
			),
		}),
		signaling: () => ({
			label: t("devices:hub.readiness.signaling.label", "Connection service"),
			what: t(
				"devices:hub.readiness.signaling.what",
				"Checks that the hub lists the secure addresses live connections start from.",
			),
			ready: t(
				"devices:hub.readiness.signaling.ready",
				"Secure connection addresses are set. Whether they can be reached is checked when you connect to a device.",
			),
			failed: t(
				"devices:hub.readiness.signaling.failed",
				"The hub needs one to four secure connection addresses.",
			),
			fix: t(
				"devices:hub.readiness.signaling.fix",
				"Configure WSS signaling URLs in the hub settings: one to four wss:// addresses, without a user name, query or fragment.",
			),
			blocks: t(
				"devices:hub.readiness.signaling.blocks",
				"Live connections to devices fail, and new devices can't be set up.",
			),
		}),
		release: () => ({
			label: t("devices:hub.readiness.release.label", "Release settings"),
			what: t(
				"devices:hub.readiness.release.what",
				"Checks that the hub says where agent releases are published and which keys sign them.",
			),
			ready: t(
				"devices:hub.readiness.release.ready",
				"The hub names where releases are published and which keys sign them. This app verifies the release itself; see {{block}}.",
				{
					block: compact
						? t("devices:setup.check.release.title", "Agent release")
						: t("devices:hub.release.title", "Signed agent releases"),
				},
			),
			failed: t(
				"devices:hub.readiness.release.failed",
				"The hub has no release address or no trusted release signing keys.",
			),
			fix: t(
				"devices:hub.readiness.release.fix",
				"Configure a release manifest URL and at least one trusted release signing key in the hub settings.",
			),
			blocks: t(
				"devices:hub.readiness.release.blocks",
				"Agents can't be updated remotely, and new devices can't be set up.",
			),
		}),
		database: () => ({
			label: t("devices:hub.readiness.database.label", "Database migrations"),
			what: t(
				"devices:hub.readiness.database.what",
				"Checks that the hub's device tables exist and answer.",
			),
			ready: t(
				"devices:hub.readiness.database.ready",
				"The hub's device tables are ready.",
			),
			failed: t(
				"devices:hub.readiness.database.failed",
				"The hub's device tables are incomplete or unavailable.",
			),
			fix: t(
				"devices:hub.readiness.database.fix",
				"Apply the hub's device database migrations: device, resource, validation, offline replay, inventory and fleet.",
			),
			blocks: t(
				"devices:hub.readiness.database.blocks",
				"The hub can't store devices, setups or check-ins.",
			),
		}),
	};
	return copies[id]();
}

const isKnown = (id: string): id is CheckId =>
	(CHECK_IDS as readonly string[]).includes(id);

/** The translated result of a known check; the hub's own sentence only for a check this app doesn't know. */
function CheckMessage({
	check,
	compact = false,
}: Readonly<{ check: Check; compact?: boolean }>) {
	const { t } = useTranslation("devices");
	if (isKnown(check.id)) {
		const copy = checkCopy(t, check.id, compact);
		return check.ready ? copy.ready : copy.failed;
	}
	return (
		<>
			<span>{t("hub.checks.said", "Hub's message:")}</span>{" "}
			<q className="text-ink-2">{check.message}</q>
		</>
	);
}

/** A hub check's name in the viewer's words, also for a check this app doesn't know. */
export function checkLabel(t: DevicesT, id: string): string {
	return isKnown(id)
		? checkCopy(t, id).label
		: t("devices:hub.checks.other", "Another hub check");
}

function FixBox({ id }: Readonly<{ id: CheckId }>) {
	const { t } = useTranslation("devices");
	const copy = checkCopy(t, id);
	return (
		<div
			data-fix=""
			className="flex w-full flex-col gap-1 rounded-sm border border-hairline bg-surface-sunken px-2.5 py-2 text-ink-2"
		>
			<p className="text-xs">
				<b className="font-semibold text-foreground">
					{t("hub.checks.fixFor", "Fix, for the hub operator:")}
				</b>{" "}
				{copy.fix}
			</p>
			<p className="text-xs">
				<b className="font-semibold text-foreground">
					{t("hub.checks.untilThen", "Until then:")}
				</b>{" "}
				{copy.blocks}
			</p>
		</div>
	);
}

function ResultWord({ ready }: Readonly<{ ready: boolean }>) {
	const { t } = useTranslation("devices");
	return (
		<span
			className={cx(
				"inline-flex shrink-0 items-center gap-1 text-xs/[18px] font-medium whitespace-nowrap",
				ready ? TONE_TEXT.good : TONE_TEXT.critical,
			)}
		>
			{ready ? null : <OctagonX aria-hidden className="size-3.25" />}
			{ready
				? t("hub.checks.passes", "Passes")
				: t("hub.checks.fails", "Fails")}
		</span>
	);
}

function fullItem(t: DevicesT, check: Check): ChecklistItem {
	const known = isKnown(check.id);
	return {
		id: check.id,
		state: check.ready ? "pass" : "fail",
		label: (
			<span className="flex items-start justify-between gap-3">
				<span className="min-w-0">
					<b className="font-semibold text-foreground">
						{checkLabel(t, check.id)}
					</b>
					<Tech>{check.id}</Tech>
					{known ? (
						<span className="mt-0.5 block text-xs font-normal text-muted-foreground">
							{checkCopy(t, check.id as CheckId).what}
						</span>
					) : null}
				</span>
				<ResultWord ready={check.ready} />
			</span>
		),
		note: <CheckMessage check={check} />,
		...(check.ready || !known
			? {}
			: { fix: <FixBox id={check.id as CheckId} /> }),
	};
}

function compactItem(t: DevicesT, check: Check): ChecklistItem {
	return {
		id: check.id,
		state: check.ready ? "pass" : "fail",
		label: (
			<>
				<b className="font-semibold">{checkLabel(t, check.id)}</b>
				{" · "}
				{check.ready
					? t("devices:hub.checks.ready", "Ready")
					: t("devices:hub.checks.notReady", "Not ready")}
				<Tech>{check.id}</Tech>
			</>
		),
		note: <CheckMessage check={check} compact />,
		...(check.ready
			? {}
			: {
					fix: t(
						"devices:hub.checks.compactFix",
						"Until the hub operator fixes this, no device can be set up on this hub.",
					),
				}),
	};
}

/** The six known checks while the first answer is on its way. */
function waitingItems(t: DevicesT): ChecklistItem[] {
	return CHECK_IDS.map((id) => ({
		id,
		state: "active" as const,
		label: (
			<span className="flex items-start justify-between gap-3">
				<b className="font-semibold">{checkCopy(t, id).label}</b>
				<span className="shrink-0 text-xs/[18px] font-medium whitespace-nowrap">
					{t("devices:hub.checks.checking", "Checking…")}
				</span>
			</span>
		),
	}));
}

function SummaryChip({
	summary,
	compact,
}: Readonly<{ summary: CheckSummary; compact: boolean }>) {
	const { t } = useTranslation("devices");
	const { passed, total } = summary;
	const allPass = passed === total;
	const text = compact
		? t(
				"hub.checks.chipReady",
				"{{passed, number}} of {{total, number}} ready",
				{
					passed,
					total,
				},
			)
		: t("hub.checks.chipPass", "{{passed, number}} of {{total, number}} pass", {
				passed,
				total,
			});
	if (allPass)
		return (
			<StatusChip tone="good" icon={CircleCheck}>
				{text}
			</StatusChip>
		);
	return compact ? (
		<StatusChip tone="warning" icon={TriangleAlert}>
			{text}
		</StatusChip>
	) : (
		<StatusChip tone="critical" icon={OctagonX}>
			{text}
		</StatusChip>
	);
}

function RecheckResultLine({ recheck }: Readonly<{ recheck: Recheck }>) {
	const { t } = useTranslation("devices");
	const time = useAreaTime();
	const { outcome } = recheck;
	if (!outcome) return null;
	const at = time.clock(outcome.at);
	const summary = outcome.ok ? summarizeChecks(outcome.data) : undefined;
	if (!outcome.ok || !summary)
		return (
			<InlineResult tone="warning" onDismiss={recheck.dismiss}>
				{t(
					"hub.checks.result.failed",
					"Couldn't check at {{time}}: {{cause}}",
					{
						time: at,
						cause: outcome.ok
							? t("hub.checks.result.noAnswer", "The hub gave no answer.")
							: hubErrorCopy(t, outcome.error.code),
					},
				)}
			</InlineResult>
		);
	const { passed, total, failing } = summary;
	return failing.length ? (
		<InlineResult tone="warning" onDismiss={recheck.dismiss}>
			{t(
				"hub.checks.result.failing",
				"Checked again at {{time}}. {{passed, number}} of {{total, number}} pass; {{check}} still fails. Only the hub operator can fix it.",
				{ time: at, passed, total, check: checkLabel(t, failing[0]) },
			)}
		</InlineResult>
	) : (
		<InlineResult tone="good" onDismiss={recheck.dismiss}>
			{t(
				"hub.checks.result.passing",
				"Checked again at {{time}}. All {{total, number}} checks pass.",
				{ time: at, total },
			)}
		</InlineResult>
	);
}

interface ChecksView {
	hub: HubSupportRead;
	readiness: HubRead<DeviceSetupReadiness>;
	recheck: Recheck;
	summary?: CheckSummary;
}

/** Why "Check again" can't run right now (R7: visible, disabled, with the reason). */
function recheckGate(t: DevicesT, view: ChecksView): Gate | null {
	if (view.readiness.loading && !view.recheck.busy)
		return {
			kind: "busy",
			reason: t(
				"devices:hub.checks.gate.checking",
				"Wait for the hub check to finish.",
			),
		};
	return null;
}

/** The hub answers its checks while device support is off, so asking again is how a fix shows up. */
const ASKS_HUB_TOO = new Set(["off", "unreachable"]);

function CheckAgain({ view }: Readonly<{ view: ChecksView }>) {
	const { t } = useTranslation("devices");
	const { recheck, hub } = view;
	const label = recheck.busy
		? t("hub.checks.checking", "Checking…")
		: hub.support.state === "unreachable"
			? t("hub.checks.retry", "Retry now")
			: t("hub.checks.again", "Check again");
	return (
		<GatedAction gate={recheckGate(t, view)} className="items-end">
			<DvButton
				size="sm"
				icon={RefreshCw}
				busy={recheck.busy}
				onClick={() => {
					if (ASKS_HUB_TOO.has(hub.support.state)) void hub.retry();
					void recheck.run();
				}}
			>
				{label}
			</DvButton>
		</GatedAction>
	);
}

function ChecksBody({
	view,
	compact,
}: Readonly<{ view: ChecksView; compact: boolean }>) {
	const { t } = useTranslation("devices");
	const time = useAreaTime();
	const { hub, readiness, recheck, summary } = view;
	const { state } = hub.support;
	const data = readiness.data;
	const list = (items: ChecklistItem[]): ReactNode => (
		<Checklist
			label={t("hub.checks.title", "Hub status checks")}
			items={items}
			className={compact ? undefined : "[&>li]:py-2.5"}
		/>
	);
	if (!data) {
		if (readiness.loading || recheck.busy || state === "checking")
			return list(waitingItems(t));
		if (state === "off")
			return (
				<>
					<RecheckResultLine recheck={recheck} />
					<GateNotice
						kind="hub"
						title={t("hub.checks.off.title", "Devices are off on this hub.")}
						text={t(
							"hub.checks.off.unread",
							"Its checks couldn't be read. Select Check again once the hub operator has turned device support on.",
						)}
					/>
				</>
			);
		return (
			<>
				<RecheckResultLine recheck={recheck} />
				<StateView
					kind={
						readiness.freshness.age === "noaccess" ? "noaccess" : "notloaded"
					}
					icon={CircleDashed}
					title={t(
						"hub.checks.notRun.title",
						"The checks haven't run: the hub didn't answer.",
					)}
					text={
						readiness.error ? hubErrorCopy(t, readiness.error.code) : undefined
					}
				/>
			</>
		);
	}
	const failing = summary?.failing.length ?? 0;
	return (
		<>
			<RecheckResultLine recheck={recheck} />
			{readiness.error && readiness.freshness.at !== undefined ? (
				<Hint>
					{t(
						"hub.checks.lastKnown",
						"Last known results, checked at {{time}}. The app can't check again until it reaches the hub.",
						{ time: time.clock(readiness.freshness.at, false) },
					)}
				</Hint>
			) : null}
			{failing && !compact ? (
				<Hint>
					{t(
						"hub.checks.operatorOnly",
						"Each failing check is fixed by the hub operator. Nothing on this computer or your devices can fix it.",
					)}
				</Hint>
			) : null}
			{list(
				data.checks.map((check) =>
					compact ? compactItem(t, check) : fullItem(t, check),
				),
			)}
		</>
	);
}

function ChecksFoot({
	view,
	compact,
}: Readonly<{ view: ChecksView; compact: boolean }>) {
	const { t } = useTranslation("devices");
	const time = useAreaTime();
	const { readiness } = view;
	const at = readiness.freshness.at ?? readiness.freshness.dataFrom;
	if (!readiness.data || at === undefined)
		return (
			<span>
				{readiness.loading || view.recheck.busy
					? t("hub.checks.foot.running", "Running the checks…")
					: t("hub.checks.foot.notRun", "Not checked yet.")}
			</span>
		);
	const checked = t("hub.checks.foot.checkedAt", "Checked at {{time}}.", {
		time: time.clock(at),
	});
	const minutes = Math.round((readiness.freshness.cadenceS ?? 300) / 60);
	return (
		<span>
			<span title={time.abs(at)}>{checked}</span>{" "}
			{compact
				? t(
						"hub.checks.foot.operator",
						"Each failing check is fixed by the hub operator.",
					)
				: t("hub.checks.foot.cadence", {
						count: minutes,
						defaultValue_one:
							"The app runs these checks when you open this page or Set up a device, and every minute while Devices is open.",
						defaultValue_other:
							"The app runs these checks when you open this page or Set up a device, and every {{count, number}} minutes while Devices is open.",
					})}
		</span>
	);
}

/**
 * The hub's readiness checks with "Check again" (SPEC §5.10, BG37). `compact`
 * is the setup wizard's block: one line per check and the hub's verdict.
 */
export function ReadinessList({
	compact = false,
	onReady,
}: Readonly<{ compact?: boolean; onReady?(ready: boolean): void }>) {
	const { t } = useTranslation("devices");
	const hub = useHubSupport();
	const readiness = useReadiness();
	const recheck = useRecheck();
	const summary = summarizeChecks(readiness.data);
	const view: ChecksView = { hub, readiness, recheck, summary };
	const ready = readiness.data?.ready;
	const notify = useRef(onReady);
	useEffect(() => {
		notify.current = onReady;
	}, [onReady]);
	useEffect(() => {
		if (ready !== undefined) notify.current?.(ready);
	}, [ready]);
	const off = hub.support.state === "off" && !readiness.data;

	return (
		<Block
			id="readiness"
			icon={ListChecks}
			title={t("hub.checks.title", "Hub status checks")}
			summary={
				summary ? <SummaryChip summary={summary} compact={compact} /> : null
			}
			stamp={
				<HubReadStamp
					freshness={readiness.freshness}
					loading={!readiness.data && readiness.loading}
				/>
			}
			tools={<CheckAgain view={view} />}
			foot={off ? undefined : <ChecksFoot view={view} compact={compact} />}
			className="scroll-mt-4"
		>
			<ChecksBody view={view} compact={compact} />
		</Block>
	);
}
