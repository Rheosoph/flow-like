"use client";

import { useTranslation } from "@flow-like/locales";
import { useQuery } from "@tanstack/react-query";
import {
	CircleCheck,
	CircleDashed,
	Download,
	Ellipsis,
	ExternalLink,
	Hourglass,
	KeyRound,
	RefreshCw,
	Share2,
	Trash2,
	UserPlus,
} from "lucide-react";
import { useCallback, useEffect, useRef, useState } from "react";
import { deviceName } from "../../../../lib/device-management/model/device-view";
import { presence } from "../../../../lib/device-management/model/presence";
import type {
	AccessRequestRecord,
	DeviceRow,
	MyAccess,
} from "../../../../lib/device-management/model/types";
import {
	accessRequestFileName,
	accessRequestFileText,
} from "../../../../lib/device-management/sharing";
import { readDeviceVault } from "../../../../lib/device-management/storage";
import type {
	Capability,
	Ed25519PublicKey,
	InventoryScope,
	ManagementGrant,
} from "../../../../lib/device-management/types";
import {
	DropdownMenu,
	DropdownMenuContent,
	DropdownMenuItem,
	DropdownMenuSeparator,
	DropdownMenuTrigger,
} from "../../../ui/dropdown-menu";
import { enumLabel } from "../copy/enum-labels";
import { gateCopy } from "../copy/gate-copy";
import { type DevicesT, useAreaTime } from "../primitives/area-context";
import { Block } from "../primitives/block";
import { DvButton } from "../primitives/dv-button";
import { CellSub, DvTable, Td, Th, Tr } from "../primitives/dv-table";
import { FreshnessStamp } from "../primitives/freshness-stamp";
import { InlineResult } from "../primitives/inline-result";
import { PresenceGlyph } from "../primitives/presence-glyph";
import { StateView } from "../primitives/state-view";
import { StatusChip } from "../primitives/status-chip";
import { cx } from "../primitives/tone";
import { useRouteLink } from "../routing/use-devices-route";
import { stampOf } from "../shell/attention-popover";
import { useDeviceWorkspace } from "../workspace/device-workspace-provider";
import { useOverlay } from "../workspace/overlay-store";
import { useAttentionState } from "../workspace/use-attention";
import { useDeviceAction } from "../workspace/use-device-action";
import { type HubRead, useDeviceRows, useMyAccess } from "../workspace/use-hub";
import { useKeySession } from "../workspace/use-keys";
import {
	AccessPerson,
	EndsCell,
	KeyFingerprint,
	LINK,
	LINK_BUTTON,
	OBJECT_LINK,
	PermissionsCell,
	ScopeLabel,
	TABLE_RESET,
	saveTextFile,
} from "./access-parts";
import type { FleetAccess, PersonNames } from "./use-access";

const MENU_ITEM = "text-[13px]/[18px] focus:bg-row-hover focus:text-foreground";
const ENDING_SOON_S = 24 * 3600;
/** A long label wraps inside a narrow Actions column instead of widening the table. */
const WRAPPING_BUTTON =
	"h-auto min-h-7 max-w-full py-1 text-left whitespace-normal";

/** The viewer's own access to a shared device, from the best source that can say. */
export interface OwnAccess {
	scope: InventoryScope;
	capabilities: Capability[];
	expiresAt: number;
	/** The device has applied the rules that carry it. */
	applied: boolean;
}

interface OwnAccessRead {
	access: OwnAccess | undefined;
	/** The end from the device row when nothing else is known. */
	endsAt: number | undefined;
	read: HubRead<MyAccess>;
}

function accessFromHub(mine: MyAccess | undefined): OwnAccess | undefined {
	const grant = mine?.grants[0];
	if (!mine || !grant) return undefined;
	return {
		scope: grant.scope,
		capabilities: grant.capabilities,
		expiresAt: grant.expires_at,
		applied: mine.applied,
	};
}

function accessFromRules(
	grant: ManagementGrant | undefined,
): OwnAccess | undefined {
	if (!grant) return undefined;
	return {
		scope: grant.scope,
		capabilities: grant.capabilities,
		expiresAt: grant.expires_at,
		applied: true,
	};
}

function endOf(access: OwnAccess | undefined, row: DeviceRow) {
	if (access) return access.expiresAt;
	return row.access_expires_at ?? undefined;
}

/**
 * BG22: the hub's `my-access`; on a hub without it, the grant read from the
 * device's rules after unlock. `expiresAt` alone (BG1) comes from the device row.
 */
export function useOwnAccess(row: DeviceRow): OwnAccessRead {
	const { input } = useAttentionState();
	const read = useMyAccess(row.device_id);
	const fromRules = accessFromRules(
		input.fleet[row.device_id]?.policy?.myGrant,
	);
	const access = accessFromHub(read.data) ?? fromRules;
	return { access, endsAt: endOf(access, row), read };
}

/** The owner key this computer pinned when its request was made: public, so it reads while the keys are locked. */
export function useOwnerKey(
	deviceId: string,
	hasKeys: boolean,
): Ed25519PublicKey | undefined {
	const workspace = useDeviceWorkspace();
	const { scope } = workspace.deps;
	const query = useQuery({
		queryKey: ["devices", workspace.scopeKey, "owner-key", deviceId],
		queryFn: async () =>
			(await readDeviceVault(scope, deviceId))?.ownerControllerKey ?? null,
		enabled: hasKeys,
		staleTime: Number.POSITIVE_INFINITY,
	});
	return query.data ?? undefined;
}

interface RowResult {
	tone: "good" | "critical";
	text: string;
}

/** "Ask to renew" and "Download request again": the same public request, made again from the keys on this computer. */
export function useRequestAgain() {
	const { t } = useTranslation("devices");
	const actions = useDeviceAction();
	const mounted = useRef(true);
	useEffect(() => {
		mounted.current = true;
		return () => {
			mounted.current = false;
		};
	}, []);
	return useCallback(
		async (deviceId: string, name: string): Promise<boolean | undefined> => {
			const outcome = await actions.run<boolean>({
				action: "request_access",
				label: t("access.shared.requestAgainLabel", "Download request again"),
				resultKey: `access-request-again:${deviceId}`,
				call: async ({ workspace }) => {
					const { scope } = workspace.deps;
					const vault = await readDeviceVault(scope, deviceId);
					if (!vault || vault.grantId === "owner") return false;
					saveTextFile(
						accessRequestFileName(deviceId),
						accessRequestFileText(
							scope.account,
							vault.controllerPublic.controller_key,
							vault.grantId,
						),
					);
					return true;
				},
			});
			if (!mounted.current) return undefined;
			return outcome.status === "done" ? outcome.result : false;
		},
		[actions, t],
	);
}

function keyLine(t: DevicesT, state: string, live: boolean): string {
	if (state === "none")
		return t("devices:access.shared.keys.none", "No keys here");
	if (state === "stale")
		return t("devices:access.shared.keys.stale", "Keys here no longer work");
	if (state === "unlocked")
		return live
			? t(
					"devices:access.shared.keys.live",
					"Shared-access keys · live connection",
				)
			: t(
					"devices:access.shared.keys.unlocked",
					"Shared-access keys · unlocked",
				);
	return t("devices:access.shared.keys.locked", "Shared-access keys · locked");
}

/** What a recipient can do about a device shared with them: ask to renew, get the request again, remove the keys here. */
export function useSharedDeviceActions(row: DeviceRow, names: PersonNames) {
	const { t } = useTranslation("devices");
	const time = useAreaTime();
	const workspace = useDeviceWorkspace();
	const { input } = useAttentionState();
	const actions = useDeviceAction();
	const requestAgain = useRequestAgain();
	const [result, setResult] = useState<RowResult | null>(null);
	const name = deviceName(row);
	const owner = names(row.owner_id);
	const nowLabel = () => time.clock(Math.floor(workspace.clock.now() / 1000));

	const again = async (renewal: boolean) => {
		const made = await requestAgain(row.device_id, name);
		if (made === undefined) return;
		setResult(
			made
				? {
						tone: "good",
						text: renewal
							? t(
									"access.shared.renewReady",
									"Renewal request downloaded at {{time}}. Send it to {{owner}}; importing it again renews your access.",
									{ time: nowLabel(), owner: owner.name },
								)
							: t(
									"access.shared.againReady",
									"Request downloaded again at {{time}}. It's the same request; {{owner}} can import it any time.",
									{ time: nowLabel(), owner: owner.name },
								),
					}
				: {
						tone: "critical",
						text: t(
							"access.shared.againNoKeys",
							"This computer has no shared-access keys for {{device}}, so its request can't be made again. Restore the keys first.",
							{ device: name },
						),
					},
		);
	};

	const removeLocal = async () => {
		const backup = input.accountBackups[row.device_id];
		const outcome = await actions.run({
			action: "delete_local_keys",
			deviceId: row.device_id,
			label: t("access.shared.removeLabel", "Remove from this computer"),
			resultKey: `access-remove-local:${row.device_id}`,
			consequence: {
				what: t(
					"access.shared.removeWhat",
					"Your shared-access keys for {{device}} are deleted from this computer.",
					{ device: name },
				),
				who: t(
					"access.shared.removeWho",
					"{{owner}} isn't told. Your access stays in their rules until it ends or they remove it.",
					{ owner: owner.name },
				),
				...(backup
					? {
							stays: t(
								"access.shared.removeStays",
								"Your account backup of these keys (v{{n}}) stays unless you delete it.",
								{ n: backup.revision },
							),
						}
					: {}),
				when: t("access.shared.removeWhen", "Immediately."),
				undo: backup
					? {
							reversible: true,
							text: t(
								"access.shared.removeUndo",
								"Restore the keys from your account backup with the device password.",
							),
						}
					: {
							reversible: false,
							text: t(
								"access.shared.removeUndoNone",
								"There is no account backup of these keys. Request access again to get new ones.",
							),
						},
			},
			// SPEC §6.5: without an account backup these are the only copy, so the name is typed.
			strength: backup ? "none" : "typed",
			confirm: {
				icon: Trash2,
				title: t(
					"access.shared.removeTitle",
					"Remove {{device}} from this computer?",
					{ device: name },
				),
				sub: t("access.shared.removeSub", "Shared by {{owner}}", {
					owner: owner.name,
				}),
				tone: "danger",
				...(backup ? {} : { typed: name }),
			},
			call: ({ workspace: current }) => current.local.deleteKeys(row.device_id),
		});
		if (outcome.status === "done")
			setResult({
				tone: "good",
				text: t(
					"access.shared.removed",
					"Removed the keys for {{device}} from this computer at {{time}}.",
					{ device: name, time: nowLabel() },
				),
			});
		else if (outcome.status === "gated")
			setResult({
				tone: "critical",
				text: gateCopy(t, outcome.gate, { at: time.at }).inline,
			});
		else if (outcome.status === "failed" || outcome.status === "rejected")
			setResult({
				tone: "critical",
				text: t(
					"access.shared.removeFailed",
					"The keys for {{device}} couldn't be removed from this computer.",
					{ device: name },
				),
			});
	};

	return { again, removeLocal, result, setResult };
}

function SharedRow({
	row,
	names,
	labels,
}: Readonly<{
	row: DeviceRow;
	names: PersonNames;
	labels: Record<string, string>;
}>) {
	const { t } = useTranslation("devices");
	const time = useAreaTime();
	const link = useRouteLink();
	const overlay = useOverlay();
	const { input } = useAttentionState();
	const keys = useKeySession(row.device_id);
	const { access, endsAt, read } = useOwnAccess(row);
	const { again, removeLocal, result, setResult } = useSharedDeviceActions(
		row,
		names,
	);
	const name = deviceName(row);
	const live = input.live[row.device_id];
	const isolation = live?.inspection?.value.hostIsolation;
	const hasKeys = keys.state !== "none" && keys.state !== "stale";
	const ownerKey = useOwnerKey(row.device_id, hasKeys);
	const now = time.nowS;
	const ended = endsAt !== undefined && endsAt <= now;
	return (
		<Tr data-shared-device={row.device_id}>
			<Td label={labels.device} kind="name">
				<span className="inline-flex min-w-0 items-center gap-2">
					<PresenceGlyph kind={presence(row, now).kind} />
					<a
						{...link({ screen: "device", deviceId: row.device_id })}
						className={cx(OBJECT_LINK, "truncate text-[12.5px]")}
					>
						{name}
					</a>
				</span>
				<CellSub>
					{keyLine(t, keys.state, live?.state.kind === "live")}
					{keys.state === "locked" ? (
						<>
							{" · "}
							<button
								type="button"
								className={LINK_BUTTON}
								onClick={() => overlay.openUnlock(row.device_id)}
							>
								{t("access.action.unlock", "Unlock…")}
							</button>
						</>
					) : hasKeys ? null : (
						<>
							{" · "}
							<a
								{...link({ screen: "keys", focusDeviceId: row.device_id })}
								className={LINK}
							>
								{t("access.action.restoreKeys", "Restore keys…")}
							</a>
						</>
					)}
				</CellSub>
			</Td>
			<Td label={labels.owner}>
				<AccessPerson userId={row.owner_id} />
				{ownerKey ? (
					<CellSub>
						{t("access.shared.ownerKey", "owner key")}{" "}
						<KeyFingerprint
							value={ownerKey.x}
							copyLabel={t(
								"access.shared.copyOwnerKey",
								"Copy owner key fingerprint",
							)}
						/>
					</CellSub>
				) : null}
			</Td>
			<Td label={labels.permissions}>
				{access ? (
					<PermissionsCell
						capabilities={access.capabilities}
						isolation={isolation}
						mine
					/>
				) : (
					<span data-own-access="unknown" className="text-muted-foreground">
						{read.loading
							? t("access.shared.permissionsLoading", "Reading…")
							: hasKeys
								? t("access.shared.permissionsLocked", "Shows once unlocked")
								: t(
										"access.shared.permissionsNoKeys",
										"Not known without keys here",
									)}
					</span>
				)}
			</Td>
			<Td label={labels.scope}>
				{access ? (
					<ScopeLabel scope={access.scope} deviceId={row.device_id} />
				) : (
					<span className="text-muted-foreground">–</span>
				)}
			</Td>
			<Td label={labels.ends}>
				{endsAt === undefined ? (
					<span className="text-muted-foreground">–</span>
				) : (
					<EndsCell expiresAt={endsAt} soonS={ENDING_SOON_S} />
				)}
			</Td>
			<Td label={labels.status}>
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
			</Td>
			<Td label={labels.actions} kind="act">
				<span className="inline-flex flex-wrap items-center gap-1.5">
					<DvButton size="sm" icon={RefreshCw} onClick={() => void again(true)}>
						{t("access.shared.askRenew", "Ask to renew")}
					</DvButton>
					<DropdownMenu modal={false}>
						<DropdownMenuTrigger asChild>
							<DvButton
								size="sm"
								variant="ghost"
								iconOnly
								icon={Ellipsis}
								aria-label={t("access.shared.more", "More for {{device}}", {
									device: name,
								})}
							/>
						</DropdownMenuTrigger>
						<DropdownMenuContent
							align="end"
							className="border-border-strong bg-popover shadow-none backdrop-blur-none"
						>
							<DropdownMenuItem asChild className={MENU_ITEM}>
								<a {...link({ screen: "device", deviceId: row.device_id })}>
									<ExternalLink aria-hidden />
									{t("access.shared.openDevice", "Open device")}
								</a>
							</DropdownMenuItem>
							<DropdownMenuItem
								onSelect={() => void again(false)}
								className={MENU_ITEM}
							>
								<Download aria-hidden />
								{t("access.shared.requestAgain", "Download request again")}
							</DropdownMenuItem>
							<DropdownMenuItem asChild className={MENU_ITEM}>
								<a {...link({ screen: "keys", focusDeviceId: row.device_id })}>
									<KeyRound aria-hidden />
									{t("access.shared.keysFor", "Keys for this device")}
								</a>
							</DropdownMenuItem>
							<DropdownMenuSeparator />
							<DropdownMenuItem
								onSelect={() => void removeLocal()}
								className={cx(MENU_ITEM, "text-critical focus:text-critical")}
							>
								<Trash2 aria-hidden />
								{t("access.shared.removeMenu", "Remove from this computer…")}
							</DropdownMenuItem>
						</DropdownMenuContent>
					</DropdownMenu>
				</span>
				{result ? (
					<InlineResult
						tone={result.tone}
						onDismiss={() => setResult(null)}
						className="mt-1.5"
					>
						{result.text}
					</InlineResult>
				) : null}
			</Td>
		</Tr>
	);
}

/**
 * Keys on this computer for a device the hub does not list for the viewer:
 * a request the owner has not approved yet, or (`ended`) access that ran out
 * or was removed.
 */
function PendingRow({
	request,
	names,
	labels,
	ended = false,
}: Readonly<{
	request: AccessRequestRecord;
	names: PersonNames;
	labels: Record<string, string>;
	ended?: boolean;
}>) {
	const { t } = useTranslation("devices");
	const time = useAreaTime();
	const actions = useDeviceAction();
	const requestAgain = useRequestAgain();
	const [result, setResult] = useState<RowResult | null>(null);
	const owner = request.ownerId ? names(request.ownerId) : undefined;
	const ownerFirst = owner?.first ?? t("access.shared.theOwner", "the owner");
	const name =
		request.deviceName ??
		t("access.shared.unnamedDevice", "Device {{id}}", {
			id: request.deviceId.slice(0, 8),
		});

	const again = async () => {
		const made = await requestAgain(request.deviceId, name);
		if (made === undefined) return;
		setResult(
			made
				? {
						tone: "good",
						text: ended
							? t(
									"access.shared.endedAgain",
									"Request downloaded at {{time}}. Send it to {{owner}}; importing it gives you access again.",
									{
										time: time.clock(Math.floor(time.nowS)),
										owner: owner?.name ?? ownerFirst,
									},
								)
							: t(
									"access.shared.pendingAgain",
									"Request downloaded again at {{time}}. It's the same request; {{owner}} can import it any time.",
									{
										time: time.clock(Math.floor(time.nowS)),
										owner: owner?.name ?? ownerFirst,
									},
								),
					}
				: {
						tone: "critical",
						text: t(
							"access.shared.againNoKeys",
							"This computer has no shared-access keys for {{device}}, so its request can't be made again. Restore the keys first.",
							{ device: name },
						),
					},
		);
	};

	const discard = async () => {
		const outcome = await actions.run({
			action: "delete_local_keys",
			deviceId: request.deviceId,
			label: ended
				? t("access.shared.removeLabel", "Remove from this computer")
				: t("access.shared.discardLabel", "Discard request"),
			resultKey: `access-discard:${request.deviceId}`,
			consequence: {
				what: t(
					"access.shared.discardWhat",
					"The keys made for {{device}} are deleted from this computer.",
					{ device: name },
				),
				who: ended
					? t(
							"access.shared.endedWho",
							"Nobody. {{owner}} isn't told, and the access had ended already.",
							{ owner: owner?.name ?? ownerFirst },
						)
					: t(
							"access.shared.discardWho",
							"If {{owner}} approves the old request later, it gives you nothing: the keys are gone.",
							{ owner: ownerFirst },
						),
				when: t("access.shared.discardWhen", "Immediately."),
				undo: {
					reversible: false,
					text: t(
						"access.shared.discardUndo",
						"Import the connection file again to make a new request.",
					),
				},
			},
			confirm: {
				icon: Trash2,
				title: ended
					? t(
							"access.shared.removeTitle",
							"Remove {{device}} from this computer?",
							{ device: name },
						)
					: t(
							"access.shared.discardTitle",
							"Discard your request for {{device}}?",
							{ device: name },
						),
				sub: ended
					? t("access.shared.endedSub", "Access ended")
					: t("access.shared.discardSub", "Waiting since {{when}}", {
							when: time.at(request.createdAt),
						}),
				tone: "danger",
			},
			call: ({ workspace }) => workspace.local.deleteKeys(request.deviceId),
		});
		// Done takes the row away; anything else leaves it and says why.
		if (outcome.status === "gated")
			setResult({
				tone: "critical",
				text: gateCopy(t, outcome.gate, { at: time.at }).inline,
			});
		else if (outcome.status === "failed" || outcome.status === "rejected")
			setResult({
				tone: "critical",
				text: t(
					"access.shared.removeFailed",
					"The keys for {{device}} couldn't be removed from this computer.",
					{ device: name },
				),
			});
	};

	return (
		<Tr
			data-pending-request={ended ? undefined : request.deviceId}
			data-ended-access={ended ? request.deviceId : undefined}
			dim={ended}
		>
			<Td label={labels.device} kind="name">
				<span className="inline-flex min-w-0 items-center gap-2">
					<PresenceGlyph
						kind="pending"
						label={
							ended
								? t(
										"access.shared.notListedAnyMore",
										"No longer in your device list",
									)
								: t("access.shared.notListed", "Not in your device list yet")
						}
					/>
					<span className="truncate font-mono text-[12.5px] font-semibold">
						{name}
					</span>
				</span>
				<CellSub>
					{t(
						"access.shared.keys.onComputer",
						"Shared-access keys · on this computer",
					)}
				</CellSub>
			</Td>
			<Td label={labels.owner}>
				{request.ownerId ? (
					<AccessPerson userId={request.ownerId} />
				) : (
					<span className="text-muted-foreground">–</span>
				)}
			</Td>
			<Td label={labels.permissions}>
				<span className="text-muted-foreground">
					{ended
						? "–"
						: t(
								"access.shared.pendingPermissions",
								"{{owner}} chooses them when approving",
								{ owner: ownerFirst },
							)}
				</span>
			</Td>
			<Td label={labels.scope}>
				<span className="text-muted-foreground">–</span>
			</Td>
			<Td label={labels.ends}>
				<span className="text-muted-foreground">–</span>
			</Td>
			<Td label={labels.status}>
				{ended ? (
					<>
						<StatusChip tone="unknown" icon={CircleDashed}>
							{t("access.shared.endedChip", "Access ended")}
						</StatusChip>
						<CellSub>
							{t(
								"access.shared.endedText",
								"It left your device list: the access ran out or {{owner}} removed it.",
								{ owner: ownerFirst },
							)}
						</CellSub>
					</>
				) : (
					<>
						<StatusChip tone="info" icon={Hourglass}>
							{enumLabel(t, "grant", "requested")}
						</StatusChip>
						<CellSub>
							{t(
								"access.shared.pendingSub",
								"Only on this computer until {{owner}} approves.",
								{ owner: ownerFirst },
							)}
						</CellSub>
						<FreshnessStamp
							source="local"
							age="current"
							compact
							text={t("access.shared.pendingCreated", "created {{when}}", {
								when: time.at(request.createdAt),
							})}
							className="mt-1"
						/>
					</>
				)}
			</Td>
			<Td label={labels.actions} kind="act">
				<span className="inline-flex max-w-full flex-col items-start gap-1.5">
					<DvButton
						size="sm"
						icon={ended ? RefreshCw : Download}
						onClick={() => void again()}
						className={WRAPPING_BUTTON}
					>
						{ended
							? t("access.shared.askRenew", "Ask to renew")
							: t("access.shared.requestAgain", "Download request again")}
					</DvButton>
					<DvButton
						size="sm"
						variant="danger-ghost"
						onClick={() => void discard()}
						className={WRAPPING_BUTTON}
					>
						{ended
							? t("access.shared.removeMenu", "Remove from this computer…")
							: t("access.shared.discard", "Discard request…")}
					</DvButton>
				</span>
				{result ? (
					<InlineResult
						tone={result.tone}
						onDismiss={() => setResult(null)}
						className="mt-1.5"
					>
						{result.text}
					</InlineResult>
				) : null}
			</Td>
		</Tr>
	);
}

function HowTo({ onRequest }: Readonly<{ onRequest(): void }>) {
	const { t } = useTranslation("devices");
	const steps = [
		{
			id: "file",
			title: t("access.shared.how.step1", "Get their connection file."),
			hint: t(
				"access.shared.how.step1Hint",
				"The owner downloads it from their Access page. It holds no secrets.",
			),
		},
		{
			id: "request",
			title: t("access.shared.how.step2", "Create your request here."),
			hint: t(
				"access.shared.how.step2Hint",
				"You check the owner's key fingerprint with them and set a device password. The app makes new keys that stay on this computer.",
			),
		},
		{
			id: "send",
			title: t("access.shared.how.step3", "Send the request file back."),
			hint: t(
				"access.shared.how.step3Hint",
				"The device appears here as Waiting for approval. Once the owner adds you and the device applies it, it moves to your device list.",
			),
		},
	];
	return (
		<section
			aria-labelledby="access-how-heading"
			className="flex flex-col gap-3.5 border-t border-hairline px-4 pt-5 pb-1 text-sm/5"
		>
			<h2
				id="access-how-heading"
				className="inline-flex items-center gap-2 text-[15px]/5 font-semibold"
			>
				<UserPlus aria-hidden className="size-4 text-ink-2" />
				{t("access.shared.how.title", "Ask for access to someone's device")}
			</h2>
			<ol className="flex max-w-[84ch] flex-col gap-3.5">
				{steps.map((step, index) => (
					<li
						key={step.id}
						className="grid grid-cols-[26px_minmax(0,1fr)] items-start gap-2.5"
					>
						<span className="inline-flex size-6 items-center justify-center rounded-full border border-border-strong font-mono text-xs font-medium text-ink-2">
							{index + 1}
						</span>
						<span className="min-w-0">
							<b className="font-semibold">{step.title}</b>
							<span className="block text-xs/4 text-muted-foreground">
								{step.hint}
							</span>
						</span>
					</li>
				))}
			</ol>
			<DvButton icon={Share2} className="self-start" onClick={onRequest}>
				{t("access.action.request", "Request shared access")}
			</DvButton>
		</section>
	);
}

const ROW_CAP = 30;

/** SPEC §5.7 Shared with me: devices others share with the viewer, requests still waiting, and how to ask. */
export function SharedWithMeTab({
	fleet,
	names,
	onRequest,
	onCloud,
}: Readonly<{
	fleet: FleetAccess;
	names: PersonNames;
	onRequest(): void;
	/** Opens the Cloud approvals & spending tab. */
	onCloud(): void;
}>) {
	const { t } = useTranslation("devices");
	const time = useAreaTime();
	const { input } = useAttentionState();
	const rowsRead = useDeviceRows();
	const { facts } = useDeviceWorkspace();
	const [all, setAll] = useState(false);
	const listed = new Set(input.devices.map((row) => row.device_id));
	const requests = input.accessRequests ?? [];
	// Keeps the name and owner of shared devices with keys here, so a device that leaves the list can still be named.
	useEffect(() => {
		for (const row of fleet.shared) {
			const record = requests.find(
				(request) => request.deviceId === row.device_id,
			);
			const name = deviceName(row);
			if (
				record &&
				(record.deviceName !== name || record.ownerId !== row.owner_id)
			)
				facts.recordAccessRequest({
					deviceId: row.device_id,
					deviceName: name,
					ownerId: row.owner_id,
				});
		}
	}, [fleet.shared, requests, facts]);
	const pending = requests.filter((request) => !request.approved);
	// Approved once, gone from the list now: the access ran out or was removed.
	const ended = requests.filter(
		(request) => request.approved && !listed.has(request.deviceId),
	);
	const total = fleet.shared.length + pending.length + ended.length;
	const labels = {
		device: t("access.shared.col.device", "Device"),
		owner: t("access.shared.col.owner", "Owner"),
		permissions: t("access.shared.col.permissions", "Your permissions"),
		scope: t("access.shared.col.scope", "Applies to"),
		ends: t("access.shared.col.ends", "Ends"),
		status: t("access.shared.col.status", "Status"),
		actions: t("access.shared.col.actions", "Actions"),
	};
	const listStamp = stampOf(rowsRead.freshness);
	const stamp = (
		<FreshnessStamp
			{...listStamp}
			{...(listStamp.observedAt === undefined
				? {}
				: {
						text: t("access.shared.listStamp", "device list checked {{ago}}", {
							ago: time.ago(Math.min(listStamp.observedAt, time.nowS)),
						}),
					})}
		/>
	);
	const shown = all ? fleet.shared : fleet.shared.slice(0, ROW_CAP);
	const empty = (
		<StateView
			kind="empty"
			icon={Share2}
			title={t("access.shared.empty", "Nothing is shared with you")}
			text={t(
				"access.shared.emptyText",
				"When someone adds you to one of their devices, it appears here with the permissions they gave you.",
			)}
			className={total ? undefined : "mx-4 my-3.5"}
		/>
	);
	return (
		<>
			{fleet.loaded ? (
				<Block
					id="access-shared"
					icon={Share2}
					title={t("access.shared.title", "Devices shared with you")}
					count={total}
					stamp={stamp}
					flush
					foot={t(
						"access.shared.foot",
						"Your permissions come from the access rules the owner signed. Only the owner can change them; ask them to.",
					)}
				>
					{total ? (
						<DvTable
							label={t("access.shared.tableLabel", "Devices shared with you")}
							cols={["15%", "16%", "16%", "11%", "10%", "14%", "18%"]}
							className={TABLE_RESET}
							head={
								<tr>
									<Th>{labels.device}</Th>
									<Th>{labels.owner}</Th>
									<Th>{labels.permissions}</Th>
									<Th>{labels.scope}</Th>
									<Th>{labels.ends}</Th>
									<Th>{labels.status}</Th>
									<Th>{labels.actions}</Th>
								</tr>
							}
						>
							{shown.map((row) => (
								<SharedRow
									key={row.device_id}
									row={row}
									names={names}
									labels={labels}
								/>
							))}
							{pending.map((request) => (
								<PendingRow
									key={request.deviceId}
									request={request}
									names={names}
									labels={labels}
								/>
							))}
							{ended.map((request) => (
								<PendingRow
									key={request.deviceId}
									request={request}
									names={names}
									labels={labels}
									ended
								/>
							))}
						</DvTable>
					) : (
						empty
					)}
					{fleet.shared.length > ROW_CAP ? (
						<DvButton
							size="sm"
							variant="ghost"
							className="mx-4 my-2 self-start"
							onClick={() => setAll((value) => !value)}
						>
							{all
								? t("access.shared.fewer", "Show fewer")
								: t("access.shared.showAll", "Show all {{count, number}}", {
										count: fleet.shared.length,
									})}
						</DvButton>
					) : null}
				</Block>
			) : (
				<StateView
					kind="loading"
					title={t("access.shared.loading", "Reading the device list…")}
					rows={4}
				/>
			)}
			{fleet.cloudOnly.length ? (
				<p
					data-cloud-only=""
					className="max-w-[84ch] text-[13px]/5 text-muted-foreground"
				>
					<span className="font-mono text-xs">
						{fleet.cloudOnly.map((row) => deviceName(row)).join(", ")}
					</span>
					{": "}
					{t(
						"access.shared.cloudOnly",
						"you only approved or pay for cloud access there.",
					)}{" "}
					<button type="button" className={LINK_BUTTON} onClick={onCloud}>
						{t("access.shared.seeCloud", "See Cloud approvals & spending")}
					</button>
				</p>
			) : null}
			<HowTo onRequest={onRequest} />
		</>
	);
}
