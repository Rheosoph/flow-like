"use client";

import { Trans, useTranslation } from "@flow-like/locales";
import {
	CircleCheck,
	FileKey,
	History,
	Hourglass,
	Inbox,
	Plus,
	RefreshCw,
	Trash2,
	TriangleAlert,
	User,
	UserCheck,
	UserPlus,
	Users,
	WifiOff,
} from "lucide-react";
import { type ReactNode, useCallback, useMemo, useState } from "react";
import { presetOf } from "../../../../lib/device-management/model/permissions";
import {
	ACCESS_RULES_ENDING_SOON_S,
	type AccessChange,
	type ImportedAccessRequest,
	MAX_GRANTS,
	usedSlots,
} from "../../../../lib/device-management/sharing";
import { enumLabel } from "../copy/enum-labels";
import { type DevicesT, useAreaTime } from "../primitives/area-context";
import { Block } from "../primitives/block";
import { useConfirm } from "../primitives/confirm-sheet";
import { DvButton } from "../primitives/dv-button";
import { FreshnessStamp } from "../primitives/freshness-stamp";
import { GatedAction } from "../primitives/gate-notice";
import { InlineResult } from "../primitives/inline-result";
import { Meter } from "../primitives/meter";
import { PresenceGlyph } from "../primitives/presence-glyph";
import { StateView } from "../primitives/state-view";
import { cx } from "../primitives/tone";
import { useRouteLink } from "../routing/use-devices-route";
import { stampOf } from "../shell/attention-popover";
import { hubErrorCopy } from "../workspace/area-context";
import { useDeviceRows } from "../workspace/use-hub";
import {
	AccessPerson,
	GrantsTable,
	KeyFingerprint,
	KeysNotice,
	LINK,
	LINK_BUTTON,
	OBJECT_LINK,
	RestoreKeysLink,
	RulesPair,
	RulesUnverified,
	SandboxFact,
	type ScopeNames,
	UnlockButton,
	keysNeedOf,
	platformLabel,
	slotsGate,
	untilText,
	useKeysGate,
	useScopeNames,
	waitSentence,
} from "./access-parts";
import type { AccessWizardStart } from "./add-people-sheet";
import { nextWizardId, useGrantFlows } from "./change-permissions-sheet";
import {
	RequestFileDrop,
	type RequestFileRow,
	requestsOfRows,
	rowsOfRequests,
} from "./import-file-sheet";
import {
	type DeviceAccess,
	type FleetAccess,
	type PersonNames,
	useAccessLocal,
	useRetryRules,
	useRulesRead,
	useRulesUnverified,
} from "./use-access";

const SECTION_CAP = 8;
const LONGER_THAN_USUAL_S = 300;
/** Gated controls carry their reason underneath, so the row aligns at the top. */
const FOOT_ACTIONS = "flex min-w-0 flex-wrap items-start gap-2";

function Fact({
	label,
	children,
}: Readonly<{ label: string; children: ReactNode }>) {
	return (
		<div className="flex min-w-0 flex-col gap-1 text-ui">
			<span className="text-label font-semibold uppercase tracking-[0.06em] text-muted-foreground">
				{label}
			</span>
			{children}
		</div>
	);
}

interface CheckedNow {
	/** Unix seconds. */
	at: number;
	failed: boolean;
}

/** The dvo line under "Access rules": active, or waiting with why and a manual check. */
function RulesState({
	device,
	onCheck,
	checking,
}: Readonly<{ device: DeviceAccess; onCheck(): void; checking: boolean }>) {
	const { t } = useTranslation("devices");
	const time = useAreaTime();
	const { rules, presence } = device;
	if (!rules) return null;
	if (!rules.waiting)
		return (
			<RulesPair
				rules={rules}
				sub={
					<span className="inline-flex items-center gap-1">
						<CircleCheck aria-hidden className="size-3.25 text-good" />
						{enumLabel(t, "accessRules", "applied")}
					</span>
				}
			/>
		);
	const since = rules.issuedAt;
	const online = presence.kind === "online" || presence.kind === "late";
	const slow =
		online && since !== undefined && time.nowS - since > LONGER_THAN_USUAL_S;
	return (
		<RulesPair
			rules={rules}
			sub={
				<span data-rules-waiting="">
					{since === undefined
						? t("access.rules.waitingPlain", "Waiting for the device.")
						: t(
								"access.rules.waitingSince",
								"Waiting for device since {{since}} ({{ago}}).",
								{ since: time.at(since), ago: time.ago(since) },
							)}{" "}
					{waitSentence(t, presence, time)}{" "}
					{slow
						? t("access.rules.slow", "It's taking longer than usual.")
						: null}{" "}
					<button
						type="button"
						className={LINK_BUTTON}
						aria-busy={checking || undefined}
						onClick={onCheck}
					>
						{checking
							? t("access.rules.checking", "Checking…")
							: t("access.rules.checkNow", "Check now")}
					</button>
				</span>
			}
		/>
	);
}

function DeviceTitle({ device }: Readonly<{ device: DeviceAccess }>) {
	const link = useRouteLink();
	return (
		<span className="inline-flex min-w-0 items-center gap-2">
			<PresenceGlyph kind={device.presence.kind} />
			<a
				{...link({
					screen: "device",
					deviceId: device.deviceId,
					tab: "access",
				})}
				className={cx(OBJECT_LINK, "truncate text-sm/5")}
			>
				{device.name}
			</a>
		</span>
	);
}

/** One owned device with access rules: its rules, its people and every action on them. */
function DeviceBlock({
	device,
	devices,
	names,
}: Readonly<{
	device: DeviceAccess;
	devices: readonly DeviceAccess[];
	names: PersonNames;
}>) {
	const { t } = useTranslation("devices");
	const time = useAreaTime();
	const read = useRulesRead(device.deviceId);
	const flows = useGrantFlows(device, devices, names);
	const gate = useKeysGate(device);
	const unverified = useRulesUnverified(device);
	const [checking, setChecking] = useState(false);
	const [checked, setChecked] = useState<CheckedNow | null>(null);
	const { rules, rows } = device;
	const check = useCallback(async () => {
		if (checking) return;
		setChecking(true);
		try {
			await read.refetch();
		} finally {
			setChecking(false);
			setChecked({ at: Math.floor(time.nowS), failed: false });
		}
	}, [checking, read, time.nowS]);
	if (!rules) return null;
	const people = rows ? usedSlots(rows) : undefined;
	const expiresAt = rules.expiresAt;
	const rulesSoon =
		expiresAt !== undefined &&
		expiresAt - time.nowS <= ACCESS_RULES_ENDING_SOON_S;
	const platform = platformLabel(t, device.platform);
	const besides =
		people === undefined
			? ""
			: people === 0
				? t("access.people.onlyYouSummary", "only you")
				: t("access.people.besidesYou", {
						count: people,
						defaultValue_one: "{{count, number}} person besides you",
						defaultValue_other: "{{count, number}} people besides you",
					});
	const checkFailed = checked && read.error !== undefined;
	const stamp = stampOf(read.freshness);
	const polling =
		rules.waiting &&
		stamp.observedAt !== undefined &&
		stamp.cadenceSec !== undefined
			? t(
					"access.rules.pollingStamp",
					"polling every {{count}} s · checked {{ago}}",
					{
						count: stamp.cadenceSec,
						ago: time.ago(Math.min(stamp.observedAt, time.nowS)),
					},
				)
			: undefined;
	return (
		<Block
			id={`access-device-${device.deviceId}`}
			title={<DeviceTitle device={device} />}
			summary={[platform, besides].filter(Boolean).join(" · ")}
			stamp={
				<FreshnessStamp {...stamp} {...(polling ? { text: polling } : {})} />
			}
			flush
			foot={
				<div className={FOOT_ACTIONS}>
					<GatedAction
						gate={
							gate
								? {
										kind: gate.kind,
										reason:
											gate.kind === "locked"
												? t(
														"access.gate.renewRules",
														"Unlock first: renewing re-signs the rules.",
													)
												: gate.reason,
									}
								: null
						}
					>
						<DvButton size="sm" icon={RefreshCw} onClick={flows.renewRules}>
							{t("access.action.renewRules", "Renew access rules…")}
						</DvButton>
					</GatedAction>
					<GatedAction gate={gate}>
						<DvButton
							size="sm"
							icon={FileKey}
							onClick={flows.downloadConnection}
						>
							{t(
								"access.action.downloadConnection",
								"Download connection file…",
							)}
						</DvButton>
					</GatedAction>
					<GatedAction gate={slotsGate(t, people)}>
						<DvButton
							size="sm"
							icon={UserPlus}
							onClick={() => flows.addPeople()}
						>
							{t("access.action.addPeople", "Add people…")}
						</DvButton>
					</GatedAction>
				</div>
			}
		>
			<div className="grid gap-x-6 gap-y-3 border-b border-hairline px-4 py-3.5 @min-[900px]/devices:grid-cols-[minmax(0,1.5fr)_repeat(3,minmax(0,1fr))] @min-[560px]/devices:grid-cols-2">
				<Fact label={t("access.facts.rules", "Access rules")}>
					<RulesState
						device={device}
						onCheck={() => void check()}
						checking={checking}
					/>
					{checked ? (
						<InlineResult
							tone={
								checkFailed ? "warning" : rules.waiting ? "unknown" : "good"
							}
							onDismiss={() => setChecked(null)}
						>
							{checkFailed
								? t(
										"access.rules.checkFailed",
										"Couldn't check at {{time}}. The last check said {{device}} uses v{{n}}.",
										{
											time: time.clock(checked.at),
											device: device.name,
											n: rules.applied,
										},
									)
								: rules.waiting
									? t(
											"access.rules.checkedWaiting",
											"Checked at {{time}}: {{device}} still uses v{{n}}. It picks up new rules at its next check-in.",
											{
												time: time.clock(checked.at),
												device: device.name,
												n: rules.applied,
											},
										)
									: t(
											"access.rules.checkedApplied",
											"Checked at {{time}}: {{device}} uses v{{n}}.",
											{
												time: time.clock(checked.at),
												device: device.name,
												n: rules.applied,
											},
										)}
						</InlineResult>
					) : null}
				</Fact>
				<Fact label={t("access.facts.expire", "Rules expire")}>
					{expiresAt === undefined ? (
						<span className="text-muted-foreground">
							{t("access.facts.unlockToSee", "Shows once unlocked")}
						</span>
					) : (
						<>
							<span className="tabular-nums" title={time.abs(expiresAt)}>
								{time.at(expiresAt)}
							</span>
							<span className="text-xs text-muted-foreground">
								{t(
									"access.facts.expireHint",
									"{{when}} · re-signed for 31 days with every change",
									{ when: untilText(time, expiresAt) },
								)}
							</span>
						</>
					)}
				</Fact>
				<Fact label={t("access.facts.slots", "Access slots")}>
					{people === undefined ? (
						<span className="text-muted-foreground">
							{keysNeedOf(device)
								? t("access.facts.unlockToSee", "Shows once unlocked")
								: t("access.facts.notKnown", "Not known yet")}
						</span>
					) : (
						<>
							<span className="tabular-nums">
								{t(
									"access.facts.slotsUsed",
									"{{count, number}} of {{max, number}} used",
									{ count: people, max: MAX_GRANTS },
								)}
							</span>
							<Meter
								segments={[{ value: (people / MAX_GRANTS) * 100 }]}
								label={t(
									"access.facts.slotsLabel",
									"{{count, number}} of {{max, number}} access slots used",
									{ count: people, max: MAX_GRANTS },
								)}
								className="max-w-30"
							/>
						</>
					)}
				</Fact>
				<Fact label={t("access.facts.code", "Code on this device")}>
					<SandboxFact isolation={device.isolation} />
				</Fact>
			</div>
			{rulesSoon && expiresAt !== undefined ? (
				<p
					data-rules-soon=""
					className="flex items-start gap-1.5 border-b border-hairline bg-surface-sunken px-4 py-2 text-xs/4 text-warning"
				>
					<TriangleAlert aria-hidden className="mt-0.5 size-3 shrink-0" />
					{t(
						"access.facts.rulesSoon",
						"The rules expire {{when}}. Then shared access ends and retained history pauses for everyone, you included. Renew the access rules to keep them.",
						{ when: untilText(time, expiresAt) },
					)}
				</p>
			) : null}
			{keysNeedOf(device) ? (
				<KeysNotice device={device} className="mx-4 mb-3" />
			) : null}
			{rows ? (
				rows.length ? (
					<GrantsTable
						device={device}
						rows={rows}
						names={names}
						gate={gate}
						handlers={flows.handlers}
					/>
				) : (
					<StateView
						kind="empty"
						icon={Users}
						title={t(
							"access.people.nobody",
							"Nobody else has access right now",
						)}
						text={t(
							"access.people.nobodyText",
							"The access rules stay in place. Add people from their access request files.",
						)}
						className="mx-4 mb-3"
					/>
				)
			) : keysNeedOf(device) ? null : unverified ? (
				<RulesUnverified
					device={device.name}
					onRetry={() => void read.refetch()}
					className="mx-4 mb-3"
				/>
			) : (
				<StateView
					kind="loading"
					title={t(
						"access.device.peopleChecking",
						"Checking the access rules with your owner key…",
					)}
					rows={2}
					className="mx-4 mb-3"
				/>
			)}
			{flows.result ? <div className="px-4 pb-3">{flows.result}</div> : null}
			{flows.sheets}
		</Block>
	);
}

function OnlyYouRow({
	device,
	devices,
	names,
}: Readonly<{
	device: DeviceAccess;
	devices: readonly DeviceAccess[];
	names: PersonNames;
}>) {
	const { t } = useTranslation("devices");
	const time = useAreaTime();
	const flows = useGrantFlows(device, devices, names);
	const need = keysNeedOf(device);
	const { kind, since } = device.presence;
	const note =
		kind === "offline"
			? since === undefined
				? t(
						"access.onlyYou.offline",
						"Offline. Rules you save now apply when it checks in again.",
					)
				: t(
						"access.onlyYou.offlineSince",
						"Offline since {{since}}. Rules you save now apply when it checks in again.",
						{ since: time.at(since) },
					)
			: kind === "never"
				? t(
						"access.onlyYou.never",
						"Hasn't checked in yet. Rules you save now apply after its first check-in.",
					)
				: t(
						"access.onlyYou.online",
						"Online. New rules usually apply within 5 minutes.",
					);
	const lock =
		need === "locked"
			? t(
					"access.onlyYou.locked",
					"Unlock it first: rules are signed with your owner key.",
				)
			: need === "nokeys"
				? t("access.onlyYou.noKeys", "This computer has no keys for it.")
				: "";
	return (
		<li
			data-only-you={device.deviceId}
			className="flex flex-wrap items-center justify-between gap-x-3 gap-y-1 border-t border-hairline py-2 first:border-t-0 first:pt-0 last:pb-0"
		>
			<div className="min-w-0">
				<DeviceTitle device={device} />
				<p className="text-xs/4 text-muted-foreground">
					{[
						platformLabel(t, device.platform),
						[note, lock].filter(Boolean).join(" "),
					]
						.filter(Boolean)
						.join(" · ")}
				</p>
				{flows.result}
			</div>
			{need === "locked" ? (
				<UnlockButton deviceId={device.deviceId} />
			) : need === "nokeys" ? (
				<RestoreKeysLink deviceId={device.deviceId} />
			) : (
				<DvButton size="sm" icon={UserPlus} onClick={() => flows.addPeople()}>
					{t("access.action.addPeople", "Add people…")}
				</DvButton>
			)}
			{flows.sheets}
		</li>
	);
}

function OnlyYouBlock({
	list,
	devices,
	names,
	revoked,
}: Readonly<{
	list: readonly DeviceAccess[];
	devices: readonly DeviceAccess[];
	names: PersonNames;
	revoked: readonly string[];
}>) {
	const { t } = useTranslation("devices");
	const link = useRouteLink();
	const rowsRead = useDeviceRows();
	const [all, setAll] = useState(false);
	if (!list.length) return null;
	const shown = all ? list : list.slice(0, SECTION_CAP);
	return (
		<Block
			icon={User}
			title={t("access.onlyYou.title", "Only you")}
			count={list.length}
			summary={t(
				"access.onlyYou.summary",
				"No access rules yet, so nobody else can reach these devices",
			)}
			stamp={<FreshnessStamp {...stampOf(rowsRead.freshness)} />}
			foot={
				revoked.length ? (
					<span>
						<Trans
							t={t}
							i18nKey="access.onlyYou.revoked"
							count={revoked.length}
							values={{ names: revoked.slice(0, 3).join(", ") }}
							tOptions={{
								defaultValue_one:
									"<1>{{names}}</1> is revoked and can't be shared.",
								defaultValue_other:
									"{{count, number}} revoked devices can't be shared (<1>{{names}}</1>).",
							}}
							components={{
								1: <span className="font-mono font-medium text-ink-2" />,
							}}
						/>{" "}
						<a
							{...link({ screen: "fleet", view: "devices", filter: "revoked" })}
							className={LINK}
						>
							{t("access.onlyYou.seeRevoked", "See revoked devices")}
						</a>
					</span>
				) : undefined
			}
		>
			<ul className="flex flex-col text-ui">
				{shown.map((device) => (
					<OnlyYouRow
						key={device.deviceId}
						device={device}
						devices={devices}
						names={names}
					/>
				))}
			</ul>
			{list.length > SECTION_CAP ? (
				<DvButton
					size="sm"
					variant="ghost"
					className="self-start"
					onClick={() => setAll((value) => !value)}
				>
					{all
						? t("access.onlyYou.fewer", "Show fewer")
						: t("access.onlyYou.more", "Show {{count, number}} more", {
								count: list.length - SECTION_CAP,
							})}
				</DvButton>
			) : null}
		</Block>
	);
}

/* Access requests: imported files, kept on this computer (BG23 interim). */

function RequestRow({
	request,
	device,
	names,
	onReview,
	onDiscard,
}: Readonly<{
	request: ImportedAccessRequest;
	device?: DeviceAccess;
	names: PersonNames;
	onReview(): void;
	onDiscard(): void;
}>) {
	const { t } = useTranslation("devices");
	const time = useAreaTime();
	const link = useRouteLink();
	const person = names(request.userId);
	const offline = device?.presence.kind === "offline";
	const offlineSince = offline ? device?.presence.since : undefined;
	return (
		<li
			data-request={request.id}
			className="grid gap-x-6 gap-y-2.5 border-t border-hairline px-4 py-3.5 first:border-t-0 @min-[900px]/devices:grid-cols-[minmax(0,1.3fr)_minmax(0,1fr)_auto]"
		>
			<div className="flex min-w-0 flex-col gap-1">
				<p className="flex flex-wrap items-center gap-x-2 gap-y-1 text-sm/5">
					<AccessPerson userId={request.userId} showId={false} />
					<span className="text-muted-foreground">
						{t("access.requests.asksFor", "asks for access to")}
					</span>
					{device ? (
						<a
							{...link({
								screen: "device",
								deviceId: device.deviceId,
								tab: "access",
							})}
							className={OBJECT_LINK}
						>
							{device.name}
						</a>
					) : (
						<span className="text-muted-foreground">
							{t(
								"access.requests.someDevice",
								"a device you choose when adding",
							)}
						</span>
					)}
				</p>
				<p className="text-xs/4 wrap-anywhere text-muted-foreground">
					<Trans
						t={t}
						i18nKey="access.requests.importedFrom"
						defaults="Imported from <1>{{file}}</1> · {{when}} ({{ago}})"
						values={{
							file: request.file,
							when: time.at(request.importedAt),
							ago: time.ago(request.importedAt),
						}}
						components={{ 1: <span className="font-mono text-[11.5px]" /> }}
					/>
				</p>
				{offline && device ? (
					<p className="flex items-start gap-1 text-xs/4 text-muted-foreground">
						<WifiOff aria-hidden className="mt-0.5 size-3 shrink-0" />
						{offlineSince === undefined
							? t(
									"access.requests.offline",
									"{{device}} is offline. You can add {{name}} now; the change applies when it checks in again.",
									{ device: device.name, name: person.first },
								)
							: t(
									"access.requests.offlineSince",
									"{{device}} is offline since {{since}}. You can add {{name}} now; the change applies when it checks in again.",
									{
										device: device.name,
										name: person.first,
										since: time.at(offlineSince),
									},
								)}
					</p>
				) : null}
			</div>
			<div className="flex min-w-0 flex-col items-start gap-1">
				<span className="text-label font-semibold uppercase tracking-[0.06em] text-muted-foreground">
					{t("access.requests.fingerprint", "Their key fingerprint")}
				</span>
				<KeyFingerprint
					value={request.controllerKey.x}
					copyLabel={t(
						"access.requests.copyFingerprint",
						"Copy key fingerprint",
					)}
				/>
				<span className="text-xs text-muted-foreground">
					{t(
						"access.requests.compare",
						"Compare with what {{name}} reads out before you add them.",
						{ name: person.first },
					)}
				</span>
			</div>
			<div className="flex flex-wrap items-start gap-2">
				<DvButton size="sm" icon={UserCheck} onClick={onReview}>
					{t("access.requests.review", "Review and add…")}
				</DvButton>
				<DvButton size="sm" variant="ghost" onClick={onDiscard}>
					{t("access.requests.discard", "Discard request…")}
				</DvButton>
			</div>
		</li>
	);
}

function RequestsBlock({
	devices,
	names,
	onWizard,
}: Readonly<{
	devices: readonly DeviceAccess[];
	names: PersonNames;
	onWizard(start: AccessWizardStart): void;
}>) {
	const { t } = useTranslation("devices");
	const time = useAreaTime();
	const confirm = useConfirm();
	const local = useAccessLocal();
	const [result, setResult] = useState<string>();
	const [all, setAll] = useState(false);
	const byId = useMemo(
		() => new Map(devices.map((device) => [device.deviceId, device])),
		[devices],
	);
	const ownedIds = useMemo(
		() => devices.map((device) => device.deviceId),
		[devices],
	);
	const openWith = useCallback(
		(rows: readonly RequestFileRow[]) => {
			const unlocked = [
				...new Set(rows.flatMap((row) => row.deviceId ?? [])),
			].filter((id) => byId.get(id)?.keys.state === "unlocked");
			onWizard({
				id: nextWizardId(),
				mode: "add",
				step: "files",
				deviceIds: unlocked,
				files: rows,
			});
		},
		[byId, onWizard],
	);
	const onRows = useCallback(
		(rows: RequestFileRow[]) => {
			local.store.addRequests(requestsOfRows(rows, Math.floor(time.nowS)));
			openWith(rows);
		},
		[local.store, openWith, time.nowS],
	);
	const discard = async (request: ImportedAccessRequest) => {
		const person = names(request.userId);
		const answer = await confirm({
			icon: Trash2,
			title: t("access.requests.discardTitle", "Discard {{name}}'s request?", {
				name: person.name,
			}),
			sub: request.file,
			tone: "danger",
			confirmLabel: t("access.requests.discardLabel", "Discard request"),
			rows: {
				what: t(
					"access.requests.discardWhat",
					"The imported file is removed from this computer. Nothing is added to any device.",
				),
				who: t(
					"access.requests.discardWho",
					"Nobody. {{name}} isn't told; ask them for a new request if you change your mind.",
					{ name: person.first },
				),
				when: t("access.requests.discardWhen", "Immediately."),
				undo: {
					reversible: true,
					text: t("access.requests.discardUndo", "Import the file again."),
				},
			},
		});
		if (!answer.ok) return;
		local.store.removeRequests([request.id]);
		setResult(
			t(
				"access.requests.discarded",
				"Discarded {{name}}'s request at {{time}}.",
				{
					name: person.name,
					time: time.clock(Math.floor(time.nowS)),
				},
			),
		);
	};
	const { requests } = local;
	return (
		<Block
			id="access-requests"
			icon={Inbox}
			title={t("access.requests.title", "Access requests")}
			count={requests.length}
			stamp={
				<FreshnessStamp
					source="local"
					age="current"
					text={t("access.requests.stamp", "imported here · not synced")}
				/>
			}
			flush
			foot={t(
				"access.requests.foot",
				"Requests arrive as files for now. Nothing in them is secret: an account ID, a public key and a request ID.",
			)}
		>
			{requests.length ? (
				<ul className="flex flex-col text-ui">
					{(all ? requests : requests.slice(0, SECTION_CAP)).map((request) => (
						<RequestRow
							key={request.id}
							request={request}
							device={request.deviceId ? byId.get(request.deviceId) : undefined}
							names={names}
							onReview={() => openWith(rowsOfRequests([request]))}
							onDiscard={() => void discard(request)}
						/>
					))}
					{requests.length > SECTION_CAP ? (
						<li className="border-t border-hairline px-4 py-2">
							<DvButton
								size="sm"
								variant="ghost"
								onClick={() => setAll((value) => !value)}
							>
								{all
									? t("access.requests.fewer", "Show fewer")
									: t("access.requests.more", "Show {{count, number}} more", {
											count: requests.length - SECTION_CAP,
										})}
							</DvButton>
						</li>
					) : null}
				</ul>
			) : (
				<p className="px-4 py-3 text-sm/5 text-muted-foreground">
					{t(
						"access.requests.none",
						"No requests waiting. People send you an access request file after they import your device's connection file.",
					)}
				</p>
			)}
			{result ? (
				<div className="px-4 py-3">
					<InlineResult tone="good" onDismiss={() => setResult(undefined)}>
						{result}
					</InlineResult>
				</div>
			) : null}
			<RequestFileDrop
				id="access-requests-files"
				ownedIds={ownedIds}
				onRows={onRows}
				slim
				className="px-4 pt-1 pb-4"
			/>
		</Block>
	);
}

/* Recent access changes made from this computer (BG13 interim). */

function changeSummary(
	t: DevicesT,
	change: AccessChange,
	names: PersonNames,
	scopes: ScopeNames,
): string {
	if (!change.entries.length)
		return t("devices:access.history.rulesRenewed", "renewed the access rules");
	return change.entries
		.map((entry) => {
			const name = names(entry.userId).name;
			switch (entry.kind) {
				case "added":
					return entry.after
						? t(
								"devices:access.history.addedWith",
								"added {{name}} ({{preset}} on {{scope}})",
								{
									name,
									preset: enumLabel(
										t,
										"preset",
										presetOf(entry.after.capabilities).preset,
									),
									scope:
										entry.after.scope.kind === "device"
											? scopes.phrase(entry.after.scope)
											: scopes.label(entry.after.scope),
								},
							)
						: t("devices:access.history.added", "added {{name}}", { name });
				case "changed":
					return t(
						"devices:access.history.changed",
						"changed {{name}}'s access",
						{ name },
					);
				case "renewed":
					return t(
						"devices:access.history.renewed",
						"renewed {{name}}'s access",
						{ name },
					);
				default:
					return t("devices:access.history.removed", "removed {{name}}", {
						name,
					});
			}
		})
		.join("; ");
}

const HISTORY_CAP = 8;

function HistoryBlock({
	devices,
	names,
}: Readonly<{ devices: readonly DeviceAccess[]; names: PersonNames }>) {
	const { t } = useTranslation("devices");
	const time = useAreaTime();
	const link = useRouteLink();
	const scopes = useScopeNames();
	const { changes } = useAccessLocal();
	const byId = useMemo(
		() => new Map(devices.map((device) => [device.deviceId, device])),
		[devices],
	);
	const items = useMemo(
		() =>
			[...changes]
				.filter((change) => byId.has(change.deviceId))
				.sort((a, b) => b.savedAt - a.savedAt)
				.slice(0, HISTORY_CAP),
		[changes, byId],
	);
	if (!items.length) return null;
	return (
		<Block
			icon={History}
			title={t("access.history.title", "Recent access changes")}
			stamp={
				<FreshnessStamp
					source="local"
					age="current"
					text={t("access.history.stamp", "kept here · not synced")}
				/>
			}
			foot={t(
				"access.history.foot",
				"Shows changes made from this computer. Changes made from another computer aren't listed here.",
			)}
		>
			<ol className="flex flex-col text-ui">
				{items.map((change) => {
					const device = byId.get(change.deviceId) as DeviceAccess;
					const applied = (device.rules?.applied ?? 0) >= change.version;
					const Icon = applied ? CircleCheck : Hourglass;
					return (
						<li
							key={`${change.deviceId}:${change.version}`}
							data-change={`${change.deviceId}:${change.version}`}
							className="grid grid-cols-[66px_18px_minmax(0,1fr)] items-start gap-x-2.5 border-t border-hairline py-1.75 first:border-t-0 first:pt-0 last:pb-0"
						>
							<time
								className="font-mono text-xs/4.5 tabular-nums text-muted-foreground"
								title={time.abs(change.savedAt)}
							>
								{time.at(change.savedAt)}
							</time>
							<Icon
								aria-hidden
								className={cx(
									"mt-0.5 size-3.5",
									applied ? "text-good" : "text-info",
								)}
							/>
							<div className="min-w-0">
								<p className="text-ui wrap-anywhere">
									<Trans
										t={t}
										i18nKey="access.history.saved"
										defaults="You saved access rules v{{n}} on <1/>: <2/>."
										values={{ n: change.version }}
										components={{
											1: (
												<a
													{...link({
														screen: "device",
														deviceId: device.deviceId,
														tab: "access",
													})}
													className={OBJECT_LINK}
												>
													{device.name}
												</a>
											),
											2: <span>{changeSummary(t, change, names, scopes)}</span>,
										}}
									/>
								</p>
								<p className="mt-0.5 text-xs/4 text-muted-foreground">
									{applied
										? t("access.history.applied", "Applied by the device")
										: t("access.history.waiting", "Waiting for the device")}
								</p>
							</div>
						</li>
					);
				})}
			</ol>
		</Block>
	);
}

/** SPEC §5.7 People: requests, one section per shared device, the devices only you reach, and what changed here. */
export function PeopleTab({
	fleet,
	devices,
	names,
	onWizard,
}: Readonly<{
	fleet: FleetAccess;
	devices: readonly DeviceAccess[];
	names: PersonNames;
	onWizard(start: AccessWizardStart): void;
}>) {
	const { t } = useTranslation("devices");
	const link = useRouteLink();
	const rowsRead = useDeviceRows();
	const retryRules = useRetryRules();
	const [all, setAll] = useState(false);
	if (!fleet.loaded)
		return rowsRead.error ? (
			<StateView
				kind="error"
				title={t("access.people.listError", "Couldn't read the device list")}
				text={hubErrorCopy(t, rowsRead.error.code)}
				actions={
					<DvButton size="sm" onClick={() => void rowsRead.refetch()}>
						{t("access.people.retry", "Try again")}
					</DvButton>
				}
			/>
		) : (
			<StateView
				kind="loading"
				title={t("access.people.loading", "Reading the device list…")}
				rows={5}
			/>
		);
	if (!devices.length)
		return (
			<StateView
				kind="empty"
				icon={Users}
				title={t("access.people.noDevices", "You don't own any devices yet")}
				text={
					<>
						{t(
							"access.people.noDevicesText",
							"Set up a device to share it. People you add get their own keys and only the permissions you choose.",
						)}{" "}
						<FreshnessStamp {...stampOf(rowsRead.freshness)} compact />
					</>
				}
				actions={
					<DvButton size="sm" icon={Plus} asChild>
						<a {...link({ screen: "setup" })}>
							{t("access.action.setup", "Set up a device")}
						</a>
					</DvButton>
				}
			/>
		);
	const withRules = devices.filter((device) => device.rules);
	const without = devices.filter((device) => device.rules === null);
	const pending = devices.filter((device) => device.rules === undefined);
	const unread = pending.filter((device) => !device.readFailed);
	const failed = pending.filter((device) => device.readFailed);
	const shown = all ? withRules : withRules.slice(0, SECTION_CAP);
	return (
		<>
			<p className="max-w-[84ch] text-[13px]/5 text-muted-foreground">
				{t(
					"access.people.intro",
					"Each device enforces access rules that you sign with your owner key. The hub only stores and forwards them, so a change takes effect when the device applies it. Every change re-signs the rules for 31 days; if they expire, shared access ends and retained history pauses for everyone, you included.",
				)}
			</p>
			<RequestsBlock devices={devices} names={names} onWizard={onWizard} />
			{shown.map((device) => (
				<DeviceBlock
					key={device.deviceId}
					device={device}
					devices={devices}
					names={names}
				/>
			))}
			{withRules.length > SECTION_CAP ? (
				<DvButton
					size="sm"
					variant="ghost"
					className="self-start"
					onClick={() => setAll((value) => !value)}
				>
					{all
						? t("access.people.fewerDevices", "Show fewer devices")
						: t(
								"access.people.moreDevices",
								"Show all {{count, number}} shared devices",
								{ count: withRules.length },
							)}
				</DvButton>
			) : null}
			{unread.length ? (
				<StateView
					kind="loading"
					title={t("access.people.readingRules", {
						count: unread.length,
						defaultValue_one:
							"Reading the access rules of {{count, number}} device…",
						defaultValue_other:
							"Reading the access rules of {{count, number}} devices…",
					})}
					rows={2}
				/>
			) : null}
			{failed.length ? (
				<StateView
					kind="error"
					title={t("access.people.rulesError", {
						count: failed.length,
						names: failed
							.slice(0, 3)
							.map((device) => device.name)
							.join(", "),
						defaultValue_one: "Couldn't read the access rules of {{names}}",
						defaultValue_other:
							"Couldn't read the access rules of {{count, number}} devices ({{names}})",
					})}
					text={t(
						"access.people.rulesErrorText",
						"The hub didn't answer for them, so it isn't known whether they are shared. Nothing changed on the devices.",
					)}
					actions={
						<DvButton
							size="sm"
							onClick={() =>
								retryRules(failed.map((device) => device.deviceId))
							}
						>
							{t("access.people.retry", "Try again")}
						</DvButton>
					}
				/>
			) : null}
			<OnlyYouBlock
				list={without}
				devices={devices}
				names={names}
				revoked={fleet.revokedOwned.map((row) => row.display_name || row.name)}
			/>
			<HistoryBlock devices={devices} names={names} />
		</>
	);
}
