/** Bitflag permission helpers for WasmPackageUser — mirrors Rust `WasmPackagePermission` */

export const PackagePermissionBits = {
	Owner: 0b01,
	Maintainer: 0b10,
	User: 0b100,
	Buyer: 0b1000,
} as const;

export type PackagePermissionBitKey = keyof typeof PackagePermissionBits;

export function hasPackagePermission(
	permission: number,
	required: number,
): boolean {
	if (permission & PackagePermissionBits.Owner) return true;
	if (
		required === PackagePermissionBits.User &&
		permission & PackagePermissionBits.Maintainer
	)
		return true;
	return (permission & required) !== 0;
}

export function isOwner(permission: number): boolean {
	return (permission & PackagePermissionBits.Owner) !== 0;
}

export function isMaintainer(permission: number): boolean {
	return (
		(permission & PackagePermissionBits.Maintainer) !== 0 || isOwner(permission)
	);
}

export function isUser(permission: number): boolean {
	return (
		(permission & PackagePermissionBits.User) !== 0 || isMaintainer(permission)
	);
}

export function isBuyer(permission: number): boolean {
	return (permission & PackagePermissionBits.Buyer) !== 0;
}

export function permissionLabel(permission: number): string {
	if (isOwner(permission)) return "Owner";
	if (isMaintainer(permission)) return "Maintainer";
	if (isBuyer(permission)) return "Buyer";
	if (isUser(permission)) return "User";
	return "None";
}

/** Whether the caller can manage (change/remove) the target permission level.
 * Returns false for Buyer targets (protected) and Owner targets. */
export function canManageLevel(
	callerPerm: number,
	targetPerm: number,
): boolean {
	if (targetPerm & PackagePermissionBits.Buyer) return false;
	if (targetPerm & PackagePermissionBits.Owner) return false;
	if (callerPerm & PackagePermissionBits.Owner) return true;
	return (
		(callerPerm & PackagePermissionBits.Maintainer) !== 0 &&
		targetPerm === PackagePermissionBits.User
	);
}

export function permissionFromLevel(
	level: "owner" | "maintainer" | "user",
): number {
	switch (level) {
		case "owner":
			return PackagePermissionBits.Owner;
		case "maintainer":
			return PackagePermissionBits.Maintainer;
		case "user":
			return PackagePermissionBits.User;
	}
}
