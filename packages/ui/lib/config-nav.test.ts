import { describe, expect, it } from "vitest";
import { IAppVisibility } from "../types";
import {
	type INavigationItem,
	buildNavigationItems,
	isConfigRouteActive,
	resolveNavigationItems,
} from "./config-nav";
import { RolePermissions } from "./permission/role-permission";

const t = (_key: string, defaultValue: string) => defaultValue;

const ALLOW_ALL = () => true;
const DENY_ALL = () => false;
const REASON = (item: INavigationItem) => `locked:${item.label}`;

function resolve(
	overrides: Partial<Parameters<typeof resolveNavigationItems>[1]> = {},
) {
	return resolveNavigationItems(buildNavigationItems(t), {
		visibility: IAppVisibility.Prototype,
		developerMode: true,
		isPaid: true,
		can: ALLOW_ALL,
		permissionLockReason: REASON,
		...overrides,
	});
}

function find(items: ReturnType<typeof resolve>, href: string) {
	const item = items.find((entry) => entry.href === href);
	if (!item) throw new Error(`no nav item for ${href}`);
	return item;
}

describe("resolveNavigationItems", () => {
	it("leaves every section unlocked for a role that holds everything", () => {
		expect(resolve().filter((item) => item.lock)).toEqual([]);
	});

	it("locks a section the role cannot read, naming the permissions that open it", () => {
		const items = resolve({
			can: (...permissions: RolePermissions[]) =>
				!permissions.some((p) => p.equals(RolePermissions.ReadTeam)),
		});

		const team = find(items, "/library/config/team");
		expect(team.lock).toEqual({
			kind: "permission",
			reason: "locked:Team",
			missing: [RolePermissions.ReadTeam],
		});
		expect(find(items, "/library/config/flows").lock).toBeUndefined();
	});

	it("prefers the visibility lock over the permission lock", () => {
		// On a private app Team is not a role problem, and telling the owner to
		// ask an admin would be advice to nobody.
		const items = resolve({
			visibility: IAppVisibility.Private,
			can: DENY_ALL,
		});

		expect(find(items, "/library/config/team").lock).toEqual({
			kind: "visibility",
			reason: expect.stringContaining("A private project"),
			target: IAppVisibility.Prototype,
		});
	});

	it("keeps locked sections in the list rather than hiding them", () => {
		const denied = resolve({ can: DENY_ALL });
		expect(denied.length).toBe(resolve().length);
		expect(denied.filter((item) => item.lock).length).toBeGreaterThan(5);
	});

	it("keeps sales history visible at a zero price while hiding developer tools", () => {
		const hidden = resolve({ developerMode: false, isPaid: false });
		expect(
			hidden.find((item) => item.href === "/library/config/sales"),
		).toBeDefined();
		expect(
			hidden.find((item) => item.href === "/library/config/flows"),
		).toBeUndefined();
		expect(
			hidden.find((item) => item.href === "/library/config/setup"),
		).toBeDefined();
	});

	it("folds payment settings into Monetization, which a local-only app lacks", () => {
		const items = resolve();
		expect(
			items.find((item) => item.href === "/library/config/payments"),
		).toBeUndefined();
		expect(find(items, "/library/config/sales").label).toBe("Monetization");
		expect(
			resolve({ visibility: IAppVisibility.Offline }).find(
				(item) => item.href === "/library/config/sales",
			),
		).toBeUndefined();
	});

	it("keeps each group contiguous so the sidebar prints every heading once", () => {
		const groups = buildNavigationItems(t).map((item) => item.group);
		const runs = groups.filter((group, index) => group !== groups[index - 1]);
		expect(runs).toEqual([...new Set(groups)]);
	});

	it("locks nothing while the role is unknown", () => {
		// `can` degrades open for an offline or local-only app, which has no
		// permission model at all; locking it would strand its owner.
		expect(resolve({ can: ALLOW_ALL }).filter((item) => item.lock)).toEqual([]);
	});

	it("treats multiple permissions on a section as any-of", () => {
		const readDatabaseOnly = (...permissions: RolePermissions[]) =>
			permissions.some((p) => p.equals(RolePermissions.ReadDatabase));

		expect(
			find(resolve({ can: readDatabaseOnly }), "/library/config/explore").lock,
		).toBeUndefined();
	});
});

describe("host capabilities", () => {
	const OFFLINE = "/library/config/offline";
	const DESKTOP = new Set(["offlineWrites"]);

	it("leaves Offline access out on a host without offline writes", () => {
		expect(resolve().find((item) => item.href === OFFLINE)).toBeUndefined();
		expect(
			resolve({ hostCapabilities: new Set(["other"]) }).find(
				(item) => item.href === OFFLINE,
			),
		).toBeUndefined();
	});

	it("lists Offline access in the Data group on a host with offline writes", () => {
		const items = resolve({ hostCapabilities: DESKTOP });
		const offline = find(items, OFFLINE);
		expect(offline.lock).toBeUndefined();
		expect(offline.group).toBe("Data");
		const hrefs = items.map((item) => item.href);
		expect(hrefs.indexOf(OFFLINE)).toBe(
			hrefs.indexOf("/library/config/explore") + 1,
		);
	});

	it("is not a developer tool", () => {
		expect(
			resolve({ hostCapabilities: DESKTOP, developerMode: false }).find(
				(item) => item.href === OFFLINE,
			),
		).toBeDefined();
	});

	it("is hidden, not locked, for a local-only project", () => {
		expect(
			resolve({
				hostCapabilities: DESKTOP,
				visibility: IAppVisibility.Offline,
			}).find((item) => item.href === OFFLINE),
		).toBeUndefined();
	});

	it("locks behind ExecuteEvents", () => {
		const withoutExecute = (...permissions: RolePermissions[]) =>
			!permissions.some((p) => p.equals(RolePermissions.ExecuteEvents));
		expect(
			find(resolve({ hostCapabilities: DESKTOP, can: withoutExecute }), OFFLINE)
				.lock,
		).toEqual({
			kind: "permission",
			reason: "locked:Offline access",
			missing: [RolePermissions.ExecuteEvents],
		});
		const executeOnly = (...permissions: RolePermissions[]) =>
			permissions.some((p) => p.equals(RolePermissions.ExecuteEvents));
		expect(
			find(resolve({ hostCapabilities: DESKTOP, can: executeOnly }), OFFLINE)
				.lock,
		).toBeUndefined();
	});
});

describe("isConfigRouteActive", () => {
	it("matches the dashboard only on an exact route", () => {
		expect(isConfigRouteActive("/library/config", "/library/config")).toBe(
			true,
		);
		expect(isConfigRouteActive("/library/config", "/library/config/team")).toBe(
			false,
		);
	});

	it("matches a section on its own subtree", () => {
		expect(
			isConfigRouteActive("/library/config/team", "/library/config/team"),
		).toBe(true);
		expect(
			isConfigRouteActive("/library/config/team", "/library/config/roles"),
		).toBe(false);
	});
});
