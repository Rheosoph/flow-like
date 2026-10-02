"use client";

import { useTranslation } from "@flow-like/locales";
import {
	Ban,
	CircleDashed,
	CircleSlash,
	Hourglass,
	type LucideIcon,
	Network,
	Rocket,
	ShieldCheck,
	Stethoscope,
} from "lucide-react";
import { type ReactNode, useMemo, useState } from "react";
import { useUserIdentity } from "../../../../hooks/use-user-lookup";
import type { EverywhereRow } from "../../../../lib/device-management/model/app-plan";
import { Checkbox } from "../../../ui/checkbox";
import { useAreaTime } from "../primitives/area-context";
import { Block } from "../primitives/block";
import { DvButton } from "../primitives/dv-button";
import { FreshnessStamp } from "../primitives/freshness-stamp";
import { GateInline } from "../primitives/gate-notice";
import { PresenceChip, RelationshipChip } from "../primitives/status-chip";
import { cx } from "../primitives/tone";
import { ACCOUNT_SCOPE } from "../routing/devices-route";
import { useRouteLink } from "../routing/use-devices-route";
import { stampOf, useAppNames } from "../shell/attention-popover";
import { useDeviceRows, useOverlay } from "../workspace";
import {
	APP_LINKS,
	LINK,
	LinkButton,
	PLAIN_CHIP,
	ShowMore,
	UnknownAction,
	useAppPage,
	useCapped,
	useGateText,
	useNameList,
} from "./app-shared";
import {
	ELSEWHERE_CAP,
	NAME_CAP,
	SENTENCE_NAME_CAP,
	blocksDeploy,
} from "./app-view-local";

const CHECK =
	"mt-0.5 border-border-strong shadow-none data-[state=checked]:border-foreground data-[state=checked]:bg-foreground data-[state=checked]:text-background focus-visible:ring-0 focus-visible:outline-2 focus-visible:outline-offset-2 focus-visible:outline-ring";

function GroupHead({
	icon: Icon,
	label,
	count,
	action,
}: Readonly<{
	icon: LucideIcon;
	label: string;
	count: number;
	action?: ReactNode;
}>) {
	return (
		<div className="flex flex-wrap items-center gap-x-2 gap-y-1 px-4 pt-3 pb-1">
			<h3 className="inline-flex items-center gap-1.5 text-label font-semibold tracking-[0.06em] text-muted-foreground uppercase">
				<Icon aria-hidden className="size-3.5" />
				{label}
				<span className="font-mono tabular-nums">{count}</span>
			</h3>
			<span className="flex-1" />
			{action}
		</div>
	);
}

function DeviceLine({
	row,
	children,
}: Readonly<{ row: EverywhereRow; children?: ReactNode }>) {
	const link = useRouteLink();
	return (
		<span className="flex flex-wrap items-center gap-x-2.5 gap-y-1">
			<a
				{...link(APP_LINKS.device(row.deviceId))}
				className={cx(LINK, "font-mono text-ui font-semibold")}
			>
				{row.name}
			</a>
			<PresenceChip
				kind={row.presence.kind}
				{...(row.presence.since === undefined
					? {}
					: { since: row.presence.since })}
				short
				className={PLAIN_CHIP}
			/>
			{children}
		</span>
	);
}

function MoreNames({
	rows,
	onShow,
}: Readonly<{ rows: readonly EverywhereRow[]; onShow(): void }>) {
	const { t } = useTranslation("devices");
	const nameList = useNameList();
	if (!rows.length) return null;
	return (
		<li
			data-more=""
			className="flex flex-wrap items-center gap-x-3 gap-y-1 border-t border-hairline px-4 py-2 text-xs text-muted-foreground"
		>
			<span>
				{t("app.else.more", "{{count, number}} more: {{names}}", {
					count: rows.length,
					names: nameList(
						rows.map((row) => row.name),
						NAME_CAP,
					),
				})}
			</span>
			<ShowMore count={rows.length} onClick={onShow} />
		</li>
	);
}

const ROW = "flex flex-wrap items-start gap-x-4 gap-y-2 px-4 py-2.5";
const SUB = "text-xs text-muted-foreground";

function RunsLine({ row }: Readonly<{ row: EverywhereRow }>) {
	const { t } = useTranslation("devices");
	const { data } = useAppPage();
	const time = useAreaTime();
	const appName = useAppNames();
	const device = data.devices.get(row.deviceId);
	const first = Array.isArray(device?.services) ? device.services[0] : null;
	const lastKnown =
		first && !["live", "current"].includes(first.freshness.age)
			? first.freshness.at
			: undefined;
	const [a, b, ...rest] = row.runs;
	const name = (entry: EverywhereRow["runs"][number]) => (
		<>
			<span className="font-mono">{entry.serviceId}</span>{" "}
			{t("app.else.runsApp", "({{app}})", {
				app: appName(entry.projectId) ?? entry.projectId,
			})}
		</>
	);
	return (
		<p className={SUB}>
			{a ? (
				<>
					{t("app.else.runs", "Runs")} {name(a)}
					{b ? <>, {name(b)}</> : null}
					{rest.length
						? ` ${t("app.else.runsMore", "and {{count, number}} more", { count: rest.length })}`
						: ""}
					{lastKnown === undefined
						? "."
						: ` · ${t("app.else.lastKnown", "last known, {{ago}}", { ago: time.ago(lastKnown) })}.`}
				</>
			) : (
				t("app.else.runsNothing", "Runs nothing yet.")
			)}
		</p>
	);
}

function NotDeployedRow({
	row,
	checked,
	onToggle,
}: Readonly<{
	row: EverywhereRow;
	checked: boolean;
	onToggle(checked: boolean): void;
}>) {
	const { t } = useTranslation("devices");
	const overlay = useOverlay();
	const gateText = useGateText();
	const gate = blocksDeploy(row.gate) ? gateText(row.gate) : null;
	const diagnose = gate && row.gate?.fix?.kind === "diagnose";
	return (
		<li
			data-device={row.deviceId}
			className={cx(ROW, "border-t border-hairline first:border-t-0")}
		>
			<div className="flex min-w-0 flex-[1_1_320px] items-start gap-4">
				<Checkbox
					checked={checked && !gate}
					disabled={!!gate}
					onCheckedChange={(next) => onToggle(next === true)}
					aria-label={t("app.else.select", "Select {{device}}", {
						device: row.name,
					})}
					className={CHECK}
				/>
				<div className="flex min-w-0 flex-1 flex-col gap-0.5">
					<DeviceLine row={row} />
					<RunsLine row={row} />
					{gate ? (
						<GateInline kind={gate.kind} className="max-w-[60ch]">
							{gate.reason}
						</GateInline>
					) : null}
				</div>
			</div>
			<div className="flex flex-col items-end gap-1.5 @max-[500px]/app:items-start @max-[500px]/app:pl-8">
				<LinkButton
					route={APP_LINKS.deploy({ deviceIds: [row.deviceId] })}
					gate={gate}
					reasonShown
					act="deploy-here"
					size="sm"
					icon={Rocket}
				>
					{t("app.else.deployHere", "Deploy here")}
				</LinkButton>
				{diagnose ? (
					<DvButton
						size="xs"
						icon={Stethoscope}
						onClick={() => overlay.openDiagnose(row.deviceId)}
					>
						{t("app.unknown.diagnose", "Diagnose")}
					</DvButton>
				) : null}
			</div>
		</li>
	);
}

function Owner({
	row,
	render,
}: Readonly<{
	row: EverywhereRow;
	render(owner: string | undefined, endsAt: number | undefined): ReactNode;
}>) {
	const { data } = useAppPage();
	const device = data.devices.get(row.deviceId);
	const owner = useUserIdentity(
		device?.relationship === "shared" ? device.row.owner_id : undefined,
	);
	const endsAt =
		typeof device?.row.access_expires_at === "number"
			? device.row.access_expires_at
			: undefined;
	return render(owner.isResolved ? owner.label : undefined, endsAt);
}

/** "Shared by Mira Novak · ends in 14 h" for a device shared with the viewer. */
function SharedChip({
	row,
	owner,
	endsAt,
}: Readonly<{ row: EverywhereRow; owner?: string; endsAt?: number }>) {
	const { data } = useAppPage();
	if (data.devices.get(row.deviceId)?.relationship !== "shared") return null;
	return (
		<RelationshipChip
			relationship="shared"
			{...(owner ? { ownerName: owner } : {})}
			{...(endsAt === undefined ? {} : { endsAt })}
		/>
	);
}

function UnknownRow({ row }: Readonly<{ row: EverywhereRow }>) {
	const { t } = useTranslation("devices");
	const kind = row.unknown?.kind ?? "notloaded";
	return (
		<li
			data-device={row.deviceId}
			className={cx(ROW, "border-t border-hairline first:border-t-0")}
		>
			<div className="flex min-w-0 flex-[1_1_320px] flex-col gap-0.5">
				<Owner
					row={row}
					render={(owner, endsAt) => (
						<>
							<DeviceLine row={row}>
								<SharedChip row={row} owner={owner} endsAt={endsAt} />
							</DeviceLine>
							<p className={SUB}>
								{kind === "locked"
									? owner
										? t(
												"app.else.lockedShared",
												"Locked on this computer · shared by {{owner}}. Unlock to check what runs there.",
												{ owner },
											)
										: t(
												"app.else.locked",
												"Locked on this computer. Unlock to check what runs there.",
											)
									: kind === "nokeys"
										? t(
												"app.else.noKeys",
												"This computer has no keys for it. Restore them to check what runs there.",
											)
										: t(
												"app.else.noStatus",
												"Its status can't be read right now, so what runs there is unknown.",
											)}
							</p>
						</>
					)}
				/>
			</div>
			{row.unknown ? (
				<UnknownAction deviceId={row.deviceId} unknown={row.unknown} />
			) : null}
		</li>
	);
}

function NeverRow({ row }: Readonly<{ row: EverywhereRow }>) {
	const { t } = useTranslation("devices");
	const { data } = useAppPage();
	const link = useRouteLink();
	const time = useAreaTime();
	const registered = data.devices.get(row.deviceId)?.row.registered_at;
	return (
		<li
			data-device={row.deviceId}
			className={cx(ROW, "border-t border-hairline first:border-t-0")}
		>
			<div className="flex min-w-0 flex-[1_1_320px] flex-col gap-0.5">
				<DeviceLine row={row} />
				<p className={SUB}>
					{registered
						? t(
								"app.else.never",
								"Registered {{ago}} and hasn't sent any status yet, so nothing runs there. Deploying needs it online first.",
								{ ago: time.ago(registered, "long") },
							)
						: t(
								"app.else.neverNoTime",
								"It hasn't sent any status yet, so nothing runs there. Deploying needs it online first.",
							)}
				</p>
			</div>
			<DvButton size="sm" asChild>
				<a
					data-act="start-instructions"
					{...link({
						screen: "device",
						deviceId: row.deviceId,
						tab: "overview",
					})}
				>
					{t("app.else.startInstructions", "Start instructions")}
				</a>
			</DvButton>
		</li>
	);
}

function NoAccessRow({ row }: Readonly<{ row: EverywhereRow }>) {
	const { t } = useTranslation("devices");
	const { view } = useAppPage();
	const link = useRouteLink();
	return (
		<li
			data-device={row.deviceId}
			className={cx(ROW, "border-t border-hairline first:border-t-0")}
		>
			<div className="flex min-w-0 flex-[1_1_320px] flex-col gap-0.5">
				<Owner
					row={row}
					render={(owner, endsAt) => (
						<>
							<DeviceLine row={row}>
								<SharedChip row={row} owner={owner} endsAt={endsAt} />
							</DeviceLine>
							<p className={SUB}>
								{owner
									? t(
											"app.else.noAccessOwner",
											"{{owner}} shared it with you for other apps only, so {{app}} can't be seen there, even after unlocking.",
											{ owner, app: view.app.name },
										)
									: t(
											"app.else.noAccess",
											"Your access to it covers other apps only, so {{app}} can't be seen there, even after unlocking.",
											{ app: view.app.name },
										)}
							</p>
						</>
					)}
				/>
			</div>
			<DvButton size="sm" asChild>
				<a
					data-act="request-access"
					{...link(
						{ screen: "access", tab: "shared", action: "request" },
						{ scope: ACCOUNT_SCOPE },
					)}
				>
					{t("app.else.requestAccess", "Request access")}
				</a>
			</DvButton>
		</li>
	);
}

/** APP §2.12: every device where the app isn't shown as running, by what is known about it. */
export function EverywhereElse({
	variant = "else",
}: Readonly<{ variant?: "else" | "could" }>) {
	const { t } = useTranslation("devices");
	const { view, data } = useAppPage();
	const link = useRouteLink();
	const time = useAreaTime();
	const rows = useDeviceRows();
	const [picked, setPicked] = useState<ReadonlySet<string>>(new Set());
	const { notDeployed, unknown, never, noAccess } = view.everywhereElse;
	const nameList = useNameList();
	const groups = {
		notDeployed: useCapped(notDeployed, ELSEWHERE_CAP),
		unknown: useCapped(unknown, ELSEWHERE_CAP),
		never: useCapped(never, ELSEWHERE_CAP),
		noAccess: useCapped(noAccess, ELSEWHERE_CAP),
	};
	const ready = useMemo(
		() =>
			notDeployed
				.filter((row) => !blocksDeploy(row.gate))
				.map((row) => row.deviceId),
		[notDeployed],
	);
	const selected = ready.filter((id) => picked.has(id));
	const revoked = [...data.devices.values()]
		.filter((device) => device.presence.kind === "revoked")
		.map((device) => device.row.display_name || device.row.name);
	const total =
		notDeployed.length + unknown.length + never.length + noAccess.length;
	if (!total && !revoked.length) return null;
	const stamp = stampOf(rows.freshness);
	const toggle = (deviceId: string, on: boolean) =>
		setPicked((current) => {
			const next = new Set(current);
			if (on) next.add(deviceId);
			else next.delete(deviceId);
			return next;
		});
	return (
		<Block
			id="ad-else"
			icon={Network}
			title={
				variant === "could"
					? t("app.else.titleCould", "Where it could run")
					: t("app.else.title", "Everywhere else")
			}
			flush
			stamp={
				<FreshnessStamp
					{...stamp}
					{...(stamp.observedAt === undefined
						? {}
						: {
								text: t("app.else.stamp", "device list checked {{ago}}", {
									ago: time.ago(stamp.observedAt),
								}),
							})}
				/>
			}
			foot={
				<div className="flex w-full flex-wrap items-center gap-x-4 gap-y-2">
					{revoked.length ? (
						<span data-revoked="">
							{t("app.else.revoked", {
								count: revoked.length,
								names: nameList(revoked, SENTENCE_NAME_CAP),
								defaultValue_one: "{{names}} is revoked, so it isn't read.",
								defaultValue_other:
									"{{names}} are revoked, so they aren't read.",
							})}
						</span>
					) : null}
					{selected.length ? (
						<DvButton size="sm" icon={Rocket} asChild>
							<a
								data-act="deploy-selected"
								{...link(APP_LINKS.deploy({ deviceIds: selected }))}
							>
								{t("app.else.deploySelected", {
									count: selected.length,
									defaultValue_one: "Deploy to {{count, number}} selected…",
									defaultValue_other: "Deploy to {{count, number}} selected…",
								})}
							</a>
						</DvButton>
					) : ready.length > 1 ? (
						<span className="ml-auto">
							{t(
								"app.else.selectHint",
								"Select devices above to deploy {{app}} to several at once.",
								{ app: view.app.name },
							)}
						</span>
					) : null}
				</div>
			}
		>
			{notDeployed.length ? (
				<section data-group="not-deployed">
					<GroupHead
						icon={CircleSlash}
						label={t("app.else.notDeployed", "Not deployed on")}
						count={notDeployed.length}
						action={
							ready.length > 1 ? (
								<DvButton
									variant="link"
									size="xs"
									data-act="select-all-ready"
									onClick={() => setPicked(new Set(ready))}
								>
									{t("app.else.selectAll", "Select all ready")}
								</DvButton>
							) : undefined
						}
					/>
					<ul className="m-0 list-none p-0">
						{groups.notDeployed.shown.map((row) => (
							<NotDeployedRow
								key={row.deviceId}
								row={row}
								checked={picked.has(row.deviceId)}
								onToggle={(on) => toggle(row.deviceId, on)}
							/>
						))}
						<MoreNames
							rows={groups.notDeployed.rest}
							onShow={groups.notDeployed.showAll}
						/>
					</ul>
				</section>
			) : null}
			{unknown.length ? (
				<section data-group="unknown" className="border-t border-hairline">
					<GroupHead
						icon={CircleDashed}
						label={t("app.else.unknown", "Unknown on")}
						count={unknown.length}
					/>
					<ul className="m-0 list-none p-0">
						{groups.unknown.shown.map((row) => (
							<UnknownRow key={row.deviceId} row={row} />
						))}
						<MoreNames
							rows={groups.unknown.rest}
							onShow={groups.unknown.showAll}
						/>
					</ul>
				</section>
			) : null}
			{never.length ? (
				<section data-group="never" className="border-t border-hairline">
					<GroupHead
						icon={Hourglass}
						label={t("app.else.neverTitle", "Hasn't checked in yet")}
						count={never.length}
					/>
					<ul className="m-0 list-none p-0">
						{groups.never.shown.map((row) => (
							<NeverRow key={row.deviceId} row={row} />
						))}
						<MoreNames rows={groups.never.rest} onShow={groups.never.showAll} />
					</ul>
				</section>
			) : null}
			{noAccess.length ? (
				<section data-group="no-access" className="border-t border-hairline">
					<GroupHead
						icon={Ban}
						label={t("app.else.noAccessTitle", "No access to status on")}
						count={noAccess.length}
					/>
					<ul className="m-0 list-none p-0">
						{groups.noAccess.shown.map((row) => (
							<NoAccessRow key={row.deviceId} row={row} />
						))}
						<MoreNames
							rows={groups.noAccess.rest}
							onShow={groups.noAccess.showAll}
						/>
					</ul>
				</section>
			) : null}
			{total ? (
				<p
					data-else-note=""
					className="flex items-start gap-1.5 border-t border-hairline bg-surface-sunken px-4 py-2.5 text-xs text-ink-2"
				>
					<ShieldCheck aria-hidden className="mt-0.5 size-3.5 shrink-0" />
					{t(
						"app.else.note",
						"This view never counts a device as not deployed unless its status was readable.",
					)}
				</p>
			) : null}
		</Block>
	);
}
