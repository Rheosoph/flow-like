"use client";

import { AlertTriangleIcon, CheckIcon, WrenchIcon } from "lucide-react";
import { useCallback, useMemo, useState } from "react";
import { toast } from "sonner";
import { useInvoke } from "../../../hooks";
import { RolePermissions } from "../../../lib/permission/role-permission";
import { useBackend } from "../../../state/backend-state";
import type { IBackendRole } from "../../../state/backend-state/types";
import { Alert, AlertDescription, AlertTitle } from "../../ui/alert";
import { Button } from "../../ui/button";

/** Required fork permissions paired with user-facing labels. Order mirrors
 * `FORK_REQUIRED_PERMISSIONS` in packages/api/src/permission/fork_permission.rs. */
const FORK_REQUIRED_PERMISSION_LABELS: ReadonlyArray<{
	permission: RolePermissions;
	label: string;
}> = [
	{ permission: RolePermissions.ReadBoards, label: "Read Boards" },
	{ permission: RolePermissions.ReadEvents, label: "Read Events" },
	{ permission: RolePermissions.ReadFiles, label: "Read Files" },
	{ permission: RolePermissions.ReadTemplates, label: "Read Templates" },
	{ permission: RolePermissions.ReadWidgets, label: "Read Widgets" },
	{ permission: RolePermissions.ReadRoles, label: "Read Roles" },
];

export interface ForkPermissionWarningProps {
	appId: string;
	/** Whether the Fork-an-app opt-in is currently enabled. */
	enabled: boolean;
	/** Owner-only: gates the one-click fix. */
	canEdit: boolean;
}

/**
 * Surfaces the common pitfall where forking is enabled but the app's
 * default (member) role lacks the read permissions a fork actually needs.
 * When that happens the Fork button stays hidden for members even though
 * the owner opted in. Renders a warning listing the missing permissions
 * plus a one-click fix that grants them to the default role through the
 * existing roles API.
 */
export function ForkPermissionWarning({
	appId,
	enabled,
	canEdit,
}: Readonly<ForkPermissionWarningProps>) {
	const backend = useBackend();
	const roles = useInvoke(
		backend.roleState.getRoles,
		backend.roleState,
		[appId],
		enabled && typeof appId === "string",
	);
	const [fixing, setFixing] = useState(false);

	const { defaultRole, missing } = useMemo(() => {
		if (!roles.data) {
			return { defaultRole: undefined, missing: [] as string[] };
		}
		const defaultRoleId = roles.data[0];
		const allRoles = roles.data[1];
		const role = allRoles.find((r) => r.id === defaultRoleId);
		if (!role) return { defaultRole: undefined, missing: [] as string[] };
		const perms = new RolePermissions(role.permissions);
		const missingLabels = FORK_REQUIRED_PERMISSION_LABELS.filter(
			({ permission }) => !perms.contains(permission),
		).map(({ label }) => label);
		return { defaultRole: role, missing: missingLabels };
	}, [roles.data]);

	const handleFix = useCallback(async () => {
		if (!defaultRole || fixing) return;
		setFixing(true);
		try {
			const updated = new RolePermissions(defaultRole.permissions).insert(
				RolePermissions.ForkRequired,
			);
			const next: IBackendRole = {
				...defaultRole,
				permissions: updated.toBigInt(),
			};
			await backend.roleState.upsertRole(appId, next);
			await roles.refetch();
			toast.success("Default role updated — members can now fork this app.");
		} catch (err) {
			toast.error(
				err instanceof Error
					? `Couldn't update the default role: ${err.message}`
					: "Couldn't update the default role.",
			);
		} finally {
			setFixing(false);
		}
	}, [appId, backend.roleState, defaultRole, fixing, roles]);

	if (!enabled || !defaultRole || missing.length === 0) return null;

	return (
		<Alert variant="destructive" className="mt-4">
			<AlertTriangleIcon className="w-4 h-4" />
			<AlertTitle>Forking won't work for members yet</AlertTitle>
			<AlertDescription>
				<p>
					Forking is enabled, but the default role{" "}
					<span className="font-medium">{defaultRole.name}</span> is missing
					read permissions a fork needs. Until these are granted, the Fork
					button stays hidden for members.
				</p>
				<ul className="list-disc pl-5">
					{missing.map((label) => (
						<li key={label}>{label}</li>
					))}
				</ul>
				{canEdit ? (
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
						{fixing ? "Applying…" : "Grant required permissions"}
					</Button>
				) : (
					<p className="text-xs">
						Ask the app owner to grant these permissions to the default role.
					</p>
				)}
			</AlertDescription>
		</Alert>
	);
}
