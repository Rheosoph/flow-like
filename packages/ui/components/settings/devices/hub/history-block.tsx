"use client";

import { useTranslation } from "@flow-like/locales";
import { Archive, User } from "lucide-react";
import type { HubStandalone } from "../../../../lib/device-management/hub/endpoints";
import { deviceName } from "../../../../lib/device-management/model/device-view";
import type { ArchiveUsage } from "../../../../lib/device-management/model/types";
import { Block } from "../primitives/block";
import { CellSub, DvTable, Td, Th, Tr } from "../primitives/dv-table";
import { Meter } from "../primitives/meter";
import { StateView } from "../primitives/state-view";
import { StatusChip } from "../primitives/status-chip";
import { useRouteLink } from "../routing/use-devices-route";
import { hubErrorCopy, useDeviceRows } from "../workspace";
import { Hint, HubReadStamp, SectionHead, Tech, useLocale } from "./hub-parts";
import { DAY, type HubView, planName } from "./hub-view";
import { bytesText, limitTone } from "./use-hub-facts";

/** The hub drops retained history after this long, whatever a plan allows. */
const MAX_RETENTION_S = 31 * DAY;
const DEVICES_SHOWN = 5;

interface Tier {
	key: string;
	bytes: number;
	retentionS: number;
}

type ArchiveDevice = ArchiveUsage["devices"][number];

/** The hub's plans, smallest first; a plan that stores nothing comes first. */
const tiersOf = (record: HubStandalone | undefined) => {
	const tiers: Tier[] = Object.entries(record?.telemetry_tiers ?? {}).map(
		([key, tier]) => ({
			key,
			bytes: tier.max_bytes,
			retentionS: Math.min(tier.retention_seconds, MAX_RETENTION_S),
		}),
	);
	return tiers.sort(
		(a, b) =>
			a.bytes - b.bytes ||
			a.retentionS - b.retentionS ||
			a.key.localeCompare(b.key),
	);
};

const daysOf = (seconds: number) => Math.round(seconds / DAY);

function TierRow({ tier, yours }: Readonly<{ tier: Tier; yours: boolean }>) {
	const { t } = useTranslation("devices");
	const locale = useLocale();
	const keeps = t("hub.history.keeps", "Keeps history for");
	const stored = tier.bytes > 0 && tier.retentionS > 0;
	return (
		<Tr selected={yours} data-tier={tier.key}>
			<Td label={t("hub.history.plan", "Plan")} kind="name">
				<b className="font-semibold">{planName(tier.key)}</b>
				<Tech>{tier.key}</Tech>
				{yours ? (
					<StatusChip tone="outline" icon={User} className="ml-2">
						{t("hub.history.yours", "Your plan")}
					</StatusChip>
				) : null}
			</Td>
			{stored ? (
				<>
					<Td label={keeps} className="tabular-nums">
						{t("hub.history.days", {
							count: daysOf(tier.retentionS),
							defaultValue_one: "{{count, number}} day",
							defaultValue_other: "{{count, number}} days",
						})}
					</Td>
					<Td label={t("hub.history.upTo", "Up to")} className="tabular-nums">
						{bytesText(locale, tier.bytes)}
					</Td>
				</>
			) : (
				<Td label={keeps} colSpan={2}>
					{t("hub.history.notStored", "Not stored in the cloud")}
					<CellSub>
						{t("hub.history.deviceOnly", "History stays on the device only.")}
					</CellSub>
				</Td>
			)}
		</Tr>
	);
}

function TierTable({
	tiers,
	mine,
}: Readonly<{ tiers: Tier[]; mine?: string }>) {
	const { t } = useTranslation("devices");
	return (
		<DvTable
			label={t("hub.history.title", "History storage")}
			cols={["40%", "34%", "26%"]}
			stackAt={560}
			head={
				<tr>
					<Th>{t("hub.history.plan", "Plan")}</Th>
					<Th>{t("hub.history.keeps", "Keeps history for")}</Th>
					<Th>{t("hub.history.upTo", "Up to")}</Th>
				</tr>
			}
		>
			{tiers.map((tier) => (
				<TierRow key={tier.key} tier={tier} yours={tier.key === mine} />
			))}
		</DvTable>
	);
}

/** One device's share of the plan, linked to where its history is set up. */
function HistoryDevice({ device }: Readonly<{ device: ArchiveDevice }>) {
	const { t } = useTranslation("devices");
	const locale = useLocale();
	const link = useRouteLink();
	const { rows } = useDeviceRows();
	const row = rows?.find((item) => item.device_id === device.device_id);
	return (
		<li
			data-history-device={device.device_id}
			className="flex items-baseline justify-between gap-3 border-t border-hairline py-1.5 text-ui first:border-t-0"
		>
			{row ? (
				<a
					{...link({
						screen: "device",
						deviceId: device.device_id,
						tab: "activity",
					})}
					className="truncate font-mono font-medium hover:underline"
				>
					{deviceName(row)}
				</a>
			) : (
				<span className="truncate text-muted-foreground">
					{t("hub.history.gone", "A device no longer in your list")}
				</span>
			)}
			<span className="font-mono text-xs whitespace-nowrap tabular-nums">
				{bytesText(locale, device.used_bytes)}
			</span>
		</li>
	);
}

function HistoryDevices({ usage }: Readonly<{ usage: ArchiveUsage }>) {
	const { t } = useTranslation("devices");
	const devices = usage.devices
		.filter((device) => device.used_bytes > 0 || device.segments > 0)
		.sort((a, b) => b.used_bytes - a.used_bytes);
	const shown = devices.slice(0, DEVICES_SHOWN);
	if (!shown.length) return null;
	return (
		<>
			<ul className="flex flex-col">
				{shown.map((device) => (
					<HistoryDevice key={device.device_id} device={device} />
				))}
			</ul>
			{devices.length > shown.length ? (
				<Hint>
					{t("hub.history.moreDevices", {
						count: devices.length - shown.length,
						defaultValue_one: "and {{count, number}} more device",
						defaultValue_other: "and {{count, number}} more devices",
					})}
				</Hint>
			) : null}
		</>
	);
}

function PlanLine({ usage }: Readonly<{ usage: ArchiveUsage }>) {
	const { t } = useTranslation("devices");
	const locale = useLocale();
	const plan = planName(usage.tier);
	if (usage.max_bytes <= 0)
		return t("hub.history.planLine", "{{plan}} plan", { plan });
	return t("hub.history.planStored", {
		plan,
		count: daysOf(Math.min(usage.retention_seconds, MAX_RETENTION_S)),
		size: bytesText(locale, usage.max_bytes),
		defaultValue_one: "{{plan}} plan · {{count, number}} day · up to {{size}}",
		defaultValue_other:
			"{{plan}} plan · {{count, number}} days · up to {{size}}",
	});
}

/** The viewer's own plan and how much of it their devices use (E14). */
function HistoryUsage({ usage }: Readonly<{ usage: ArchiveUsage }>) {
	const { t } = useTranslation("devices");
	const locale = useLocale();
	const stored = usage.max_bytes > 0;
	const usedText = t("hub.history.used", "{{used}} of {{max}} used", {
		used: bytesText(locale, usage.used_bytes),
		max: bytesText(locale, usage.max_bytes),
	});
	return (
		<>
			<SectionHead
				title={t("hub.history.yourHistory", "Your history")}
				note={<PlanLine usage={usage} />}
			/>
			{stored ? (
				<Meter
					segments={[
						{
							value: (usage.used_bytes / usage.max_bytes) * 100,
							tone: limitTone(usage.used_bytes, usage.max_bytes),
						},
					]}
					label={usedText}
					caption={usedText}
				/>
			) : (
				<Hint>
					{t(
						"hub.history.freePlan",
						"Your plan doesn't store history in the cloud. History stays on each device only.",
					)}
				</Hint>
			)}
			<HistoryDevices usage={usage} />
			<Hint>
				{t(
					"hub.history.ownerPlan",
					"History is stored under each device owner's plan. On devices shared with you, the owner's plan applies. Set it up per device in its Activity tab.",
				)}
			</Hint>
		</>
	);
}

/** Why the viewer's plan isn't shown: an older hub (BG30 interim), devices off, or a failed read. */
function NoUsageNote({ view }: Readonly<{ view: HubView }>) {
	const { t } = useTranslation("devices");
	const { archive, hub } = view;
	if (archive.missingOnHub)
		return t(
			"hub.history.interim",
			"This hub doesn't report your plan or how much history you use yet. Each device's Activity tab shows what it keeps.",
		);
	if (hub.support.state === "off")
		return t(
			"hub.history.off",
			"Your plan and usage show once device support is on.",
		);
	return t(
		"hub.history.unread",
		"Your plan and usage couldn't be read: {{cause}}",
		{
			cause: archive.error
				? hubErrorCopy(t, archive.error.code)
				: t("hub.history.noAnswer", "The hub gave no answer."),
		},
	);
}

function YourHistory({ view }: Readonly<{ view: HubView }>) {
	const { t } = useTranslation("devices");
	const { archive } = view;
	if (archive.data) return <HistoryUsage usage={archive.data} />;
	if (archive.loading && !archive.missingOnHub)
		return (
			<StateView
				kind="loading"
				title={t("hub.history.loadingUsage", "Loading your history")}
			/>
		);
	return (
		<>
			<SectionHead title={t("hub.history.yourHistory", "Your history")} />
			<Hint>
				<NoUsageNote view={view} />
			</Hint>
		</>
	);
}

function Plans({ view, tiers }: Readonly<{ view: HubView; tiers: Tier[] }>) {
	const { t } = useTranslation("devices");
	return (
		<div className="grid grid-cols-[minmax(0,1.15fr)_minmax(0,1fr)] @max-[900px]/devices:grid-cols-1">
			<div className="min-w-0">
				<TierTable tiers={tiers} mine={view.archive.data?.tier} />
				<Hint className="px-4 pt-2.5 pb-3">
					{t(
						"hub.history.cap",
						"The hub never keeps history longer than 31 days, whatever a plan allows. When the space is full, the oldest records go first.",
					)}
				</Hint>
			</div>
			<div
				data-hub="your-history"
				className="flex min-w-0 flex-col gap-3 border-l border-hairline px-4 py-3 @max-[900px]/devices:border-t @max-[900px]/devices:border-l-0"
			>
				<YourHistory view={view} />
			</div>
		</div>
	);
}

function HistoryBody({ view }: Readonly<{ view: HubView }>) {
	const { t } = useTranslation("devices");
	const tiers = tiersOf(view.record);
	if (view.record && tiers.length && view.hub.support.state !== "checking")
		return <Plans view={view} tiers={tiers} />;
	return (
		<div className="px-4 py-3">
			{view.hub.support.state === "checking" ? (
				<StateView
					kind="loading"
					title={t("hub.history.loading", "Loading plans")}
				/>
			) : view.record ? (
				<StateView
					kind="unsupported"
					title={t(
						"hub.history.none",
						"This hub doesn't state how long it keeps device history.",
					)}
				/>
			) : (
				<StateView
					kind="notloaded"
					title={t(
						"hub.history.notLoaded",
						"The plans haven't been read: the hub didn't answer.",
					)}
				/>
			)}
		</div>
	);
}

/** SPEC §5.10 block 5 (BG30, hub part): what each plan keeps, and the viewer's own plan and usage. */
export function HistoryBlock({ view }: Readonly<{ view: HubView }>) {
	const { t } = useTranslation("devices");
	return (
		<Block
			id="history"
			icon={Archive}
			title={t("hub.history.title", "History storage")}
			stamp={
				<HubReadStamp
					freshness={view.hub.freshness}
					loading={view.hub.support.state === "checking"}
				/>
			}
			flush
			className="scroll-mt-4"
			foot={
				<span>
					{t(
						"hub.history.foot",
						"Retained logs and metrics are encrypted on the device; the hub stores them without being able to read them. Not a billing record.",
					)}
				</span>
			}
		>
			<HistoryBody view={view} />
		</Block>
	);
}
