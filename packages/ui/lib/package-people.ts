/**
 * What a package's Access tab may offer for each person. Mirrors the API:
 * `WasmPackagePermission::can_manage_level`, `assignable_role` and the transfer
 * and self-removal rules in `packages/api/src/routes/registry/users.rs`. Offering
 * anything the API refuses only produces a 403 toast.
 */

import { PackagePermissionBits } from "./permission/wasm-package-permission";

export type PackageRole = "owner" | "maintainer" | "user" | "buyer";
export type AssignableRole = "maintainer" | "user";

export function packageRole(permission: number): PackageRole {
	if (permission & PackagePermissionBits.Owner) return "owner";
	if (permission & PackagePermissionBits.Maintainer) return "maintainer";
	if (permission & PackagePermissionBits.User) return "user";
	if (permission & PackagePermissionBits.Buyer) return "buyer";
	return "user";
}

export function roleBits(role: AssignableRole): number {
	return role === "maintainer"
		? PackagePermissionBits.Maintainer
		: PackagePermissionBits.User;
}

/** `can_manage_level`: Buyers are never managed; an Owner manages the rest; a Maintainer manages exactly User. */
export function canManageLevel(caller: number, target: number): boolean {
	if (target & PackagePermissionBits.Buyer) return false;
	if (caller & PackagePermissionBits.Owner) return true;
	return (
		(caller & PackagePermissionBits.Maintainer) !== 0 &&
		target === PackagePermissionBits.User
	);
}

/** Roles the caller may hand out in an invitation or a role change. */
export function assignableRoles(caller: number): AssignableRole[] {
	return (["maintainer", "user"] as const).filter((role) =>
		canManageLevel(caller, roleBits(role)),
	);
}

export interface PersonActions {
	/** Roles the row may switch to; empty when the role is fixed for this caller. */
	roleOptions: AssignableRole[];
	canTransfer: boolean;
	canRemove: boolean;
	canLeave: boolean;
}

const NONE: PersonActions = {
	roleOptions: [],
	canTransfer: false,
	canRemove: false,
	canLeave: false,
};

export function personActions(
	caller: number,
	target: number,
	isSelf: boolean,
): PersonActions {
	const owner = (target & PackagePermissionBits.Owner) !== 0;
	const buyer = (target & PackagePermissionBits.Buyer) !== 0;
	if (isSelf) return { ...NONE, canLeave: !owner && !buyer };
	if (owner) return NONE;

	const callerIsOwner = (caller & PackagePermissionBits.Owner) !== 0;
	const manageable = canManageLevel(caller, target);
	const roleOptions = manageable ? assignableRoles(caller) : [];
	return {
		roleOptions: roleOptions.length > 1 ? roleOptions : [],
		canTransfer: callerIsOwner,
		canRemove: manageable,
		canLeave: false,
	};
}

/** `transfer_in_transaction`: the previous Owner takes the new Owner's old role, Maintainer when that was a purchase. */
export function roleAfterTransfer(targetPermission: number): PackageRole {
	if (targetPermission & PackagePermissionBits.Buyer) return "maintainer";
	return packageRole(targetPermission);
}

export function canWithdrawInvitation(
	caller: number,
	invitationPermission: number,
): boolean {
	return canManageLevel(caller, invitationPermission);
}
