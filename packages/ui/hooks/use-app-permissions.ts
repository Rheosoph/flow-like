"use client";

import { useMemo } from "react";
import { RolePermissions } from "../lib/permission/role-permission";
import { useBackend, useBackendReady } from "../state/backend-state";
import type { IOwnRole } from "../state/backend-state/types";
import { useInvoke } from "./use-invoke";

/**
 * What the signed-in account may do on one app, resolved once from
 * `GET /apps/{id}/roles/me` and shared by every gate in the settings surface.
 *
 * `known` is the axis every caller has to reason about. A local-only or
 * signed-out app has no role table to ask, so the answer is "no permission
 * model here", not "no permission" — {@link AppPermissions.can} degrades open
 * in that case, matching the rule the setup screen already documents. Anything
 * destructive must test `known` itself instead of trusting `can`.
 */
export interface AppPermissions {
	/** The role could be resolved; `can` reflects real server-side bits. */
	known: boolean;
	/** The role request is still in flight. Gates should render a skeleton. */
	isLoading: boolean;
	/** The role could not be read — offline, signed out, or the request failed. */
	isUnavailable: boolean;
	/** Display name of the caller's role, when known. */
	roleName?: string;
	/** Passes an `Owner` check. `Admin` satisfies it, exactly as the server does. */
	isOwner: boolean;
	/** The caller may quit the app. False for owners and for principals with no membership. */
	canLeave: boolean;
	/**
	 * Whether the caller holds a permission. Uses `hasPermission`, so `Owner`
	 * and `Admin` satisfy every flag — the same escalation `ensure_permission!`
	 * applies server-side. Returns true while the role is unknown.
	 */
	can: (...permissions: RolePermissions[]) => boolean;
	/**
	 * Like {@link AppPermissions.can} but fails closed while the role is
	 * unknown. Use it for destructive or irreversible affordances.
	 */
	canStrict: (...permissions: RolePermissions[]) => boolean;
	/** Raw bits, for callers that need to diff or display a whole set. */
	permissions: RolePermissions;
	raw?: IOwnRole;
}

const NO_PERMISSIONS = new RolePermissions(0n);

/**
 * Resolve the caller's permissions on an app.
 *
 * Gated on {@link useBackendReady} because the prerender placeholder backend
 * throws from `getOwnRole`, and a cached error would outlive the real backend
 * landing (the query key is the method name, so it survives the swap).
 */
export function useAppPermissions(
	appId: string | undefined | null,
): AppPermissions {
	const backend = useBackend();
	const backendReady = useBackendReady();
	const enabled = backendReady && typeof appId === "string" && appId.length > 0;

	const ownRole = useInvoke(
		backend.roleState.getOwnRole,
		backend.roleState,
		[appId ?? ""],
		enabled,
	);

	return useMemo(() => {
		// A role without integer bits (older hub, restored cache) is not a known role.
		const data = Number.isInteger(ownRole.data?.permissions)
			? ownRole.data
			: undefined;
		const known = !!data;
		const permissions = data
			? new RolePermissions(BigInt(data.permissions))
			: NO_PERMISSIONS;

		const holds = (list: RolePermissions[]) =>
			list.length === 0 || list.some((p) => permissions.hasPermission(p));

		return {
			known,
			// A disabled query sits at `isPending` forever, so ask `isLoading`:
			// "not enabled" is an answer, not a loading state.
			isLoading: enabled && ownRole.isLoading,
			isUnavailable: !known && !(enabled && ownRole.isLoading),
			roleName: data?.role_name,
			isOwner: data?.is_owner ?? false,
			canLeave: data?.can_leave ?? false,
			can: (...list: RolePermissions[]) => (known ? holds(list) : true),
			canStrict: (...list: RolePermissions[]) => (known ? holds(list) : false),
			permissions,
			raw: data,
		};
	}, [ownRole.data, ownRole.isLoading, enabled]);
}
