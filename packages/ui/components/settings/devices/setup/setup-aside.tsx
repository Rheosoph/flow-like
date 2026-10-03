"use client";

import { useTranslation } from "@flow-like/locales";
import {
	ClipboardList,
	FileUp,
	Globe,
	Hourglass,
	ListChecks,
	Server,
	Timer,
} from "lucide-react";
import type { ReactNode } from "react";
import type { PendingSetup } from "../../../../lib/device-management/model/types";
import { enumLabel } from "../copy/enum-labels";
import { useAreaTime } from "../primitives/area-context";
import { Block } from "../primitives/block";
import { DvButton } from "../primitives/dv-button";
import { FreshnessStamp } from "../primitives/freshness-stamp";
import { IdRef } from "../primitives/id-ref";
import { PresenceGlyph } from "../primitives/presence-glyph";
import { useEnrollments, usePendingSetups } from "../workspace";
import { useSetup } from "./setup-context";
import {
	IconList,
	Mono,
	PendingSetupsStamp,
	startByReleaseText,
} from "./setup-parts";
import { CREATE_STEP, startBy } from "./setup-state";
import { outcomeLabel } from "./steps/save-step";

/** How many pending setups the side list shows before it points to the full list (R11). */
const PENDING_CAP = 5;

const muted = "text-muted-foreground";

/** One fact of the side column. It keeps label and value side by side at 340 px, where `KeyValueList` stacks them. */
function KvRow({
	label,
	children,
}: Readonly<{ label: string; children: ReactNode }>) {
	return (
		<div className="contents">
			<dt className={muted}>{label}</dt>
			<dd className="m-0 min-w-0 wrap-anywhere">{children}</dd>
		</div>
	);
}

function PackageFact() {
	const { t } = useTranslation("devices");
	const time = useAreaTime();
	const { draft, create, expired } = useSetup();
	const { created } = draft;
	const sub = "block text-xs text-muted-foreground";
	if (draft.cancelledAt) return <>{enumLabel(t, "enrollment", "cancelled")}</>;
	if (created && expired) return <>{enumLabel(t, "enrollment", "expired")}</>;
	if (created && draft.registeredAt !== undefined)
		return (
			<>
				{t("setup.aside.package.used", "Used at {{time}}", {
					time: time.clock(draft.registeredAt),
				})}
				<span className={sub}>
					{t("setup.aside.package.usedHint", "its one-time secret is deleted")}
				</span>
			</>
		);
	if (created)
		return (
			<>
				{t("setup.aside.package.created", "Created {{when}}", {
					when: time.at(created.createdAt),
				})}
				<span className={sub}>
					{t("setup.aside.package.until", "works until {{until}} · {{left}}", {
						until: time.at(startBy(created)),
						left: time.ago(startBy(created)),
					})}
				</span>
			</>
		);
	if (create.run.status === "failed")
		return <>{t("setup.aside.package.failed", "Couldn't be created")}</>;
	if (create.run.status === "running")
		return <>{t("setup.aside.package.creating", "Being created")}</>;
	return (
		<span className={muted}>
			{t("setup.aside.package.none", "Not created yet")}
		</span>
	);
}

function ThisSetup() {
	const { t } = useTranslation("devices");
	const { draft, create } = useSetup();
	const { created } = draft;
	const ids = created ?? create.run.registration;
	const notChosen = (
		<span className={muted}>
			{t("setup.aside.notChosen", "Not chosen yet")}
		</span>
	);
	const backup: ReactNode = created
		? created.outcome
			? outcomeLabel(t, created.outcome)
			: null
		: draft.backup
			? t("setup.aside.backupOn", "On")
			: t("setup.aside.backupOff", "Off");
	return (
		<Block
			icon={ClipboardList}
			title={t("setup.aside.title", "This setup")}
			stamp={
				<FreshnessStamp
					source="local"
					age="current"
					text={t("setup.aside.stamp", "this window")}
				/>
			}
			foot={t(
				"setup.aside.foot",
				"Kept in this window so the waiting step can resume. The password and the package are never stored.",
			)}
		>
			<dl className="m-0 grid grid-cols-[minmax(96px,max-content)_minmax(0,1fr)] gap-x-4 gap-y-1.5 text-ui">
				<KvRow label={t("setup.field.name", "Name")}>
					{draft.name ? <Mono>{draft.name}</Mono> : notChosen}
				</KvRow>
				{draft.resumed && !draft.target ? null : (
					<KvRow label={t("setup.field.platform", "Platform")}>
						{draft.target
							? enumLabel(t, "targetShort", draft.target)
							: notChosen}
					</KvRow>
				)}
				{draft.resumed && !draft.mode ? null : (
					<KvRow label={t("setup.field.mode", "Run with")}>
						{draft.target && draft.mode
							? enumLabel(t, "packageMode", draft.mode)
							: notChosen}
					</KvRow>
				)}
				{backup === null ? null : (
					<KvRow label={t("setup.field.backup", "Account backup")}>
						{backup}
					</KvRow>
				)}
				<KvRow label={t("setup.field.package", "Package")}>
					<PackageFact />
				</KvRow>
				{ids?.deviceId ? (
					<KvRow label={t("setup.field.deviceId", "Device ID")}>
						<IdRef
							id={ids.deviceId}
							copyLabel={t("setup.copy.deviceId", "Copy device ID")}
						/>
					</KvRow>
				) : null}
				{ids?.enrollmentId ? (
					<KvRow label={t("setup.field.setupId", "Setup ID")}>
						<IdRef
							id={ids.enrollmentId}
							copyLabel={t("setup.copy.setupId", "Copy setup ID")}
						/>
					</KvRow>
				) : null}
				{created && !draft.cancelledAt ? (
					<KvRow label={t("setup.aside.device", "Device")}>
						{draft.checkedInAt !== undefined ? (
							<span className="inline-flex items-center gap-1.5">
								<PresenceGlyph kind="online" decorative />
								{t("setup.wait.chip.done", "Checked in")}
							</span>
						) : draft.registeredAt !== undefined ? (
							t("setup.wait.registered", "Registered")
						) : (
							t("setup.wait.chip.waiting", "Not seen yet")
						)}
					</KvRow>
				) : null}
			</dl>
		</Block>
	);
}

function Needs() {
	const { t } = useTranslation("devices");
	const time = useAreaTime();
	const { host, limits, releaseCutoff } = useSetup();
	return (
		<Block
			icon={ListChecks}
			title={t("setup.needs.title", "You'll need")}
			stamp={
				<FreshnessStamp
					source="local"
					age="current"
					text={t("setup.needs.stamp", "checklist")}
				/>
			}
		>
			<IconList
				rows={[
					{
						id: "device",
						icon: Server,
						text: t(
							"setup.needs.device",
							"The device: Linux (Intel/AMD or ARM) or an Apple-silicon Mac, with a terminal.",
						),
					},
					{
						id: "copy",
						icon: FileUp,
						text: t(
							"setup.needs.copy",
							"A way to copy one file to it: USB stick, file share or scp.",
						),
					},
					{
						id: "network",
						icon: Globe,
						text: t(
							"setup.needs.network",
							"Outbound HTTPS from the device to {{host}}.",
							{ host },
						),
					},
					{
						id: "time",
						icon: Timer,
						text:
							releaseCutoff === undefined
								? t(
										"setup.needs.time",
										"About 10 minutes, and {{count, number}} h to start the package once it's made.",
										{ count: Math.round(limits.lifetimeS / 3600) },
									)
								: `${t("setup.needs.timeOnly", "About 10 minutes.")} ${startByReleaseText(t, time, releaseCutoff, false)}`,
					},
				]}
			/>
		</Block>
	);
}

function PendingRow({
	setup,
	here,
}: Readonly<{ setup: PendingSetup; here: boolean }>) {
	const { t } = useTranslation("devices");
	const time = useAreaTime();
	const { startNew, leaveLink, nowS } = useSetup();
	const lapsed = setup.state === "expired" || setup.expiresAt <= nowS;
	return (
		<li className="grid grid-cols-[14px_minmax(0,1fr)_auto] items-center gap-x-2 gap-y-0.5 border-t border-hairline py-2.25 text-ui first:border-t-0 first:pt-0 last:pb-0">
			<PresenceGlyph kind="pending" decorative />
			<span className="truncate font-mono font-medium" title={setup.name}>
				{setup.name}
			</span>
			{here ? (
				<span className={muted}>{t("setup.aside.stamp", "this window")}</span>
			) : lapsed ? (
				<DvButton size="xs" onClick={() => startNew(setup.name)}>
					{t("setup.pending.again", "Set up again")}
				</DvButton>
			) : (
				<DvButton size="xs" asChild>
					<a
						{...leaveLink({
							screen: "setup",
							enrollmentId: setup.enrollmentId,
							step: 6,
						})}
					>
						{t("setup.pending.instructions", "Start instructions")}
					</a>
				</DvButton>
			)}
			<p className="col-start-2 col-end-[-1] text-xs text-muted-foreground">
				{lapsed
					? t("setup.pending.expired", "Expired {{when}} · no longer works", {
							when: time.at(setup.expiresAt),
						})
					: t("setup.pending.waiting", "Waiting · expires {{left}}", {
							left: time.ago(setup.expiresAt),
						})}
			</p>
		</li>
	);
}

function Pending() {
	const { t } = useTranslation("devices");
	const { draft, limits, leaveLink } = useSetup();
	const enrollments = useEnrollments();
	const setups = usePendingSetups().filter(
		(setup) => setup.state !== "cancelled",
	);
	const own = draft.created?.enrollmentId;
	const shown = setups.slice(0, PENDING_CAP);
	return (
		<Block
			icon={Hourglass}
			title={t("setup.pending.title", "Pending setups")}
			count={setups.length}
			stamp={<PendingSetupsStamp />}
			foot={
				limits.maxPending === undefined
					? t(
							"setup.pending.footPlain",
							"Unused packages count toward your limit until they're used, cancelled or expire.",
						)
					: t(
							"setup.pending.foot",
							"Unused packages count toward your limit of {{max, number}} until they're used, cancelled or expire.",
							{ max: limits.maxPending },
						)
			}
		>
			{shown.length ? (
				<ul className="m-0 flex list-none flex-col p-0">
					{shown.map((setup) => (
						<PendingRow
							key={setup.enrollmentId}
							setup={setup}
							here={setup.enrollmentId === own}
						/>
					))}
				</ul>
			) : (
				<p className="text-xs text-muted-foreground">
					{enrollments.loading
						? t("setup.pending.loading", "Reading the pending setups…")
						: t("setup.pending.none", "No other setups are waiting.")}
				</p>
			)}
			{setups.length > shown.length ? (
				<a
					{...leaveLink({ screen: "fleet", view: "devices" })}
					className="text-xs underline decoration-border-strong underline-offset-2 hover:decoration-current"
				>
					{t("setup.pending.all", "Show all {{count, number}}", {
						count: setups.length,
					})}
				</a>
			) : null}
		</Block>
	);
}

/** The side column: this window's choices, what to have at hand, and the account's other setups. */
export function SetupAside() {
	const { draft, step } = useSetup();
	return (
		<>
			<ThisSetup />
			{step < CREATE_STEP && !draft.created ? <Needs /> : null}
			<Pending />
		</>
	);
}
