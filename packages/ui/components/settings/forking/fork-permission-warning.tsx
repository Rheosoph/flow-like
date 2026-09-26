"use client";

import { useTranslation } from "@flow-like/locales";
import { AlertTriangleIcon, CheckIcon, WrenchIcon } from "lucide-react";
import { useCallback, useMemo, useState } from "react";
import { toast } from "sonner";
import { useInvoke } from "../../../hooks";
import { useAppPermissions } from "../../../hooks/use-app-permissions";
import { RolePermissions } from "../../../lib/permission/role-permission";
import { asArray } from "../../../lib/response-shape";
import type { IForkPolicy } from "../../../lib/schema/app/fork";
import { useBackend } from "../../../state/backend-state";
import type { IBackendRole } from "../../../state/backend-state/types";
import { Alert, AlertDescription, AlertTitle } from "../../ui/alert";
import { Button } from "../../ui/button";

/** Which fork categories make each read permission necessary. A category the
 * owner excludes is never copied, so its permission is not demanded.
 * Mirrors `fork_required_permissions` in
 * packages/api/src/permission/fork_permission.rs. */
const FORK_PERMISSION_REQUIREMENTS: ReadonlyArray<{
	permission: RolePermissions;
	label: string;
	/** Why the fork needs it — shown so the owner can see which setting to
	 * change instead of granting the permission. */
	reason: string;
	requiredBy: (policy: IForkPolicy) => boolean;
}> = [
	{
		permission: RolePermissions.ReadBoards,
		label: "Read Boards",
		reason: "Flows",
		requiredBy: (policy) => policy.flows,
	},
	{
		permission: RolePermissions.ReadEvents,
		label: "Read Events",
		reason: "Flows",
		requiredBy: (policy) => policy.flows,
	},
	{
		// `ReadFiles` implies `ReadDatabase`, and the project-database routes
		// accept either, so both categories map onto this one permission.
		permission: RolePermissions.ReadFiles,
		label: "Read Files",
		reason: "Files and databases",
		requiredBy: (policy) => policy.files || policy.databases !== "none",
	},
	{
		permission: RolePermissions.ReadTemplates,
		label: "Read Templates",
		reason: "Templates",
		requiredBy: (policy) => policy.templates,
	},
	{
		permission: RolePermissions.ReadWidgets,
		label: "Read Widgets",
		reason: "Widgets",
		requiredBy: (policy) => policy.widgets,
	},
	{
		permission: RolePermissions.ReadRoles,
		label: "Read Roles",
		reason: "Roles",
		requiredBy: (policy) => policy.roles,
	},
];

type ForkRequirement = (typeof FORK_PERMISSION_REQUIREMENTS)[number];

/** Permissive fallback matching the server's NULL-policy default, used while
 * the owner's policy is still loading and for viewers who can't read it. */
const PERMISSIVE_FORK_POLICY: IForkPolicy = {
	flows: true,
	files: true,
	databases: "with_data",
	roles: true,
	widgets: true,
	templates: true,
};

function MissingPermissionList({
	missing,
}: Readonly<{ missing: readonly ForkRequirement[] }>) {
	const { t } = useTranslation("settings");
	return (
		<ul className="list-disc pl-5">
			{missing.map(({ label, reason }) => (
				<li key={label}>
					{label}
					<span className="text-xs opacity-80">
						{" "}
						{t("neededForReason", "— needed for {{reason}}", { reason })}
					</span>
				</li>
			))}
		</ul>
	);
}

export interface ForkPermissionWarningProps {
	appId: string;
	/** Whether the Fork-an-app opt-in is currently enabled. */
	enabled: boolean;
	/** Owner-only: gates the one-click fix. */
	canEdit: boolean;
	/** The owner's fork policy. Only the categories it ships need read
	 * permissions. Falls back to the permissive default when unknown. */
	policy?: IForkPolicy;
}

/**
 * Surfaces the common pitfall where forking is enabled but the reader cannot
 * actually fork, so the Fork button silently stays hidden.
 *
 * Owners see it from the app's side: the default (member) role is missing the
 * reads a fork needs, plus a one-click fix that grants them through the roles
 * API. Everyone else sees it from their own side — asking `GET /apps/{id}/roles`
 * would need `ReadRoles`, which is exactly what the affected members lack, so
 * their answer comes from their own resolved role instead.
 */
export function ForkPermissionWarning({
	appId,
	enabled,
	canEdit,
	policy,
}: Readonly<ForkPermissionWarningProps>) {
	const { t } = useTranslation("settings");
	const backend = useBackend();
	const permissions = useAppPermissions(appId);
	const roles = useInvoke(
		backend.roleState.getRoles,
		backend.roleState,
		[appId],
		enabled &&
			typeof appId === "string" &&
			canEdit &&
			permissions.can(RolePermissions.ReadRoles),
	);
	const [fixing, setFixing] = useState(false);

	const effectivePolicy = policy ?? PERMISSIVE_FORK_POLICY;
	const required = useMemo(
		() =>
			FORK_PERMISSION_REQUIREMENTS.filter((r) => r.requiredBy(effectivePolicy)),
		[effectivePolicy],
	);

	const { defaultRole, missing } = useMemo(() => {
		if (!roles.data) {
			return { defaultRole: undefined, missing: [] as ForkRequirement[] };
		}
		const defaultRoleId = roles.data[0];
		const allRoles = asArray(roles.data[1]);
		const role = allRoles.find((r) => r.id === defaultRoleId);
		if (!role) {
			return { defaultRole: undefined, missing: [] as ForkRequirement[] };
		}
		const perms = new RolePermissions(role.permissions);
		return {
			defaultRole: role,
			missing: required.filter(({ permission }) => !perms.contains(permission)),
		};
	}, [roles.data, required]);

	/** What the reader's own role is missing. Empty while the role is unknown. */
	const ownMissing = useMemo(
		() => required.filter(({ permission }) => !permissions.can(permission)),
		[required, permissions],
	);

	const handleFix = useCallback(async () => {
		if (!defaultRole || fixing) return;
		setFixing(true);
		try {
			// Grant only what this app's fork policy actually needs — never the
			// whole historic set, or excluding a category would still widen the
			// default role beyond the app's own settings.
			const updated = required.reduce(
				(perms, { permission }) => perms.insert(permission),
				new RolePermissions(defaultRole.permissions),
			);
			const next: IBackendRole = {
				...defaultRole,
				permissions: updated.toBigInt(),
			};
			await backend.roleState.upsertRole(appId, next);
			await roles.refetch();
			toast.success(
				t(
					"defaultRoleUpdatedMembersCanNowForkThisApp",
					"Default role updated — members can now fork this app.",
				),
			);
		} catch (err) {
			toast.error(
				err instanceof Error
					? t(
							"couldntUpdateTheDefaultRoleMessage",
							"Couldn't update the default role: {{message}}",
							{ message: err.message },
						)
					: t(
							"couldntUpdateTheDefaultRole",
							"Couldn't update the default role.",
						),
			);
		} finally {
			setFixing(false);
		}
	}, [appId, backend.roleState, defaultRole, fixing, required, roles, t]);

	if (!enabled) return null;

	// The reader is not the owner, so `policy` was never fetched and `required`
	// is the permissive superset. Naming a permission this app's fork settings
	// exclude would be a false accusation, so the copy says "can need", not
	// "needs", and points at the setting only the owner can see.
	if (!canEdit) {
		if (ownMissing.length === 0) return null;
		return (
			<Alert className="mt-4">
				<AlertTriangleIcon className="w-4 h-4" />
				<AlertTitle>
					{t(
						"forkingMayNotBeAvailableToYourRole",
						"Forking may not be available to your role",
					)}
				</AlertTitle>
				<AlertDescription>
					<p>
						{t(
							"forkingIsEnabledButYourRoleIsMissingReadPermissionsAForkCanNeedWhichOnesApplyDependsOnThisAppsForkSettingsWhichOnlyTheOwnerCanSee",
							"Forking is enabled, but your role is missing read permissions a fork can need. Which of these actually apply depends on this app's fork settings, which only the owner can see.",
						)}
					</p>
					<MissingPermissionList missing={ownMissing} />
					<p className="text-xs">
						{t(
							"askTheAppOwnerToGrantThesePermissionsToYourRole",
							"Ask the app owner to grant these permissions to your role.",
						)}
					</p>
				</AlertDescription>
			</Alert>
		);
	}

	if (!defaultRole || missing.length === 0) return null;

	return (
		<Alert variant="destructive" className="mt-4">
			<AlertTriangleIcon className="w-4 h-4" />
			<AlertTitle>
				{t(
					"forkingWontWorkForMembersYet",
					"Forking won't work for members yet",
				)}
			</AlertTitle>
			<AlertDescription>
				<p>
					{t(
						"forkingIsEnabledButTheDefaultRole",
						"Forking is enabled, but the default role",
					)}{" "}
					<span className="font-medium">{defaultRole.name}</span>{" "}
					{t(
						"isMissingReadPermissionsAForkNeedsUntilTheseAreGrantedTheForkButtonStaysHiddenForMembers",
						"is missing read permissions a fork needs. Until these are granted, the Fork button stays hidden for members.",
					)}
				</p>
				<MissingPermissionList missing={missing} />
				<p className="text-xs">
					{t(
						"onlyWhatThisAppsForkSettingsIncludeIsRequiredExcludingACategoryAboveRemovesItsPermissionFromThisList",
						"Only what this app's fork settings include is required. Excluding a category above removes its permission from this list.",
					)}
				</p>
				<Button
					type="button"
					size="sm"
					variant="outline"
					className="mt-2"
					disabled={fixing}
					onClick={handleFix}
				>
					{fixing ? (
						<CheckIcon className="w-3.5 h-3.5" />
					) : (
						<WrenchIcon className="w-3.5 h-3.5" />
					)}
					{fixing
						? t("applying", "Applying…")
						: t("grantRequiredPermissions", "Grant required permissions")}
				</Button>
			</AlertDescription>
		</Alert>
	);
}
