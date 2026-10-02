"use client";

import { Trans, useTranslation } from "@flow-like/locales";
import {
	Bell,
	BellOff,
	CalendarClock,
	History,
	Mail,
	Search,
	Smartphone,
} from "lucide-react";
import { type FormEvent, useId, useMemo, useState } from "react";
import { deviceName } from "../../../../lib/device-management/model/device-view";
import { Label } from "../../../ui/label";
import { useAreaTime } from "../primitives/area-context";
import { Block } from "../primitives/block";
import { DvButton } from "../primitives/dv-button";
import { CellSub, DvTable, Td, Th, Tr } from "../primitives/dv-table";
import { DvInput } from "../primitives/form-fields";
import { FreshnessStamp } from "../primitives/freshness-stamp";
import { InlineResult, type ResultTone } from "../primitives/inline-result";
import { StateView } from "../primitives/state-view";
import { useRouteLink } from "../routing/use-devices-route";
import { stampOf } from "../shell/attention-popover";
import { useCertificateNotices } from "../workspace/use-hub";
import {
	type CertificateFleet,
	type CertificateRow,
	LIST_CAP,
	REMINDER_STAGES,
	type ReminderStage,
	type UpcomingReminder,
	findCertificate,
	isExpired,
	selfRenewing,
	upcomingReminders,
} from "./certificates-model";
import {
	DayOf,
	DeviceLink,
	DeviceSelect,
	LABEL,
	LINK,
	PROSE,
	TABLE_RESET,
	certificateTitle,
	dayText,
	shortId,
	untilText,
} from "./parts";
import { CertificateReminderHistory, stageLabel } from "./reminder-history";
import { RecipientsBlock } from "./reminder-recipients";
import type { CertificateFleetRead } from "./use-certificates";

/* How reminders work. */

function Stages() {
	const { t } = useTranslation("devices");
	return (
		<ol
			aria-label={t("certificates.reminders.stages", "Reminder stages")}
			className="m-0 grid list-none grid-cols-4 gap-px overflow-hidden rounded-lg border border-border bg-hairline p-0 @max-[560px]/devices:grid-cols-2"
		>
			{REMINDER_STAGES.map((stage, index) => (
				<li
					key={stage.id}
					className="flex min-w-0 flex-col gap-0.75 bg-card px-3.5 py-2.5"
				>
					<span className="font-mono text-[11px] leading-3.5 font-medium text-muted-foreground">
						{stage.daysBefore
							? t(
									"certificates.reminders.stageOffset",
									"{{n, number}} · expiry − {{days, number}} d",
									{ n: index + 1, days: stage.daysBefore },
								)
							: t(
									"certificates.reminders.stageDay",
									"{{n, number}} · expiry day",
									{ n: index + 1 },
								)}
					</span>
					<b className="text-ui font-semibold">{stageLabel(t, stage.id)}</b>
					<span className="inline-flex flex-wrap items-center gap-x-2.5 gap-y-1 text-xs text-muted-foreground">
						<span className="inline-flex items-center gap-1">
							<Smartphone aria-hidden className="size-3" />
							{t("certificates.channel.push", "Push")}
						</span>
						<span className="inline-flex items-center gap-1">
							<Mail aria-hidden className="size-3" />
							{t("certificates.channel.email", "Email")}
						</span>
					</span>
				</li>
			))}
		</ol>
	);
}

function FoundCertificate({ row }: Readonly<{ row: CertificateRow }>) {
	const { t } = useTranslation("devices");
	const time = useAreaTime();
	const link = useRouteLink();
	const values = {
		name: certificateTitle(row),
		date: dayText(time, row.notAfter),
	};
	const components = {
		1: (
			<b className={row.detail ? "font-semibold" : "font-mono font-semibold"} />
		),
		2: <DeviceLink deviceId={row.device.device_id} name={row.deviceName} />,
		3: (
			<a
				{...link({
					screen: "device",
					deviceId: row.device.device_id,
					tab: "certificates",
					certificateId: row.certificateId,
				})}
				className={LINK}
			/>
		),
	};
	return isExpired(row) ? (
		<Trans
			t={t}
			i18nKey="certificates.lookup.foundExpired"
			defaults="<1>{{name}}</1> on <2/> · expired {{date}}. <3>Open it</3>"
			values={values}
			components={components}
		/>
	) : (
		<Trans
			t={t}
			i18nKey="certificates.lookup.found"
			defaults="<1>{{name}}</1> on <2/> · expires {{date}}. <3>Open it</3>"
			values={values}
			components={components}
		/>
	);
}

/** Reminders name a certificate by its ID only: paste it to find the certificate. */
function Lookup({ fleet }: Readonly<{ fleet: CertificateFleet }>) {
	const { t } = useTranslation("devices");
	const inputId = useId();
	const [query, setQuery] = useState("");
	const [asked, setAsked] = useState<string | null>(null);
	const found = useMemo(
		() => (asked === null ? null : findCertificate(fleet.rows, asked)),
		[asked, fleet.rows],
	);
	const hidden = fleet.silent
		.filter((entry) => entry.state === "noaccess")
		.map((entry) => entry.deviceName);
	const submit = (event: FormEvent) => {
		event.preventDefault();
		setAsked(query);
	};
	const tone: ResultTone = found?.kind === "found" ? "good" : "warning";
	return (
		<form onSubmit={submit} className="flex flex-col gap-1.5">
			<Label htmlFor={inputId} className="text-[13px]/[18px] font-medium">
				{t("certificates.lookup.label", "Find a certificate from a reminder")}
			</Label>
			<div className="flex flex-wrap items-center gap-2">
				<DvInput
					id={inputId}
					mono
					value={query}
					onChange={(event) => setQuery(event.target.value)}
					placeholder={t(
						"certificates.lookup.placeholder",
						"Paste the ID, for example 93bcc1ef-5f49-…",
					)}
					autoComplete="off"
					spellCheck={false}
					className="max-w-115 flex-[1_1_280px]"
				/>
				<DvButton type="submit" icon={Search}>
					{t("certificates.lookup.button", "Look up")}
				</DvButton>
			</div>
			{found ? (
				<InlineResult tone={tone} onDismiss={() => setAsked(null)}>
					{found.kind === "short" ? (
						t(
							"certificates.lookup.tooShort",
							"Paste at least the first 8 characters of the certificate ID from the reminder.",
						)
					) : found.kind === "missing" ? (
						<>
							{t(
								"certificates.lookup.missing",
								"No certificate with that ID on the devices you can see.",
							)}
							{hidden.length ? (
								<>
									{" "}
									{t(
										"certificates.lookup.maybeHidden",
										"It may be on {{devices}}, whose certificates are outside your access.",
										{ devices: hidden.slice(0, 3).join(", ") },
									)}
								</>
							) : null}
						</>
					) : (
						<FoundCertificate row={found.row} />
					)}
				</InlineResult>
			) : null}
		</form>
	);
}

function HowBlock({ fleet }: Readonly<{ fleet: CertificateFleet }>) {
	const { t } = useTranslation("devices");
	return (
		<Block
			icon={Bell}
			title={t("certificates.reminders.howTitle", "How expiry reminders work")}
			stamp={
				<FreshnessStamp
					source="hub"
					age="current"
					text={t("certificates.reminders.sentByHub", "sent by the hub")}
					noFail
				/>
			}
			bodyClassName="gap-4"
		>
			<p className={PROSE}>
				{t(
					"certificates.reminders.how",
					"The hub reminds people before a certificate expires, so nobody has to watch this page. It checks every minute and looks at each device at most every 5 minutes. Renewing, replacing or deleting a certificate cancels its remaining reminders.",
				)}
			</p>
			<Stages />
			<div className="grid grid-cols-2 gap-x-8 gap-y-3 @max-[720px]/devices:grid-cols-1">
				<div className="flex flex-col gap-1">
					<p className={LABEL}>
						{t("certificates.reminders.whoLabel", "Who receives them")}
					</p>
					<p className="text-xs text-muted-foreground">
						{t(
							"certificates.reminders.who",
							"The owner, plus people with whole-device View status or Manage certificates who accepted the share. Access to one app or service isn't enough.",
						)}
					</p>
				</div>
				<div className="flex flex-col gap-1">
					<p className={LABEL}>
						{t("certificates.reminders.whatLabel", "What they say")}
					</p>
					<p className="text-xs text-muted-foreground">
						{t(
							"certificates.reminders.what",
							'"Service certificate expires soon", with the device\'s name. They name the certificate by its ID only, because the hub never learns names, and link to its device.',
						)}
					</p>
				</div>
			</div>
			<Lookup fleet={fleet} />
		</Block>
	);
}

/* Coming up: expected times from the expiry dates and the fixed stages. */

type UpcomingLabels = Record<
	"certificate" | "device" | "next" | "after",
	string
>;

function NextReminder({ entry }: Readonly<{ entry: UpcomingReminder }>) {
	const { t } = useTranslation("devices");
	const time = useAreaTime();
	const { row } = entry;
	const [first] = entry.next;
	if (!first)
		return (
			<>
				<span className="text-muted-foreground">
					{t("certificates.upcoming.noneLeft", "None left")}
				</span>
				<CellSub>
					{t(
						"certificates.upcoming.allPassed",
						"every stage has passed; it expired {{date}}",
						{ date: dayText(time, row.notAfter) },
					)}
				</CellSub>
			</>
		);
	const distance = untilText(time, first.at);
	return (
		<>
			{stageLabel(t, first.stage)}
			{" · "}
			<span className="tabular-nums" title={time.abs(first.at)}>
				{time.at(first.at)}
			</span>
			<CellSub>
				{selfRenewing(row)
					? t(
							"certificates.upcoming.cancelledOnRenewal",
							"{{distance}} · cancelled once it renews",
							{ distance },
						)
					: distance}
			</CellSub>
		</>
	);
}

function UpcomingRow({
	entry,
	labels,
}: Readonly<{ entry: UpcomingReminder; labels: UpcomingLabels }>) {
	const { t } = useTranslation("devices");
	const { row } = entry;
	const later = entry.next.slice(1);
	const stage = (id: ReminderStage) => stageLabel(t, id);
	return (
		<Tr>
			<Td label={labels.certificate} kind="name">
				{row.detail ? (
					<b className="font-semibold">{row.detail.label}</b>
				) : (
					<span className="font-mono text-[12.5px]/[18px] font-semibold">
						{shortId(row.certificateId)}
					</span>
				)}
				<CellSub>
					{row.detail
						? t("certificates.upcoming.namedAs", "{{id}} in the reminder", {
								id: shortId(row.certificateId),
							})
						: t(
								"certificates.upcoming.namedById",
								"named by ID in the reminder",
							)}
				</CellSub>
			</Td>
			<Td label={labels.device} kind="name">
				<DeviceLink deviceId={row.device.device_id} name={row.deviceName} />
			</Td>
			<Td label={labels.next}>
				<NextReminder entry={entry} />
			</Td>
			<Td label={labels.after}>
				{later.length ? (
					<span className="flex flex-col text-xs text-muted-foreground">
						{later.map((item) => (
							<span key={item.stage}>
								{stage(item.stage)}
								{" · "}
								<DayOf at={item.at} />
							</span>
						))}
					</span>
				) : (
					<span className="text-muted-foreground">–</span>
				)}
			</Td>
		</Tr>
	);
}

function UpcomingBlock({ read }: Readonly<{ read: CertificateFleetRead }>) {
	const { t } = useTranslation("devices");
	const [all, setAll] = useState(false);
	const upcoming = useMemo(
		() => upcomingReminders(read.fleet.rows, read.now),
		[read.fleet.rows, read.now],
	);
	const shown = all ? upcoming : upcoming.slice(0, LIST_CAP);
	const labels: UpcomingLabels = {
		certificate: t("certificates.upcoming.column.certificate", "Certificate"),
		device: t("certificates.upcoming.column.device", "Device"),
		next: t("certificates.upcoming.column.next", "Next reminder"),
		after: t("certificates.upcoming.column.after", "After that"),
	};
	return (
		<Block
			icon={CalendarClock}
			title={t("certificates.upcoming.title", "Coming up")}
			count={
				read.loaded
					? upcoming.filter((entry) => entry.next.length).length
					: undefined
			}
			stamp={
				<FreshnessStamp
					source="local"
					age="current"
					text={t(
						"certificates.upcoming.workedOut",
						"worked out from expiry dates",
					)}
				/>
			}
			foot={
				<span>
					{t(
						"certificates.upcoming.foot",
						"Expected times, worked out on this computer from each certificate's expiry and the fixed stages. The hub sends each one within about 5 minutes of its time.",
					)}
				</span>
			}
			flush
		>
			{!read.loaded ? (
				<div className="p-4">
					<StateView kind="loading" rows={2} />
				</div>
			) : upcoming.length ? (
				<DvTable
					cols={["24%", "20%", "30%", "26%"]}
					className={TABLE_RESET}
					label={t(
						"certificates.upcoming.caption",
						"Expected reminders per certificate",
					)}
					head={
						<tr>
							<Th>{labels.certificate}</Th>
							<Th>{labels.device}</Th>
							<Th>{labels.next}</Th>
							<Th>{labels.after}</Th>
						</tr>
					}
				>
					{shown.map((entry) => (
						<UpcomingRow key={entry.row.key} entry={entry} labels={labels} />
					))}
				</DvTable>
			) : (
				<div className="p-4">
					<StateView
						kind="empty"
						icon={Bell}
						title={t("certificates.upcoming.empty", "No reminders coming up")}
						text={t(
							"certificates.upcoming.emptyText",
							"No device has reported a certificate, so there is nothing to remind about.",
						)}
					/>
				</div>
			)}
			{upcoming.length > shown.length ? (
				<div className="flex flex-wrap items-center justify-between gap-2 border-t border-hairline px-4 py-2.5 text-xs text-muted-foreground">
					<span>
						{t(
							"certificates.upcoming.showing",
							"Showing {{shown, number}} of {{count, number}}",
							{ shown: shown.length, count: upcoming.length },
						)}
					</span>
					<DvButton size="sm" onClick={() => setAll(true)}>
						{t("certificates.upcoming.more", "Show {{count, number}} more", {
							count: upcoming.length - shown.length,
						})}
					</DvButton>
				</div>
			) : null}
		</Block>
	);
}

/* Sent reminders, Mute and Send test (BG26). */

/** Devices whose reminders the viewer can see, the most urgent certificate's device first. */
function reminderDevices(fleet: CertificateFleet) {
	const hidden = new Set(
		fleet.silent
			.filter((entry) => entry.state === "noaccess")
			.map((entry) => entry.device.device_id),
	);
	const urgent = fleet.rows.map((row) => row.device);
	return [...new Set([...urgent, ...fleet.devices])]
		.filter((device) => !hidden.has(device.device_id))
		.map((device) => ({ id: device.device_id, name: deviceName(device) }));
}

function SentBlock({ fleet }: Readonly<{ fleet: CertificateFleet }>) {
	const { t } = useTranslation("devices");
	const options = useMemo(() => reminderDevices(fleet), [fleet]);
	const [chosen, setChosen] = useState<string | null>(null);
	const deviceId =
		options.find((option) => option.id === chosen)?.id ?? options[0]?.id;
	const read = useCertificateNotices(deviceId);
	if (!deviceId) return null;
	if (read.missingOnHub)
		return (
			<Block
				icon={BellOff}
				title={t("certificates.sent.interimTitle", "Muting and test reminders")}
				stamp={
					<FreshnessStamp
						source="hub"
						age="unsupported"
						text={t("certificates.sent.notOnHub", "not available on this hub")}
						noFail
					/>
				}
				flush
			>
				<CertificateReminderHistory deviceId={deviceId} />
			</Block>
		);
	return (
		<Block
			icon={History}
			title={t("certificates.sent.title", "Sent reminders")}
			count={read.data?.length}
			stamp={<FreshnessStamp {...stampOf(read.freshness)} />}
			toolbar={
				<>
					<DeviceSelect
						label={t(
							"certificates.sent.device",
							"Device for reminder history and settings",
						)}
						devices={options}
						value={deviceId}
						onChange={setChosen}
					/>
					<span className="text-xs text-muted-foreground">
						{t(
							"certificates.sent.perDevice",
							"The hub keeps reminders per device. Choose one to see what it sent you.",
						)}
					</span>
				</>
			}
			foot={
				<span>
					{t(
						"certificates.sent.foot",
						"Sent by the hub to you. Muting and test reminders apply to every certificate on the chosen device.",
					)}
				</span>
			}
			flush
		>
			<CertificateReminderHistory key={deviceId} deviceId={deviceId} />
		</Block>
	);
}

/** SPEC §5.8 Reminders: what the hub sends before a certificate expires, to whom, and what it sent. */
export function RemindersTab({
	read,
}: Readonly<{ read: CertificateFleetRead }>) {
	return (
		<>
			<HowBlock fleet={read.fleet} />
			<RecipientsBlock read={read} />
			<UpcomingBlock read={read} />
			{read.loaded ? <SentBlock fleet={read.fleet} /> : null}
		</>
	);
}
