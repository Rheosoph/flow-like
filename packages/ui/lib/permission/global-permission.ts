/**
 * Represents a set of global permission flags.
 */
export class GlobalPermission {
	private readonly value: bigint;

	// Permission constants
	static readonly Admin = new GlobalPermission(0b00000000_00000001n);
	static readonly ReadPublishing = new GlobalPermission(0b00000000_00000010n);
	static readonly WritePublishing = new GlobalPermission(0b00000000_00000100n);
	static readonly ReadProfile = new GlobalPermission(0b00000000_00001000n);
	static readonly WriteProfile = new GlobalPermission(0b00000000_00010000n);
	static readonly ReadApps = new GlobalPermission(0b00000000_00100000n);
	static readonly WriteApps = new GlobalPermission(0b00000000_01000000n);
	static readonly WriteLandingPage = new GlobalPermission(0b00000000_10000000n);
	static readonly ReadTransactions = new GlobalPermission(0b00000001_00000000n);
	static readonly WriteTransactions = new GlobalPermission(
		0b00000010_00000000n,
	);
	static readonly WriteBits = new GlobalPermission(0b00000100_00000000n);
	static readonly ReadSolutions = new GlobalPermission(0b00001000_00000000n);
	static readonly WriteSolutions = new GlobalPermission(0b00010000_00000000n);
	static readonly ManagePackages = new GlobalPermission(0b00100000_00000000n);
	static readonly ReadCourses = new GlobalPermission(0b01000000_00000000n);
	static readonly WriteCourses = new GlobalPermission(0b10000000_00000000n);

	constructor(value: bigint | number = 0n) {
		this.value = typeof value === "number" ? BigInt(value) : value;
	}

	hasPermission(permission: GlobalPermission): boolean {
		return this.contains(permission) || this.contains(GlobalPermission.Admin);
	}

	/**
	 * Check if this permission set contains the specified permission(s)
	 */
	contains(permission: GlobalPermission): boolean {
		return (this.value & permission.value) === permission.value;
	}

	/**
	 * Create a new permission set with the specified permission(s) added
	 */
	insert(permission: GlobalPermission): GlobalPermission {
		return new GlobalPermission(this.value | permission.value);
	}

	/**
	 * Create a new permission set with the specified permission(s) removed
	 */
	remove(permission: GlobalPermission): GlobalPermission {
		return new GlobalPermission(this.value & ~permission.value);
	}

	/**
	 * Create a new permission set with the specified permission(s) toggled
	 */
	toggle(permission: GlobalPermission): GlobalPermission {
		return new GlobalPermission(this.value ^ permission.value);
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
	static from(...permissions: GlobalPermission[]): GlobalPermission {
		return permissions.reduce(
			(acc, perm) => acc.insert(perm),
			new GlobalPermission(),
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
	equals(other: GlobalPermission): boolean {
		return this.value === other.value;
	}

	/**
	 * String representation for debugging
	 */
	toString(): string {
		return `GlobalPermission(${this.value.toString(2).padStart(16, "0")})`;
	}
}
