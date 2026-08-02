import {
	ACCESS_LADDERS,
	ALL_PERMISSIONS,
	ROLE_TEMPLATES,
	RolePermissions,
	TOTAL_PERMISSION_COUNT,
	applyElevation,
	applyLevel,
	describeAccess,
	effectiveLevel,
	effectivePermissionCount,
	elevationOf,
	levelOf,
	permissionsFromTemplate,
	writePermissionCount,
} from "@flow-like/flow-like-ui";
import { describe, expect, it } from "vitest";

const maskOf = (perms: RolePermissions[]) =>
	perms.reduce((acc, perm) => acc | perm.toBigInt(), 0n);

const topOf = (ladder: (typeof ACCESS_LADDERS)[number]) =>
	ladder.levels[ladder.levels.length - 1];

describe("ladder structure", () => {
	it("covers every permission exactly once, plus Owner and Admin", () => {
		const covered = ACCESS_LADDERS.flatMap(
			(ladder) => topOf(ladder).permissions,
		);
		const seen = new Set<string>();
		for (const perm of covered) {
			const key = perm.toBigInt().toString();
			expect(seen.has(key), `${key} appears in two ladders`).toBe(false);
			seen.add(key);
		}
		const reachable = new Set([
			...seen,
			RolePermissions.Owner.toBigInt().toString(),
			RolePermissions.Admin.toBigInt().toString(),
		]);
		for (const perm of ALL_PERMISSIONS) {
			expect(
				reachable.has(perm.toBigInt().toString()),
				`${perm.toString()} is unreachable from the ladders`,
			).toBe(true);
		}
		expect(reachable.size).toBe(ALL_PERMISSIONS.length);
		expect(TOTAL_PERMISSION_COUNT).toBe(ALL_PERMISSIONS.length);
	});

	it("escalates monotonically — each level is a superset of the one below", () => {
		for (const ladder of ACCESS_LADDERS) {
			expect(ladder.levels[0].permissions).toHaveLength(0);
			for (let i = 1; i < ladder.levels.length; i++) {
				const lower = maskOf(ladder.levels[i - 1].permissions);
				const upper = maskOf(ladder.levels[i].permissions);
				expect(
					(upper & lower) === lower && upper !== lower,
					`${ladder.label}: "${ladder.levels[i].name}" does not extend "${ladder.levels[i - 1].name}"`,
				).toBe(true);
			}
		}
	});
});

describe("levelOf", () => {
	it("round-trips every level of every ladder", () => {
		for (const ladder of ACCESS_LADDERS) {
			ladder.levels.forEach((_, index) => {
				const perms = applyLevel(new RolePermissions(), ladder, index);
				expect(levelOf(perms, ladder)).toBe(index);
			});
		}
	});

	it("reports -1 for a set that matches no level", () => {
		const ladder = ACCESS_LADDERS.find((entry) => entry.id === "boards");
		if (!ladder) throw new Error("workflows ladder missing");
		// Edit without View — reachable only through the advanced switches.
		const perms = new RolePermissions().insert(RolePermissions.WriteBoards);
		expect(levelOf(perms, ladder)).toBe(-1);
	});

	it("leaves other ladders untouched when one level changes", () => {
		const boards = ACCESS_LADDERS.find((entry) => entry.id === "boards");
		const content = ACCESS_LADDERS.find((entry) => entry.id === "content");
		if (!boards || !content) throw new Error("ladder missing");

		let perms = applyLevel(new RolePermissions(), content, 2);
		perms = applyLevel(perms, boards, 3);
		perms = applyLevel(perms, boards, 0);

		expect(levelOf(perms, content)).toBe(2);
		expect(levelOf(perms, boards)).toBe(0);
	});
});

describe("elevation", () => {
	it("puts Owner and Admin at the top of every ladder", () => {
		for (const elevation of ["admin", "owner"] as const) {
			const perms = applyElevation(new RolePermissions(), elevation);
			expect(elevationOf(perms)).toBe(elevation);
			for (const ladder of ACCESS_LADDERS) {
				expect(effectiveLevel(perms, ladder)).toBe(ladder.levels.length - 1);
			}
		}
	});

	it("counts what elevation implies, not what is stored", () => {
		const owner = applyElevation(new RolePermissions(), "owner");
		const admin = applyElevation(new RolePermissions(), "admin");
		expect(effectivePermissionCount(owner)).toBe(TOTAL_PERMISSION_COUNT);
		expect(effectivePermissionCount(admin)).toBe(TOTAL_PERMISSION_COUNT - 1);
		expect(writePermissionCount(admin)).toBeGreaterThan(0);
	});

	it("drops back to the stored levels when set to standard", () => {
		const boards = ACCESS_LADDERS.find((entry) => entry.id === "boards");
		if (!boards) throw new Error("workflows ladder missing");
		const withLevel = applyLevel(new RolePermissions(), boards, 2);
		const elevated = applyElevation(withLevel, "admin");
		const back = applyElevation(elevated, "standard");
		expect(elevationOf(back)).toBe("standard");
		expect(levelOf(back, boards)).toBe(2);
	});
});

describe("templates", () => {
	it("produces permissions that map back to whole levels", () => {
		for (const template of ROLE_TEMPLATES) {
			const perms = permissionsFromTemplate(template);
			if (template.elevation) {
				expect(elevationOf(perms)).toBe(template.elevation);
				continue;
			}
			for (const ladder of ACCESS_LADDERS) {
				expect(levelOf(perms, ladder)).toBe(template.levels?.[ladder.id] ?? 0);
			}
		}
	});

	it("keeps Viewer free of write access and Editor capable of it", () => {
		const viewer = ROLE_TEMPLATES.find((entry) => entry.name === "Viewer");
		const editor = ROLE_TEMPLATES.find((entry) => entry.name === "Editor");
		if (!viewer || !editor) throw new Error("template missing");
		expect(writePermissionCount(permissionsFromTemplate(viewer))).toBe(0);
		expect(
			writePermissionCount(permissionsFromTemplate(editor)),
		).toBeGreaterThan(0);
	});
});

describe("describeAccess", () => {
	it("says only what a role cannot do when it has nothing", () => {
		const { can, cannot } = describeAccess(new RolePermissions());
		expect(can).toHaveLength(0);
		expect(cannot).toHaveLength(ACCESS_LADDERS.length);
	});

	it("names the custom mix instead of guessing a level", () => {
		const perms = new RolePermissions().insert(RolePermissions.WriteBoards);
		const { can } = describeAccess(perms);
		expect(can).toContain("a custom mix in workflows");
	});

	it("describes the Operator template in plain language", () => {
		const operator = ROLE_TEMPLATES.find((entry) => entry.name === "Operator");
		if (!operator) throw new Error("template missing");
		const { can, cannot } = describeAccess(permissionsFromTemplate(operator));
		expect(can).toContain("open and run workflows");
		expect(can).toContain("trigger events");
		expect(cannot).toHaveLength(0);
	});
});
