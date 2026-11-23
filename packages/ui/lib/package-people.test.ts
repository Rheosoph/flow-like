import { describe, expect, test } from "bun:test";
import {
	assignableRoles,
	canWithdrawInvitation,
	packageRole,
	personActions,
	roleAfterTransfer,
} from "./package-people";
import { PackagePermissionBits as P } from "./permission/wasm-package-permission";

describe("package people actions mirror the API", () => {
	test("an owner manages maintainers and users, transfers, and cannot touch buyers' roles", () => {
		expect(personActions(P.Owner, P.Maintainer, false)).toEqual({
			roleOptions: ["maintainer", "user"],
			canTransfer: true,
			canRemove: true,
			canLeave: false,
		});
		expect(personActions(P.Owner, P.Buyer, false)).toEqual({
			roleOptions: [],
			canTransfer: true,
			canRemove: false,
			canLeave: false,
		});
	});

	test("a maintainer can only remove users and never sees a role switch", () => {
		expect(personActions(P.Maintainer, P.User, false)).toEqual({
			roleOptions: [],
			canTransfer: false,
			canRemove: true,
			canLeave: false,
		});
		expect(personActions(P.Maintainer, P.Maintainer, false).canRemove).toBe(
			false,
		);
		expect(personActions(P.Maintainer, P.Owner, false).canRemove).toBe(false);
	});

	test("nobody edits the owner row, the owner included", () => {
		expect(personActions(P.Owner, P.Owner, true)).toEqual({
			roleOptions: [],
			canTransfer: false,
			canRemove: false,
			canLeave: false,
		});
		expect(personActions(P.Owner, P.Owner, false).roleOptions).toEqual([]);
	});

	test("maintainers and users may leave; buyers keep their purchase", () => {
		expect(personActions(P.Maintainer, P.Maintainer, true).canLeave).toBe(true);
		expect(personActions(P.User, P.User, true).canLeave).toBe(true);
		expect(personActions(P.Buyer, P.Buyer, true).canLeave).toBe(false);
	});

	test("invitation roles and withdrawal follow the caller's level", () => {
		expect(assignableRoles(P.Owner)).toEqual(["maintainer", "user"]);
		expect(assignableRoles(P.Maintainer)).toEqual(["user"]);
		expect(assignableRoles(P.User)).toEqual([]);
		expect(canWithdrawInvitation(P.Maintainer, P.Maintainer)).toBe(false);
		expect(canWithdrawInvitation(P.Maintainer, P.User)).toBe(true);
		expect(canWithdrawInvitation(P.Owner, P.Maintainer)).toBe(true);
	});

	test("buyers read as buyers and the previous owner takes the new owner's role", () => {
		expect(packageRole(P.Buyer)).toBe("buyer");
		expect(packageRole(P.User | P.Buyer)).toBe("user");
		expect(roleAfterTransfer(P.User)).toBe("user");
		expect(roleAfterTransfer(P.Buyer)).toBe("maintainer");
		expect(roleAfterTransfer(P.Maintainer)).toBe("maintainer");
	});
});
