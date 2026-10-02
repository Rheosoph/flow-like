"use client";

import { useTranslation } from "@flow-like/locales";
import {
	CircleCheck,
	CircleDashed,
	FileKey,
	Hourglass,
	RefreshCw,
	ShieldCheck,
	User,
	UserPlus,
	Users,
} from "lucide-react";
import { useMemo } from "react";
import { relationshipOf } from "../../../../lib/device-management/model/presence";
import type { DeviceRow } from "../../../../lib/device-management/model/types";
import {
	MAX_GRANTS,
	usedSlots,
} from "../../../../lib/device-management/sharing";
import {
	AccessPerson,
	EndsCell,
	GrantsTable,
	KeyFingerprint,
	KeysNotice,
	LINK,
	PermissionsCell,
	RulesPair,
	RulesUnverified,
	ScopeLabel,
	UnlockButton,
	keysNeedOf,
	useKeysGate,
	waitSentence,
} from "../access/access-parts";
import { useGrantFlows } from "../access/change-permissions-sheet";
import {
	useOwnAccess,
	useOwnerKey,
	useSharedDeviceActions,
} from "../access/shared-with-me-tab";
import {
	type DeviceAccess,
	useDeviceAccess,
	usePersonNames,
	useRulesRead,
	useRulesUnverified,
} from "../access/use-access";
import { DeviceCloudApprovals } from "../cloud/device-cloud-approvals";
import { enumLabel } from "../copy/enum-labels";
import { useAreaTime } from "../primitives/area-context";
import { Block } from "../primitives/block";
import { DvButton } from "../primitives/dv-button";
import { FreshnessStamp } from "../primitives/freshness-stamp";
import { GatedAction } from "../primitives/gate-notice";
import { InlineResult } from "../primitives/inline-result";
import { KeyValueList, KvRow } from "../primitives/key-value-list";
import { StateView } from "../primitives/state-view";
import { StatusChip } from "../primitives/status-chip";
import { useRouteLink } from "../routing/use-devices-route";
import type { DeviceTabProps } from "../screen-props";
import { stampOf } from "../shell/attention-popover";
import { hubErrorCopy } from "../workspace/area-context";
import { useAttentionState } from "../workspace/use-attention";
import { useDeviceRow } from "../workspace/use-hub";
import { useKeySession } from "../workspace/use-keys";

/** The people aren't known yet: never the empty state, which would say nobody else has access (R6). */
function PeopleUnknown({
	state,
	device,
	onRetry,
}: Readonly<{
	/** `checking`: the rules are here, their people are still verified with the owner key. */
	state: "reading" | "checking" | "failed";
	device: string;
	onRetry(): void;
}>) {
	const { t } = useTranslation("devices");
	if (state !== "failed")
		return (
			<StateView
				kind="loading"
				title={
					state === "reading"
						? t("access.device.peopleReading", "Reading who has access…")
						: t(
								"access.device.peopleChecking",
								"Checking the access rules with your owner key…",
							)
				}
				rows={2}
				className="mx-4 my-3"
			/>
		);
	return (
		<StateView
			kind="error"
			title={t(
				"access.device.peopleError",
				"Couldn't read who has access to {{device}}",
				{ device },
			)}
			text={t(
				"access.device.peopleErrorText",
				"The hub didn't answer, so it isn't known whether {{device}} is shared. Nothing changed on the device.",
				{ device },
			)}
			actions={
				<DvButton size="sm" onClick={onRetry}>
					{t("access.people.retry", "Try again")}
				</DvButton>
			}
			className="mx-4 my-3"
		/>
	);
}

/** Owner view: the rules of this device, its people and every change to them. */
function OwnerAccess({ device }: Readonly<{ device: DeviceAccess }>) {
	const { t } = useTranslation("devices");
	const time = useAreaTime();
	const link = useRouteLink();
	const read = useRulesRead(device.deviceId);
	const devices = useMemo(() => [device], [device]);
	const names = usePersonNames(
		t,
		useMemo(
			() => device.rows?.map((row) => row.grant.user_id) ?? [],
			[device.rows],
		),
	);
	const flows = useGrantFlows(device, devices, names);
	const gate = useKeysGate(device);
	const unverified = useRulesUnverified(device);
	const { rules, rows } = device;
	const stamp = <FreshnessStamp {...stampOf(read.freshness)} />;
	const failed = read.error && !read.data;
	return (
		<>
			<Block
				id="device-access-rules"
				icon={ShieldCheck}
				title={
					rules
						? t("access.device.rulesTitle", "Access rules v{{n}}", {
								n: rules.saved,
							})
						: t("access.device.rules", "Access rules")
				}
				stamp={stamp}
			>
				{rules ? (
					<>
						<RulesPair
							rules={rules}
							sub={
								rules.waiting ? (
									<span data-rules-waiting="">
										{rules.issuedAt === undefined
											? enumLabel(t, "accessRules", "waiting")
											: enumLabel(t, "accessRules", "waiting", {
													time: time.at(rules.issuedAt),
												})}
										{". "}
										{waitSentence(t, device.presence, time)}
									</span>
								) : (
									enumLabel(t, "accessRules", "applied")
								)
							}
						/>
						<KeyValueList>
							<KvRow label={t("access.facts.expire", "Rules expire")}>
								{rules.expiresAt === undefined ? (
									<span className="text-muted-foreground">
										{t("access.facts.unlockToSee", "Shows once unlocked")}
									</span>
								) : (
									<span
										className="tabular-nums"
										title={time.abs(rules.expiresAt)}
									>
										{time.at(rules.expiresAt)}
									</span>
								)}
							</KvRow>
							<KvRow label={t("access.facts.slots", "Access slots")}>
								{rows ? (
									<span className="tabular-nums">
										{t(
											"access.device.slots",
											"{{count, number}} of {{max, number}}",
											{ count: usedSlots(rows), max: MAX_GRANTS },
										)}
									</span>
								) : (
									<span className="text-muted-foreground">
										{keysNeedOf(device)
											? t("access.facts.unlockToSee", "Shows once unlocked")
											: t("access.facts.notKnown", "Not known yet")}
									</span>
								)}
							</KvRow>
							<KvRow label={t("access.device.whenExpire", "When they expire")}>
								{t(
									"access.device.whenExpireText",
									"Shared access ends and retained history pauses for everyone, you included.",
								)}
							</KvRow>
						</KeyValueList>
					</>
				) : rules === null ? (
					<p data-rules="none" className="text-ui">
						{t(
							"access.device.notShared",
							"Not shared. Only you can reach this device.",
						)}
					</p>
				) : failed && read.error ? (
					<StateView
						kind="error"
						title={t(
							"access.device.rulesError",
							"Couldn't read the access rules",
						)}
						text={hubErrorCopy(t, read.error.code)}
						actions={
							<DvButton size="sm" onClick={() => void read.refetch()}>
								{t("access.people.retry", "Try again")}
							</DvButton>
						}
					/>
				) : (
					<StateView
						kind="loading"
						title={t("access.device.rulesLoading", "Reading the access rules…")}
						rows={3}
					/>
				)}
				<div className="flex flex-wrap items-start gap-2">
					{rules ? (
						<GatedAction
							gate={
								gate
									? {
											kind: gate.kind,
											reason:
												gate.kind === "locked"
													? t(
															"access.device.renewLocked",
															"Unlock {{device}}: renewing signs the rules with your owner key.",
															{ device: device.name },
														)
													: gate.reason,
										}
									: null
							}
						>
							<DvButton icon={RefreshCw} onClick={flows.renewRules}>
								{t("access.action.renewRules", "Renew access rules…")}
							</DvButton>
						</GatedAction>
					) : null}
					<GatedAction gate={gate}>
						<DvButton icon={FileKey} onClick={flows.downloadConnection}>
							{t(
								"access.action.downloadConnection",
								"Download connection file…",
							)}
						</DvButton>
					</GatedAction>
					<DvButton icon={UserPlus} onClick={() => flows.addPeople()}>
						{t("access.action.addPeople", "Add people…")}
					</DvButton>
				</div>
				{flows.result}
			</Block>
			<Block
				id="device-access-people"
				icon={Users}
				title={t("access.device.people", "People")}
				count={rows?.length}
				stamp={
					<FreshnessStamp
						{...stampOf(read.freshness)}
						{...(rules && read.freshness.at !== undefined
							? {
									text: t(
										"access.device.peopleStamp",
										"access rules v{{n}} · checked {{ago}}",
										{ n: rules.saved, ago: time.ago(read.freshness.at) },
									),
								}
							: {})}
					/>
				}
				flush
				foot={
					<>
						<a {...link({ screen: "access", tab: "people" })} className={LINK}>
							{t("access.device.allPeople", "People on all your devices")}
						</a>
						<span>
							{t(
								"access.device.peopleFoot",
								"Changes are signed with your owner key and take effect when the device applies the new access rules.",
							)}
						</span>
					</>
				}
			>
				{keysNeedOf(device) && rules ? (
					<KeysNotice device={device} className="mx-4 my-3" />
				) : rows?.length ? (
					<GrantsTable
						device={device}
						rows={rows}
						names={names}
						gate={gate}
						handlers={flows.handlers}
					/>
				) : rows || rules === null ? (
					<StateView
						kind="empty"
						icon={Users}
						title={t("access.device.nobody", "Nobody else has access")}
						text={t(
							"access.device.nobodyText",
							"Add people from their access request file.",
						)}
						actions={
							<DvButton size="sm" onClick={() => flows.addPeople()}>
								{t("access.action.addPeople", "Add people…")}
							</DvButton>
						}
						className="mx-4 my-3"
					/>
				) : unverified ? (
					<RulesUnverified
						device={device.name}
						onRetry={() => void read.refetch()}
						className="mx-4 my-3"
					/>
				) : (
					<PeopleUnknown
						state={failed ? "failed" : rules ? "checking" : "reading"}
						device={device.name}
						onRetry={() => void read.refetch()}
					/>
				)}
			</Block>
			{flows.sheets}
		</>
	);
}

/** Recipient view: what the owner gave the viewer on this device. */
function RecipientAccess({ row }: Readonly<{ row: DeviceRow }>) {
	const { t } = useTranslation("devices");
	const time = useAreaTime();
	const link = useRouteLink();
	const { input } = useAttentionState();
	const keys = useKeySession(row.device_id);
	const names = usePersonNames(
		t,
		useMemo(() => [row.owner_id], [row.owner_id]),
	);
	const { access, endsAt, read } = useOwnAccess(row);
	const { again, removeLocal, result, setResult } = useSharedDeviceActions(
		row,
		names,
	);
	const isolation = input.live[row.device_id]?.inspection?.value.hostIsolation;
	const hasKeys = keys.state !== "none" && keys.state !== "stale";
	const ownerKey = useOwnerKey(row.device_id, hasKeys);
	const ended = endsAt !== undefined && endsAt <= time.nowS;
	const unknown = (
		<span data-own-access="unknown" className="text-muted-foreground">
			{read.loading
				? t("access.shared.permissionsLoading", "Reading…")
				: hasKeys
					? t("access.shared.permissionsLocked", "Shows once unlocked")
					: t("access.shared.permissionsNoKeys", "Not known without keys here")}
		</span>
	);
	return (
		<Block
			id="device-your-access"
			icon={User}
			title={t("access.device.yourAccess", "Your access")}
			stamp={<FreshnessStamp {...stampOf(read.freshness)} />}
			foot={
				<a {...link({ screen: "access", tab: "shared" })} className={LINK}>
					{t("access.device.allShared", "Everything shared with you")}
				</a>
			}
		>
			<KeyValueList>
				<KvRow label={t("access.shared.col.owner", "Owner")}>
					<AccessPerson userId={row.owner_id} showId={false} />
					{ownerKey ? (
						<>
							{" · "}
							{t("access.shared.ownerKey", "owner key")}{" "}
							<KeyFingerprint
								value={ownerKey.x}
								copyLabel={t(
									"access.shared.copyOwnerKey",
									"Copy owner key fingerprint",
								)}
							/>
						</>
					) : null}
				</KvRow>
				<KvRow label={t("access.shared.col.scope", "Applies to")}>
					{access ? (
						<ScopeLabel scope={access.scope} deviceId={row.device_id} />
					) : (
						unknown
					)}
				</KvRow>
				<KvRow label={t("access.shared.col.permissions", "Your permissions")}>
					{access ? (
						<PermissionsCell
							capabilities={access.capabilities}
							isolation={isolation}
							mine
						/>
					) : (
						unknown
					)}
				</KvRow>
				<KvRow label={t("access.shared.col.ends", "Ends")}>
					{endsAt === undefined ? (
						<span className="text-muted-foreground">
							{t(
								"access.device.endsUnknown",
								"Not reported by this hub; shows once unlocked",
							)}
						</span>
					) : (
						<EndsCell expiresAt={endsAt} soonS={24 * 3600} />
					)}
				</KvRow>
				<KvRow label={t("access.shared.col.status", "Status")}>
					{ended ? (
						<StatusChip tone="unknown" icon={CircleDashed}>
							{enumLabel(t, "grant", "expired")}
						</StatusChip>
					) : access && !access.applied ? (
						<StatusChip tone="info" icon={Hourglass}>
							{enumLabel(t, "accessRules", "waiting")}
						</StatusChip>
					) : (
						<StatusChip tone="good" icon={CircleCheck}>
							{enumLabel(t, "grant", "active")}
						</StatusChip>
					)}
				</KvRow>
			</KeyValueList>
			<div className="flex flex-wrap items-start gap-2">
				{keys.state === "locked" && !access ? (
					<UnlockButton deviceId={row.device_id} size="md" />
				) : null}
				<DvButton icon={RefreshCw} onClick={() => void again(true)}>
					{t("access.shared.askRenew", "Ask to renew")}
				</DvButton>
				<DvButton variant="danger-ghost" onClick={() => void removeLocal()}>
					{t("access.shared.removeMenu", "Remove from this computer…")}
				</DvButton>
			</div>
			{result ? (
				<InlineResult tone={result.tone} onDismiss={() => setResult(null)}>
					{result.text}
				</InlineResult>
			) : null}
		</Block>
	);
}

/**
 * SPEC §5.2 Access tab: the device-scoped view of Access. Owners see the
 * rules and the people; recipients what they were given; a revoked device and
 * a cloud-approval-only viewer only the cloud approvals.
 */
export function DeviceAccessTab(props: Readonly<DeviceTabProps>) {
	const { t } = useTranslation("devices");
	const { deviceId } = props;
	const row = useDeviceRow(deviceId);
	const { input } = useAttentionState();
	const device = useDeviceAccess(deviceId);
	if (!row || !device)
		return (
			<StateView
				kind="notloaded"
				title={t(
					"access.device.notListed",
					"This device isn't in your device list",
				)}
			/>
		);
	const relationship = relationshipOf(row, input.me);
	const approvals = <DeviceCloudApprovals {...props} />;
	if (row.status === "revoked" || relationship === "cloud_approval")
		return approvals;
	return (
		<div data-device-access={relationship} className="flex flex-col gap-4">
			{relationship === "owner" ? (
				<OwnerAccess device={device} />
			) : (
				<RecipientAccess row={row} />
			)}
			{approvals}
		</div>
	);
}
