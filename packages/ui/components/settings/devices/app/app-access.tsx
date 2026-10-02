"use client";

import { useTranslation } from "@flow-like/locales";
import { useQueries } from "@tanstack/react-query";
import {
	CircleCheck,
	Hourglass,
	LockOpen,
	RefreshCw,
	TriangleAlert,
	Users,
} from "lucide-react";
import type { ReactNode } from "react";
import { useUserIdentity } from "../../../../hooks/use-user-lookup";
import type { HubResult } from "../../../../lib/device-management/hub/endpoints";
import { queries } from "../../../../lib/device-management/hub/queries";
import type { AppDeviceGroup } from "../../../../lib/device-management/model/app-plan";
import { presetOf } from "../../../../lib/device-management/model/permissions";
import type { MyAccess } from "../../../../lib/device-management/model/types";
import type {
	Capability,
	InventoryScope,
	ManagementPolicy,
	PolicyView,
} from "../../../../lib/device-management/types";
import { GrantRowActions } from "../access/change-permissions-sheet";
import { useDeviceAccess } from "../access/use-access";
import { enumLabel } from "../copy/enum-labels";
import { type DevicesT, useAreaTime } from "../primitives/area-context";
import { Block } from "../primitives/block";
import { DvButton } from "../primitives/dv-button";
import { CellSub, DvTable, Td, Th, Tr } from "../primitives/dv-table";
import { FreshnessStamp } from "../primitives/freshness-stamp";
import { StateView } from "../primitives/state-view";
import { StatusChip } from "../primitives/status-chip";
import { cx } from "../primitives/tone";
import { ACCOUNT_SCOPE } from "../routing/devices-route";
import {
	type RouteLinkProps,
	useRouteLink,
} from "../routing/use-devices-route";
import { stampOf } from "../shell/attention-popover";
import {
	useAttentionState,
	useDeviceRows,
	useDeviceWorkspace,
	useOverlay,
} from "../workspace";
import {
	LINK,
	LinkButton,
	Person,
	ShowMore,
	TABLE_RESET,
	UnknownAction,
	useAppPage,
	useCapped,
	useNameList,
} from "./app-shared";
import {
	GROUP_CAP,
	NAME_CAP,
	ROW_CAP,
	SENTENCE_NAME_CAP,
} from "./app-view-local";

const covers = (scope: InventoryScope, appId: string) =>
	scope.kind === "device" || scope.project_id === appId;

function scopeLabel(t: DevicesT, scope: InventoryScope, app: string): string {
	if (scope.kind === "device") return enumLabel(t, "scopeKind", "device");
	return scope.kind === "project"
		? enumLabel(t, "scopeKind", "project", { name: app })
		: enumLabel(t, "scopeKind", "placement", { name: scope.placement_id });
}

/** "Viewer" and " · 3 permissions": the preset leads, how many permissions it holds follows. */
function presetParts(t: DevicesT, capabilities: readonly Capability[]) {
	const { preset, count } = presetOf(capabilities);
	return {
		preset: enumLabel(t, "preset", preset),
		held: t("devices:app.access.presetCount", {
			count,
			defaultValue_one: " · {{count, number}} permission",
			defaultValue_other: " · {{count, number}} permissions",
		}),
	};
}

/** Access that ends within this many seconds is marked (PROTO: six hours). */
const ENDS_SOON_S = 6 * 3600;

interface Row {
	id: string;
	group: AppDeviceGroup;
	userId: string;
	scope: InventoryScope;
	capabilities: readonly Capability[];
	expiresAt: number;
	/** The viewer's own access to a device someone else owns. */
	mine: boolean;
	/** The access rules this row comes from, and whether the device runs them already. */
	rules: { version: number; applied: boolean };
}

interface AccessRead {
	rows: Row[];
	/** Devices of the viewer that nobody else can reach. */
	nobody: AppDeviceGroup[];
	/** Devices whose access rules can be read once they are unlocked. */
	locked: AppDeviceGroup[];
	/** Devices whose access rules aren't readable for another reason. */
	unread: AppDeviceGroup[];
	/** Shared devices whose access for the viewer the hub hasn't told. */
	sharedUnknown: AppDeviceGroup[];
}

/** The viewer's access as the hub answered it; nothing while it hasn't, or on a hub without the route. */
function okAccess(result: unknown): MyAccess | undefined {
	const read = result as HubResult<MyAccess> | undefined;
	return read?.kind === "ok" ? read.data : undefined;
}

/** One hub read per listed device: its access rules (own devices) or the viewer's access (shared ones). */
function useAccessRead(groups: readonly AppDeviceGroup[]): AccessRead {
	const { data } = useAppPage();
	const { hub } = useDeviceWorkspace();
	const { verifyPolicy } = useAttentionState();
	const time = useAreaTime();
	const owned = groups.filter((group) => group.relationship === "owner");
	const shared = groups.filter((group) => group.relationship === "shared");
	const policies = useQueries({
		queries: owned.map((group) => queries.policy(hub, group.deviceId)),
	});
	const access = useQueries({
		queries: shared.map((group) => queries.myAccess(hub, group.deviceId)),
	});
	const read: AccessRead = {
		rows: [],
		nobody: [],
		locked: [],
		unread: [],
		sharedUnknown: [],
	};
	const bucketOf = (
		group: AppDeviceGroup,
		view: PolicyView | undefined,
	): AppDeviceGroup[] | ManagementPolicy => {
		if (!view) return read.unread;
		if (!view.policy_jws) return read.nobody;
		const policy = verifyPolicy(group.deviceId, view);
		if (policy) return policy;
		const keys = data.devices.get(group.deviceId)?.keys.state;
		return keys !== "unlocked" && keys !== "none" ? read.locked : read.unread;
	};
	owned.forEach((group, index) => {
		const view = policies[index]?.data as PolicyView | undefined;
		const found = bucketOf(group, view);
		if (Array.isArray(found)) {
			found.push(group);
			return;
		}
		const grants = found.grants.filter(
			(grant) =>
				covers(grant.scope, data.appId) && grant.expires_at > time.nowS,
		);
		if (!grants.length) read.nobody.push(group);
		const rules = {
			version: found.policy_version,
			applied: view?.applied_version === view?.version,
		};
		for (const grant of grants)
			read.rows.push({
				id: grant.grant_id,
				group,
				userId: grant.user_id,
				scope: grant.scope,
				capabilities: grant.capabilities,
				expiresAt: grant.expires_at,
				mine: false,
				rules,
			});
	});
	shared.forEach((group, index) => {
		const mine = okAccess(access[index]?.data);
		const grants = mine
			? mine.grants.filter((grant) => covers(grant.scope, data.appId))
			: [];
		if (!mine || !grants.length) {
			read.sharedUnknown.push(group);
			return;
		}
		const rules = { version: mine.policy_version, applied: mine.applied };
		for (const grant of grants)
			read.rows.push({
				id: grant.grant_id,
				group,
				userId: data.me,
				scope: grant.scope,
				capabilities: grant.capabilities,
				expiresAt: grant.expires_at,
				mine: true,
				rules,
			});
	});
	const order = new Map(groups.map((group, index) => [group.deviceId, index]));
	read.rows.sort(
		(a, b) =>
			(order.get(a.group.deviceId) ?? 0) - (order.get(b.group.deviceId) ?? 0) ||
			a.expiresAt - b.expiresAt,
	);
	return read;
}

const accessLabels = (t: DevicesT) => ({
	person: t("devices:app.access.colPerson", "Person"),
	device: t("devices:app.access.colDevice", "Device"),
	scope: t("devices:app.access.colAppliesTo", "Applies to"),
	permissions: t("devices:app.access.colPermissions", "Permissions"),
	ends: t("devices:app.access.colEnds", "Ends"),
	actions: t("devices:app.access.colActions", "Actions"),
});

function SharedBy({ group }: Readonly<{ group: AppDeviceGroup }>) {
	const { t } = useTranslation("devices");
	const { data } = useAppPage();
	const owner = useUserIdentity(data.devices.get(group.deviceId)?.row.owner_id);
	if (!owner.isResolved) return null;
	return (
		<CellSub>
			{t("app.access.sharedBy", "shared by {{owner}}", { owner: owner.label })}
		</CellSub>
	);
}

/** Whether the device already runs the access rules the row comes from. */
function RulesChip({ rules }: Readonly<{ rules: Row["rules"] }>) {
	const { t } = useTranslation("devices");
	return rules.applied ? (
		<StatusChip tone="good" icon={CircleCheck}>
			{t("app.access.active", "Active · rules v{{version, number}}", {
				version: rules.version,
			})}
		</StatusChip>
	) : (
		<StatusChip tone="info" icon={Hourglass}>
			{t("app.access.waiting", "Waiting for device")}
		</StatusChip>
	);
}

function EndsCell({ row }: Readonly<{ row: Row }>) {
	const { t } = useTranslation("devices");
	const time = useAreaTime();
	const left = row.expiresAt - time.nowS;
	if (left <= 0)
		return t("app.access.ended", "Ended {{time}}", {
			time: time.at(row.expiresAt),
		});
	const soon = left <= ENDS_SOON_S;
	return (
		<>
			{time.at(row.expiresAt)}
			<CellSub
				data-ends-soon={soon || undefined}
				className={cx("flex items-center gap-1", soon && "text-warning")}
			>
				{soon ? (
					<TriangleAlert aria-hidden className="size-3 shrink-0" />
				) : null}
				{time.ago(row.expiresAt)}
			</CellSub>
		</>
	);
}

/**
 * What can be done about one row: the viewer's own access can only be asked
 * for; someone else's gets Renew… and its menu where the device's people are
 * readable here (W3-N7's actions), else the way to the device's Access tab.
 */
function RowActions({
	row,
	deviceAccess,
}: Readonly<{ row: Row; deviceAccess: RouteLinkProps }>) {
	const { t } = useTranslation("devices");
	const people = useDeviceAccess(row.group.deviceId);
	if (row.mine)
		return (
			<LinkButton
				route={{ screen: "access", tab: "shared", action: "request" }}
				scope={ACCOUNT_SCOPE}
				size="sm"
				icon={RefreshCw}
				act="ask-renew"
			>
				{t("app.access.askRenew", "Ask to renew")}
			</LinkButton>
		);
	if (people?.rows?.some((entry) => entry.grant.grant_id === row.id))
		return <GrantRowActions deviceId={row.group.deviceId} grantId={row.id} />;
	return (
		<DvButton size="sm" asChild>
			<a data-act="open-device-access" {...deviceAccess}>
				{t("app.access.openDevice", "Open device access")}
			</a>
		</DvButton>
	);
}

function AccessRow({ row }: Readonly<{ row: Row }>) {
	const { t } = useTranslation("devices");
	const { view } = useAppPage();
	const link = useRouteLink();
	const labels = accessLabels(t);
	const permissions = presetParts(t, row.capabilities);
	const { group } = row;
	const deviceAccess = link({
		screen: "device",
		deviceId: group.deviceId,
		tab: "access",
	});
	return (
		<Tr data-grant={row.id}>
			<Td label={labels.person} kind="name">
				<Person userId={row.userId} />
				{row.mine ? (
					<SharedBy group={group} />
				) : (
					<CellSub>
						<RulesChip rules={row.rules} />
					</CellSub>
				)}
			</Td>
			<Td label={labels.device} kind="mono">
				<a {...deviceAccess} className={LINK}>
					{group.name}
				</a>
			</Td>
			<Td label={labels.scope}>{scopeLabel(t, row.scope, view.app.name)}</Td>
			<Td label={labels.permissions}>
				<b className="font-semibold">{permissions.preset}</b>
				{permissions.held}
			</Td>
			<Td label={labels.ends}>
				<EndsCell row={row} />
			</Td>
			<Td label={labels.actions}>
				<RowActions row={row} deviceAccess={deviceAccess} />
			</Td>
		</Tr>
	);
}

function Note({
	kind,
	children,
	action,
}: Readonly<{ kind: string; children: ReactNode; action?: ReactNode }>) {
	return (
		<p
			data-access-note={kind}
			className="flex flex-wrap items-center gap-x-3 gap-y-1.5 border-t border-hairline px-4 py-2 text-ui text-ink-2 first:border-t-0"
		>
			<span className="min-w-0">{children}</span>
			{action}
		</p>
	);
}

/** Devices whose access rules need an unlock: one sentence with the one action that helps. */
function LockedNote({
	locked,
	names,
}: Readonly<{ locked: readonly AppDeviceGroup[]; names: string }>) {
	const { t } = useTranslation("devices");
	const { view } = useAppPage();
	const overlay = useOverlay();
	const app = view.app.name;
	const [only] = locked;
	if (!only) return null;
	if (locked.length === 1)
		return (
			<Note
				kind="locked"
				action={
					<UnknownAction
						deviceId={only.deviceId}
						unknown={{ kind: "locked" }}
						size="xs"
					/>
				}
			>
				{t(
					"app.access.locked",
					"Unlock {{device}} to see who can reach {{app}} there.",
					{ device: only.name, app },
				)}
			</Note>
		);
	return (
		<Note
			kind="locked"
			action={
				<DvButton
					size="xs"
					icon={LockOpen}
					data-act="unlock-several"
					onClick={() => overlay.openUnlockSeveral()}
				>
					{t("app.coverage.unlockSeveral", "Unlock several…")}
				</DvButton>
			}
		>
			{t(
				"app.access.lockedMany",
				"{{count, number}} devices are locked, so who can reach {{app}} there isn't known: {{names}}.",
				{ count: locked.length, app, names },
			)}
		</Note>
	);
}

/** What the table can't list, one sentence per reason (R6: never "nobody" for "not readable"). */
function AccessNotes({ read }: Readonly<{ read: AccessRead }>) {
	const { t } = useTranslation("devices");
	const nameList = useNameList();
	const names = (groups: readonly AppDeviceGroup[], cap = SENTENCE_NAME_CAP) =>
		nameList(
			groups.map((group) => group.name),
			cap,
		);
	const { locked, unread, sharedUnknown, nobody } = read;
	return (
		<>
			<LockedNote locked={locked} names={names(locked)} />
			{unread.length ? (
				<Note kind="unread">
					{t(
						"app.access.notLoaded",
						"The access rules of {{names}} aren't loaded yet.",
						{ names: names(unread) },
					)}
				</Note>
			) : null}
			{sharedUnknown.length ? (
				<Note kind="shared">
					{t("app.access.sharedNoDetail", {
						count: sharedUnknown.length,
						names: names(sharedUnknown),
						defaultValue_one:
							"{{names}} is shared with you. Unlock it to check your permissions.",
						defaultValue_other:
							"{{names}} are shared with you. Unlock them to check your permissions.",
					})}
				</Note>
			) : null}
			{nobody.length ? (
				<Note kind="nobody">
					{t("app.access.onlyYou", "Nobody else has access on {{names}}.", {
						names: names(nobody, NAME_CAP),
					})}
				</Note>
			) : null}
		</>
	);
}

/** APP §2.14: people whose access covers the app, per device it runs on. */
export function AppAccess() {
	const { t } = useTranslation("devices");
	const { view } = useAppPage();
	const deviceRows = useDeviceRows();
	const time = useAreaTime();
	const nameList = useNameList();
	const groups = view.groups.filter(
		(group) =>
			group.relationship === "owner" || group.relationship === "shared",
	);
	const devices = useCapped(groups, GROUP_CAP);
	const read = useAccessRead(devices.shown);
	const rows = useCapped(read.rows, ROW_CAP);
	if (!view.app.canReadFlows || !groups.length) return null;
	const labels = accessLabels(t);
	const unreadable =
		read.locked.length + read.unread.length + read.sharedUnknown.length;
	const empty = !read.rows.length && !unreadable && !devices.rest.length;
	const stamp = stampOf(deviceRows.freshness);
	return (
		<Block
			id="ad-access"
			icon={Users}
			title={t("app.access.title", "Access to this app")}
			{...(read.rows.length ? { count: read.rows.length } : {})}
			flush
			stamp={
				<FreshnessStamp
					{...stamp}
					{...(stamp.observedAt === undefined
						? {}
						: {
								text: t("app.access.stamp", "access rules · checked {{ago}}", {
									ago: time.ago(stamp.observedAt),
								}),
							})}
				/>
			}
			tools={
				<LinkButton
					route={{ screen: "access", tab: "people" }}
					scope={ACCOUNT_SCOPE}
					size="sm"
					variant="ghost"
					icon={Users}
					act="manage-access"
				>
					{t("app.access.all", "All access")}
				</LinkButton>
			}
			foot={
				<span>
					{t(
						"app.access.footnote",
						"Lists device access that covers this app, and access to whole devices it runs on. When access rules expire, shared access ends and retained history pauses for everyone, you included.",
					)}
				</span>
			}
		>
			{read.rows.length ? (
				<DvTable
					label={t("app.access.tableLabel", "People with access to {{app}}", {
						app: view.app.name,
					})}
					cols={["19%", "16%", "13%", "20%", "12%", "20%"]}
					className={TABLE_RESET}
					head={
						<tr>
							<Th>{labels.person}</Th>
							<Th>{labels.device}</Th>
							<Th>{labels.scope}</Th>
							<Th>{labels.permissions}</Th>
							<Th>{labels.ends}</Th>
							<Th>{labels.actions}</Th>
						</tr>
					}
				>
					{rows.shown.map((row) => (
						<AccessRow key={`${row.group.deviceId}/${row.id}`} row={row} />
					))}
				</DvTable>
			) : null}
			{rows.rest.length ? (
				<p
					data-more="rows"
					className="flex flex-wrap items-center gap-x-3 gap-y-1 border-t border-hairline px-4 py-2 text-xs text-muted-foreground"
				>
					<span>
						{t(
							"app.access.capped",
							"Showing {{shown, number}} of {{total, number}}, sorted by device.",
							{ shown: rows.shown.length, total: read.rows.length },
						)}
					</span>
					<ShowMore count={rows.rest.length} onClick={rows.showAll} />
				</p>
			) : null}
			{empty ? (
				<div className="p-3">
					<StateView
						kind="empty"
						icon={Users}
						title={t(
							"app.access.emptyTitle",
							"Nobody else has device access that covers {{app}}",
							{ app: view.app.name },
						)}
						text={t(
							"app.access.emptyText",
							"Add people from their access request file. Access can cover a whole device or one app.",
						)}
						actions={
							<LinkButton
								route={{
									screen: "access",
									tab: "people",
									action: "add-people",
								}}
								scope={ACCOUNT_SCOPE}
								size="sm"
								act="add-people"
							>
								{t("app.access.addPeople", "Add people…")}
							</LinkButton>
						}
					/>
				</div>
			) : (
				<AccessNotes read={read} />
			)}
			{devices.rest.length ? (
				<p
					data-more="devices"
					className="flex flex-wrap items-center gap-x-3 gap-y-1 border-t border-hairline px-4 py-2 text-xs text-muted-foreground"
				>
					<span>
						{t(
							"app.access.more",
							"Access on {{count, number}} more devices isn't shown: {{names}}.",
							{
								count: devices.rest.length,
								names: nameList(
									devices.rest.map((group) => group.name),
									NAME_CAP,
								),
							},
						)}
					</span>
					<ShowMore count={devices.rest.length} onClick={devices.showAll} />
				</p>
			) : null}
		</Block>
	);
}
