"use client";

import { useTranslation } from "@flow-like/locales";
import { useQuery } from "@tanstack/react-query";
import {
	Bell,
	BellOff,
	CircleCheck,
	CircleDashed,
	CircleX,
	Hourglass,
	type LucideIcon,
	Mail,
	RefreshCw,
	Send,
	Smartphone,
} from "lucide-react";
import { useMemo, useState } from "react";
import {
	type CertificateNotice,
	type CertificateNoticeMute,
	type HubResult,
	type TestNoticeResult,
	muteCertificateNotices,
	sendTestCertificateNotice,
	toHubError,
	unmuteCertificateNotices,
} from "../../../../lib/device-management/hub/endpoints";
import {
	deviceKeys,
	queries,
} from "../../../../lib/device-management/hub/queries";
import { deviceName } from "../../../../lib/device-management/model/device-view";
import {
	type AreaTime,
	type DevicesT,
	useAreaTime,
} from "../primitives/area-context";
import { DvButton } from "../primitives/dv-button";
import { CellSub, DvTable, Td, Th, Tr } from "../primitives/dv-table";
import { InlineResult, type ResultTone } from "../primitives/inline-result";
import { StateView } from "../primitives/state-view";
import { StatusChip } from "../primitives/status-chip";
import type { ChipTone } from "../primitives/tone";
import { useRouteLink } from "../routing/use-devices-route";
import { hubErrorCopy } from "../workspace/area-context";
import { useDeviceWorkspace } from "../workspace/device-workspace-provider";
import { inlineResultsOf, useInlineResults } from "../workspace/use-activity";
import { useAttentionInput } from "../workspace/use-attention";
import {
	actionResultKey,
	useDeviceAction,
} from "../workspace/use-device-action";
import { useCertificateNotices, useDeviceRow } from "../workspace/use-hub";
import { LIST_CAP } from "./certificates-model";
import {
	LINK_BUTTON,
	OBJECT_LINK,
	PROSE,
	TABLE_RESET,
	dayText,
	shortId,
} from "./parts";
import { useStillHere } from "./use-certificates";

const COMPACT_CAP = 5;
/** E11: one test reminder per person and device every ten minutes. */
const TEST_WAIT_MINUTES = 10;

export function stageLabel(t: DevicesT, stage: string): string {
	const labels: Record<string, string> = {
		week: t("devices:certificates.stage.week", "7 days before"),
		three_days: t("devices:certificates.stage.threeDays", "3 days before"),
		day: t("devices:certificates.stage.day", "1 day before"),
		expired: t("devices:certificates.stage.expired", "On expiry"),
		test: t("devices:certificates.stage.test", "Test"),
	};
	return labels[stage] ?? t("devices:certificates.stage.other", "Reminder");
}

export function channelLook(
	t: DevicesT,
	channel: string,
): { icon: LucideIcon; label: string } {
	if (channel === "push")
		return {
			icon: Smartphone,
			label: t("devices:certificates.channel.push", "Push"),
		};
	if (channel === "email")
		return {
			icon: Mail,
			label: t("devices:certificates.channel.email", "Email"),
		};
	return {
		icon: Bell,
		label: t("devices:certificates.channel.other", "Other"),
	};
}

function statusLook(
	t: DevicesT,
	status: string,
): { tone: ChipTone; icon: LucideIcon; label: string } {
	if (status === "sent")
		return {
			tone: "good",
			icon: CircleCheck,
			label: t("devices:certificates.noticeStatus.sent", "Sent"),
		};
	if (status === "pending")
		return {
			tone: "info",
			icon: Hourglass,
			label: t("devices:certificates.noticeStatus.pending", "Scheduled"),
		};
	if (status === "cancelled")
		return {
			tone: "outline",
			icon: CircleX,
			label: t("devices:certificates.noticeStatus.cancelled", "Cancelled"),
		};
	return {
		tone: "unknown",
		icon: CircleDashed,
		label: t("devices:certificates.noticeStatus.other", "Unknown"),
	};
}

function skipReason(t: DevicesT, reason: string): string {
	const reasons: Record<string, string> = {
		not_a_recipient: t(
			"devices:certificates.skip.notRecipient",
			"you haven't added this device's keys, so its reminders don't reach you",
		),
		not_configured: t(
			"devices:certificates.skip.notConfigured",
			"this hub doesn't send it",
		),
		no_email: t(
			"devices:certificates.skip.noEmail",
			"your account has no email address",
		),
		failed: t(
			"devices:certificates.skip.failed",
			"the hub couldn't hand it over",
		),
		throttled: t(
			"devices:certificates.skip.throttled",
			"too many messages went out recently",
		),
	};
	return (
		reasons[reason] ??
		t("devices:certificates.skip.other", "the hub skipped it")
	);
}

interface Note {
	tone: ResultTone;
	text: string;
}

/** What a test reminder did, channel by channel. */
function testNote(t: DevicesT, time: AreaTime, result: TestNoticeResult): Note {
	const list = (items: string[]) =>
		new Intl.ListFormat(time.locale, {
			style: "long",
			type: "conjunction",
		}).format(items);
	const sent = result.sent.map((channel) => channelLook(t, channel).label);
	const skipped = result.skipped.map((entry) =>
		t("devices:certificates.test.skipped", "{{channel}}: {{reason}}", {
			channel: channelLook(t, entry.channel).label,
			reason: skipReason(t, entry.reason),
		}),
	);
	if (!sent.length)
		return {
			tone: "warning",
			text: t(
				"devices:certificates.test.none",
				"No test reminder was sent. {{skipped}}.",
				{ skipped: skipped.join(". ") },
			),
		};
	const done = t(
		"devices:certificates.test.sent",
		"Test reminder sent by {{channels}} at {{time}}. It can take a minute to arrive.",
		{ channels: list(sent), time: time.clock(time.nowS) },
	);
	return skipped.length
		? {
				tone: "warning",
				text: t(
					"devices:certificates.test.partly",
					"{{done}} Not sent: {{skipped}}.",
					{ done, skipped: skipped.join(". ") },
				),
			}
		: { tone: "good", text: done };
}

/** E11 answers 429 while the ten minutes run; the wait is stated, never a code. */
function waitNote(t: DevicesT, retryAfterS: number | undefined): Note {
	return {
		tone: "warning",
		text:
			retryAfterS === undefined
				? t(
						"devices:certificates.test.waitFixed",
						"The hub sends one test reminder per device every {{minutes, number}} minutes. Try again a little later.",
						{ minutes: TEST_WAIT_MINUTES },
					)
				: t("devices:certificates.test.wait", {
						count: Math.max(1, Math.ceil(retryAfterS / 60)),
						minutes: TEST_WAIT_MINUTES,
						defaultValue_one:
							"The hub sends one test reminder per device every {{minutes, number}} minutes. Try again in {{count, number}} minute.",
						defaultValue_other:
							"The hub sends one test reminder per device every {{minutes, number}} minutes. Try again in {{count, number}} minutes.",
					}),
	};
}

/** A hub without the route answered: nothing happened, and the result says so instead of reading as done. */
function notOnHubNote(t: DevicesT): Note {
	return {
		tone: "warning",
		text: t(
			"devices:certificates.reminders.notOnHub",
			"This hub can't do that yet. Nothing changed.",
		),
	};
}

/** The viewer's mutes; asked only once the hub answered the reminder list, so an older hub isn't asked at all. */
function useNoticeMutes(deviceId: string, enabled: boolean) {
	const { hub } = useDeviceWorkspace();
	const query = useQuery({
		...queries.certNoticeMutes(hub, deviceId),
		enabled,
	});
	const read = query.data as HubResult<CertificateNoticeMute[]> | undefined;
	return read?.kind === "ok" ? read.data : undefined;
}

function muteText(t: DevicesT, time: AreaTime, until: number | null): string {
	return until === null
		? t("devices:certificates.mute.untilUnmuted", "Muted until you unmute")
		: t("devices:certificates.mute.until", "Muted until {{date}}", {
				date: dayText(time, until),
			});
}

interface ReminderActionsProps {
	deviceId: string;
	device: string;
	certificateId?: string;
	mutes: readonly CertificateNoticeMute[] | undefined;
	compact: boolean;
}

/** Mute and Send test (BG26), both through the action layer. Mutes are the viewer's own. */
function ReminderActions({
	deviceId,
	device,
	certificateId,
	mutes,
	compact,
}: Readonly<ReminderActionsProps>) {
	const { t } = useTranslation("devices");
	const time = useAreaTime();
	const workspace = useDeviceWorkspace();
	const actions = useDeviceAction();
	const stillHere = useStillHere();
	const [note, setNote] = useState<Note | null>(null);
	const muteKey = `${actionResultKey("certificate_reminders", deviceId)}:mute:${certificateId ?? "*"}`;
	const testKey = `${actionResultKey("certificate_reminders", deviceId)}:test`;
	const muteResults = useInlineResults(muteKey);
	const testResults = useInlineResults(testKey);
	const { scopeKey } = workspace.hub;
	const deviceMute = mutes?.find((mute) => mute.certificate_id === null);
	const ownMute = certificateId
		? mutes?.find((mute) => mute.certificate_id === certificateId)
		: deviceMute;
	const muted = ownMute ?? deviceMute;
	/* A device-wide mute covers the certificate, so lifting it is the only way to hear about this one again. */
	const lifts = ownMute ? (certificateId ?? null) : null;
	const invalidate = [deviceKeys.certNoticeMutes(scopeKey, deviceId)];

	const say = (key: string, next: Note) => {
		inlineResultsOf(workspace).clear(key);
		setNote(next);
	};

	const mute = async () => {
		const here = stillHere();
		setNote(null);
		const outcome = await actions.run({
			action: "certificate_reminders",
			deviceId,
			label: certificateId
				? t(
						"certificates.mute.labelCertificate",
						"Mute reminders for this certificate",
					)
				: t("certificates.mute.label", "Mute reminders for {{device}}", {
						device,
					}),
			consequence: {
				what: certificateId
					? t(
							"certificates.mute.whatCertificate",
							"You stop getting expiry reminders for this certificate on {{device}}, by push and by email.",
							{ device },
						)
					: t(
							"certificates.mute.what",
							"You stop getting expiry reminders for every certificate on {{device}}, by push and by email.",
							{ device },
						),
				who: t(
					"certificates.mute.who",
					"Only you. Everyone else who receives them keeps getting them.",
				),
				stays: t(
					"certificates.mute.stays",
					"The certificates and their renewal aren't touched. This page keeps showing what expires.",
				),
				when: t("certificates.mute.when", "From now until you unmute."),
				undo: {
					reversible: true,
					text: t("certificates.mute.undo", "Unmute here at any time."),
				},
			},
			confirm: { icon: BellOff },
			resultKey: muteKey,
			invalidate,
			call: ({ workspace: target }) =>
				muteCertificateNotices(
					target.hub.api,
					target.hub.profile,
					deviceId,
					certificateId ?? null,
					null,
				),
		});
		if (!here() || outcome.status !== "done") return;
		if (outcome.result.kind !== "ok") {
			say(muteKey, notOnHubNote(t));
			return;
		}
		say(muteKey, {
			tone: "good",
			text: certificateId
				? t(
						"certificates.mute.doneCertificate",
						"Muted at {{time}}. You won't be reminded about this certificate until you unmute it.",
						{ time: time.clock(time.nowS) },
					)
				: t(
						"certificates.mute.done",
						"Muted at {{time}}. You won't be reminded about {{device}} until you unmute it.",
						{ time: time.clock(time.nowS), device },
					),
		});
	};

	const unmute = async () => {
		const here = stillHere();
		setNote(null);
		const outcome = await actions.run({
			action: "certificate_reminders",
			deviceId,
			label: t(
				"certificates.mute.unmuteLabel",
				"Unmute reminders for {{device}}",
				{
					device,
				},
			),
			resultKey: muteKey,
			invalidate,
			call: ({ workspace: target }) =>
				unmuteCertificateNotices(
					target.hub.api,
					target.hub.profile,
					deviceId,
					lifts,
				),
		});
		if (!here() || outcome.status !== "done") return;
		if (outcome.result.kind !== "ok") {
			say(muteKey, notOnHubNote(t));
			return;
		}
		say(muteKey, {
			tone: "good",
			text: t(
				"certificates.mute.unmuted",
				"Unmuted at {{time}}. Reminders reach you again from the next stage on.",
				{ time: time.clock(time.nowS) },
			),
		});
	};

	const sendTest = async () => {
		const here = stillHere();
		setNote(null);
		const outcome = await actions.run({
			action: "certificate_reminders",
			deviceId,
			label: t(
				"certificates.test.label",
				"Send a test reminder for {{device}}",
				{
					device,
				},
			),
			resultKey: testKey,
			invalidate: [["devices", scopeKey, "cert-notices", deviceId]],
			call: ({ workspace: target }) =>
				sendTestCertificateNotice(
					target.hub.api,
					target.hub.profile,
					deviceId,
					"both",
				),
		});
		if (!here()) return;
		if (outcome.status === "done")
			say(
				testKey,
				outcome.result.kind === "ok"
					? testNote(t, time, outcome.result.data)
					: notOnHubNote(t),
			);
		else if (
			outcome.status === "failed" &&
			toHubError(outcome.error).code === "rate_limited"
		)
			say(testKey, waitNote(t, toHubError(outcome.error).retryAfterS));
	};

	const unmuteLabel =
		certificateId && ownMute
			? t("certificates.mute.unmuteCertificate", "Unmute this certificate")
			: t("certificates.mute.unmuteDevice", "Unmute device");
	const results = note
		? []
		: [...muteResults, ...testResults].filter(
				(result) => result.state !== "done",
			);
	return (
		<div className="flex min-w-0 flex-col gap-2">
			<div className="flex flex-wrap items-center gap-2">
				{muted ? (
					<>
						<StatusChip tone="paused" icon={BellOff}>
							{muteText(t, time, muted.until)}
						</StatusChip>
						<DvButton
							size="sm"
							icon={Bell}
							busy={actions.pending(muteKey)}
							onClick={() => void unmute()}
						>
							{unmuteLabel}
						</DvButton>
					</>
				) : (
					<DvButton
						size="sm"
						icon={BellOff}
						busy={actions.pending(muteKey)}
						onClick={() => void mute()}
					>
						{certificateId
							? t(
									"certificates.mute.buttonCertificate",
									"Mute this certificate…",
								)
							: t("certificates.mute.button", "Mute device…")}
					</DvButton>
				)}
				<DvButton
					size="sm"
					icon={Send}
					busy={actions.pending(testKey)}
					onClick={() => void sendTest()}
				>
					{t("certificates.test.button", "Send test")}
				</DvButton>
				{compact ? null : (
					<span className="text-xs text-muted-foreground">
						{t(
							"certificates.reminders.onlyYours",
							"Both apply to your own reminders for {{device}}.",
							{ device },
						)}
					</span>
				)}
			</div>
			{note ? (
				<InlineResult tone={note.tone} onDismiss={() => setNote(null)}>
					{note.text}
				</InlineResult>
			) : null}
			{results.map((result) => (
				<InlineResult
					key={result.id}
					tone={result.tone}
					onDismiss={result.dismiss}
				>
					{result.text}
				</InlineResult>
			))}
		</div>
	);
}

function whenText(t: DevicesT, time: AreaTime, notice: CertificateNotice) {
	if (notice.completed_at !== null) return time.at(notice.completed_at);
	return notice.next_attempt_at === null
		? t("devices:certificates.history.notYet", "Not sent yet")
		: t("devices:certificates.history.nextTry", "next try {{when}}", {
				when: time.at(notice.next_attempt_at),
			});
}

function HistoryTable({
	deviceId,
	device,
	notices,
	showCertificate,
}: Readonly<{
	deviceId: string;
	device: string;
	notices: readonly CertificateNotice[];
	showCertificate: boolean;
}>) {
	const { t } = useTranslation("devices");
	const time = useAreaTime();
	const link = useRouteLink();
	const names = useAttentionInput().live[deviceId]?.certificates;
	const labels = {
		certificate: t("certificates.history.column.certificate", "Certificate"),
		stage: t("certificates.history.column.stage", "Stage"),
		channel: t("certificates.history.column.channel", "Channel"),
		status: t("certificates.history.column.status", "Status"),
		when: t("certificates.history.column.when", "When"),
	};
	const rows = notices.map((notice, index) => ({
		notice,
		key: `${notice.certificate_id}/${notice.stage}/${notice.channel}/${notice.certificate_revision}/${index}`,
	}));
	return (
		<DvTable
			cols={
				showCertificate
					? ["24%", "18%", "14%", "16%", "28%"]
					: ["26%", "20%", "22%", "32%"]
			}
			className={TABLE_RESET}
			label={t(
				"certificates.history.caption",
				"Reminders the hub sent for {{device}}",
				{ device },
			)}
			head={
				<tr>
					{showCertificate ? <Th>{labels.certificate}</Th> : null}
					<Th>{labels.stage}</Th>
					<Th>{labels.channel}</Th>
					<Th>{labels.status}</Th>
					<Th>{labels.when}</Th>
				</tr>
			}
		>
			{rows.map(({ notice, key }) => {
				const channel = channelLook(t, notice.channel);
				const status = statusLook(t, notice.status);
				const ChannelIcon = channel.icon;
				const isTest = notice.stage === "test";
				const name = names?.certificates.find(
					(row) => row.certificate_id === notice.certificate_id,
				)?.label;
				return (
					<Tr key={key} data-notice={notice.stage}>
						{showCertificate ? (
							<Td label={labels.certificate} kind="name">
								{isTest ? (
									<span className="text-muted-foreground">
										{t("certificates.history.noCertificate", "No certificate")}
									</span>
								) : (
									<a
										{...link({
											screen: "device",
											deviceId,
											tab: "certificates",
											certificateId: notice.certificate_id,
										})}
										title={notice.certificate_id}
										className={
											name
												? "font-semibold hover:underline"
												: `${OBJECT_LINK} text-[12.5px]/[18px]`
										}
									>
										{name ?? shortId(notice.certificate_id)}
									</a>
								)}
							</Td>
						) : null}
						<Td label={labels.stage}>{stageLabel(t, notice.stage)}</Td>
						<Td label={labels.channel}>
							<span className="inline-flex items-center gap-1.5">
								<ChannelIcon
									aria-hidden
									className="size-3.5 text-muted-foreground"
								/>
								{channel.label}
							</span>
						</Td>
						<Td label={labels.status}>
							<StatusChip tone={status.tone} icon={status.icon}>
								{status.label}
							</StatusChip>
						</Td>
						<Td label={labels.when}>
							<span
								title={
									notice.completed_at === null
										? undefined
										: time.abs(notice.completed_at)
								}
							>
								{whenText(t, time, notice)}
							</span>
							<CellSub>
								{t("certificates.history.attempts", {
									count: notice.attempts,
									defaultValue_one: "{{count, number}} attempt",
									defaultValue_other: "{{count, number}} attempts",
								})}
							</CellSub>
						</Td>
					</Tr>
				);
			})}
		</DvTable>
	);
}

/**
 * BG26: the reminders the hub sent for one device (or one of its certificates)
 * to the viewer, with Mute and Send test. On a hub without these routes it
 * explains what reminders do instead. The block around it states source and
 * age: `useCertificateNotices(deviceId, certificateId)` shares this read.
 */
export function CertificateReminderHistory({
	deviceId,
	certificateId,
	compact = false,
}: Readonly<{
	deviceId: string;
	certificateId?: string;
	compact?: boolean;
}>) {
	const { t } = useTranslation("devices");
	const row = useDeviceRow(deviceId);
	const read = useCertificateNotices(deviceId, certificateId);
	const mutes = useNoticeMutes(deviceId, read.data !== undefined);
	const [all, setAll] = useState(false);
	const device = row ? deviceName(row) : shortId(deviceId);
	const notices = read.data;
	const cap = compact ? COMPACT_CAP : LIST_CAP;
	const shown = useMemo(
		() => (all ? (notices ?? []) : (notices ?? []).slice(0, cap)),
		[notices, all, cap],
	);
	const pad = compact ? "" : "px-4 py-3";

	if (read.missingOnHub)
		return (
			<div className={pad} data-reminders="interim">
				<p className={PROSE}>
					{t(
						"certificates.history.interim",
						"This hub can't list the reminders it already sent, mute a device or send a test reminder yet. Reminders stop on their own when a certificate is renewed, replaced or deleted, when its device is revoked, and when a person's whole-device access ends. To check that they reach you, look for a message titled \"Service certificate expires soon\" in your email or on your phone.",
					)}
				</p>
			</div>
		);
	if (read.error?.code === "forbidden")
		return (
			<div className={pad}>
				<StateView
					kind="noaccess"
					title={t(
						"certificates.history.noAccess",
						"Reminders for {{device}} aren't yours to see",
						{ device },
					)}
					text={t(
						"certificates.history.noAccessText",
						"They go to the owner and to people with whole-device View status or Manage certificates.",
					)}
				/>
			</div>
		);
	if (!notices)
		return (
			<div className={pad}>
				{read.error ? (
					<StateView
						kind="error"
						title={t(
							"certificates.history.failed",
							"Couldn't read the reminders for {{device}}",
							{ device },
						)}
						text={hubErrorCopy(t, read.error.code)}
						actions={
							<DvButton
								size="sm"
								icon={RefreshCw}
								onClick={() => void read.refetch()}
							>
								{t("certificates.action.retry", "Try again")}
							</DvButton>
						}
					/>
				) : (
					<StateView kind="loading" rows={2} />
				)}
			</div>
		);

	return (
		<div className="flex min-w-0 flex-col" data-reminders="history">
			<div
				className={compact ? "pb-2.5" : "border-b border-hairline px-4 py-2.5"}
			>
				<ReminderActions
					deviceId={deviceId}
					device={device}
					certificateId={certificateId}
					mutes={mutes}
					compact={compact}
				/>
			</div>
			{notices.length ? (
				<HistoryTable
					deviceId={deviceId}
					device={device}
					notices={shown}
					showCertificate={!certificateId}
				/>
			) : (
				<div className={pad}>
					<StateView
						kind="empty"
						icon={Bell}
						title={
							certificateId
								? t(
										"certificates.history.emptyCertificate",
										"No reminders sent for this certificate yet",
									)
								: t(
										"certificates.history.empty",
										"No reminders sent to you for {{device}} yet",
										{ device },
									)
						}
						text={t(
							"certificates.history.emptyText",
							"The first one goes out 7 days before a certificate expires.",
						)}
					/>
				</div>
			)}
			{notices.length > shown.length ? (
				<div
					className={compact ? "pt-2" : "border-t border-hairline px-4 py-2.5"}
				>
					<button
						type="button"
						onClick={() => setAll(true)}
						className={LINK_BUTTON}
					>
						{t("certificates.history.more", "Show {{count, number}} more", {
							count: notices.length - shown.length,
						})}
					</button>
				</div>
			) : null}
		</div>
	);
}
