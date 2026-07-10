/**
 * Represents a set of role permission flags.
 * Must stay in sync with packages/api/src/permission/role_permission.rs
 */
export class RolePermissions {
	private readonly value: bigint;

	static readonly Owner = new RolePermissions(
		0b00000000_00000000_00000000_00000001n,
	);
	static readonly Admin = new RolePermissions(
		0b00000000_00000000_00000000_00000010n,
	);
	static readonly ReadTeam = new RolePermissions(
		0b00000000_00000000_00000000_00000100n,
	);
	static readonly ReadRoles = new RolePermissions(
		0b00000000_00000000_00000000_00001000n,
	);
	static readonly ReadFiles = new RolePermissions(
		0b00000000_00000000_00000000_00010000n,
	);
	static readonly WriteFiles = new RolePermissions(
		0b00000000_00000000_00000000_00100000n,
	);
	static readonly InvokeApi = new RolePermissions(
		0b00000000_00000000_00000000_01000000n,
	);
	static readonly WriteMeta = new RolePermissions(
		0b00000000_00000000_00000000_10000000n,
	);
	static readonly ReadBoards = new RolePermissions(
		0b00000000_00000000_00000001_00000000n,
	);
	static readonly ExecuteBoards = new RolePermissions(
		0b00000000_00000000_00000010_00000000n,
	);
	static readonly WriteBoards = new RolePermissions(
		0b00000000_00000000_00000100_00000000n,
	);
	static readonly ListEvents = new RolePermissions(
		0b00000000_00000000_00001000_00000000n,
	);
	static readonly ReadEvents = new RolePermissions(
		0b00000000_00000000_00010000_00000000n,
	);
	static readonly ExecuteEvents = new RolePermissions(
		0b00000000_00000000_00100000_00000000n,
	);
	static readonly WriteEvents = new RolePermissions(
		0b00000000_00000000_01000000_00000000n,
	);
	static readonly ReadLogs = new RolePermissions(
		0b00000000_00000000_10000000_00000000n,
	);
	static readonly ReadAnalytics = new RolePermissions(
		0b00000000_00000001_00000000_00000000n,
	);
	static readonly ReadConfig = new RolePermissions(
		0b00000000_00000010_00000000_00000000n,
	);
	static readonly WriteConfig = new RolePermissions(
		0b00000000_00000100_00000000_00000000n,
	);
	static readonly ReadTemplates = new RolePermissions(
		0b00000000_00001000_00000000_00000000n,
	);
	static readonly WriteTemplates = new RolePermissions(
		0b00000000_00010000_00000000_00000000n,
	);
	static readonly ReadCourses = new RolePermissions(
		0b00000000_00100000_00000000_00000000n,
	);
	static readonly WriteCourses = new RolePermissions(
		0b00000000_01000000_00000000_00000000n,
	);
	static readonly ReadWidgets = new RolePermissions(
		0b00000000_10000000_00000000_00000000n,
	);
	static readonly WriteWidgets = new RolePermissions(
		0b00000001_00000000_00000000_00000000n,
	);
	static readonly WriteRoutes = new RolePermissions(
		0b00000010_00000000_00000000_00000000n,
	);
	static readonly ReadDatabase = new RolePermissions(
		0b00000100_00000000_00000000_00000000n,
	);
	static readonly WriteDatabase = new RolePermissions(
		0b00001000_00000000_00000000_00000000n,
	);

	/**
	 * Permission set a member's (default) role must grant for the
	 * Fork-an-app feature to actually work. Forking copies boards, events,
	 * files, templates, widgets and roles, so the caller must be able to
	 * read each of those resource types.
	 *
	 * Must stay in sync with `FORK_REQUIRED_PERMISSIONS` in
	 * packages/api/src/permission/fork_permission.rs.
	 */
	static readonly ForkRequired = new RolePermissions(
		RolePermissions.ReadBoards.value |
			RolePermissions.ReadEvents.value |
			RolePermissions.ReadFiles.value |
			RolePermissions.ReadTemplates.value |
			RolePermissions.ReadWidgets.value |
			RolePermissions.ReadRoles.value,
	);

	/** @deprecated Use ListEvents instead */
	static readonly ListReleases = RolePermissions.ListEvents;
	/** @deprecated Use ReadEvents instead */
	static readonly ReadReleases = RolePermissions.ReadEvents;
	/** @deprecated Use ExecuteEvents instead */
	static readonly ExecuteReleases = RolePermissions.ExecuteEvents;
	/** @deprecated Use WriteEvents instead */
	static readonly WriteReleases = RolePermissions.WriteEvents;

	constructor(value: bigint | number = 0n) {
		this.value = typeof value === "number" ? BigInt(value) : value;
	}

	hasPermission(permission: RolePermissions): boolean {
		return (
			this.contains(permission) ||
			this.contains(RolePermissions.Admin) ||
			this.contains(RolePermissions.Owner)
		);
	}

	/**
	 * Check if this permission set contains the specified permission(s)
	 */
	contains(permission: RolePermissions): boolean {
		return (this.value & permission.value) === permission.value;
	}

	/**
	 * Create a new permission set with the specified permission(s) added
	 */
	insert(permission: RolePermissions): RolePermissions {
		return new RolePermissions(this.value | permission.value);
	}

	/**
	 * Create a new permission set with the specified permission(s) removed
	 */
	remove(permission: RolePermissions): RolePermissions {
		return new RolePermissions(this.value & ~permission.value);
	}

	/**
	 * Create a new permission set with the specified permission(s) toggled
	 */
	toggle(permission: RolePermissions): RolePermissions {
		return new RolePermissions(this.value ^ permission.value);
	}

	/**
	 * Check if this permission set is empty
	 */
	isEmpty(): boolean {
		return this.value === 0n;
	}

	/**
	 * Create a new permission set from multiple permissions
	 */
	static from(...permissions: RolePermissions[]): RolePermissions {
		return permissions.reduce(
			(acc, perm) => acc.insert(perm),
			new RolePermissions(),
		);
	}

	/**
	 * Get the raw numeric value
	 */
	toBigInt(): bigint {
		return this.value;
	}

	/**
	 * Convert to number (use with caution for large values)
	 */
	toNumber(): number {
		return Number(this.value);
	}

	/**
	 * Check equality with another permission set
	 */
	equals(other: RolePermissions): boolean {
		return this.value === other.value;
	}

	/**
	 * String representation for debugging
	 */
	toString(): string {
		return `RolePermissions(${this.value.toString(2).padStart(24, "0")})`;
	}
}
