import { describe, expect, test } from "bun:test";
import type {
	AttentionItem,
	DeviceRow,
	DevicesRoute,
} from "../../../../lib/device-management/model/types";
import { devicesHref } from "./devices-href";
import { parseDevicesRoute } from "./devices-route";
import { type ResolveFleet, resolveTarget } from "./resolve-target";

const NOW = 1_790_000_000;
const EDGE = "6f1c2a3b-0d4e-4f5a-8b6c-7d8e9f0a1b2c";
const PI = "0a1b2c3d-4e5f-4a6b-8c7d-9e0f1a2b3c4d";
const OLD = "11111111-2222-4333-8444-555555555555";
const GONE = "99999999-8888-4777-8666-555555555555";

const row = (patch: Partial<DeviceRow>): DeviceRow => ({
	device_id: EDGE,
	owner_id: "me",
	name: "edge-berlin-01",
	status: "active",
	registered_at: NOW - 86_400,
	last_seen_at: NOW,
	auth_epoch: 1,
	identity: {} as DeviceRow["identity"],
	...patch,
});

type AttentionHit = Pick<AttentionItem, "key" | "subject">;

const certificateItem = (deviceId: string): AttentionHit => ({
	key: "certificate_expiring",
	subject: { kind: "certificate", deviceId, certificateId: "cert_01" },
});

const fleet = (patch: Partial<ResolveFleet> = {}): ResolveFleet => ({
	devices: [
		row({}),
		row({ device_id: PI, name: "warehouse-pi", display_name: "Warehouse Pi" }),
		row({
			device_id: OLD,
			name: "old-kiosk",
			status: "revoked",
			revoked_at: NOW - 3_600,
		}),
	],
	hub: "api.flow-like.com",
	...patch,
});

function resolveUrl(query: string, input: ResolveFleet) {
	const { scope, route } = parseDevicesRoute(query, "account");
	return { scope, result: resolveTarget(route, input) };
}

describe("legacy ?device=<id> (IA §6.1.4)", () => {
	test("opens Certificates when the device has a certificate attention item", () => {
		const { result } = resolveUrl(
			`device=${EDGE}`,
			fleet({ attention: [certificateItem(EDGE)] }),
		);
		expect(result).toStrictEqual({
			ok: true,
			route: { screen: "device", deviceId: EDGE, tab: "certificates" },
			changed: true,
		});
	});

	test("a certificate key on a device subject counts too", () => {
		const { result } = resolveUrl(
			`device=${EDGE}`,
			fleet({
				attention: [
					{
						key: "certificate_inventory_stale",
						subject: { kind: "device", deviceId: EDGE },
					},
				],
			}),
		);
		expect(result).toMatchObject({ ok: true, route: { tab: "certificates" } });
	});

	test("opens Overview otherwise", () => {
		const items: AttentionHit[] = [
			certificateItem(PI),
			{ key: "offline_since", subject: { kind: "device", deviceId: EDGE } },
			{ key: "keys_missing_here", subject: { kind: "keys" } },
		];
		const { result } = resolveUrl(
			`device=${EDGE}`,
			fleet({ attention: items }),
		);
		expect(result).toStrictEqual({
			ok: true,
			route: { screen: "device", deviceId: EDGE, tab: "overview" },
			changed: true,
		});
	});

	test("opens Overview when attention is not known yet", () => {
		const { result } = resolveUrl(`device=${EDGE}`, fleet());
		expect(result).toMatchObject({ ok: true, route: { tab: "overview" } });
	});

	test("a certificate param without a tab opens Certificates", () => {
		const { result } = resolveUrl(
			`device=${EDGE}&certificate=cert_01`,
			fleet(),
		);
		expect(result).toMatchObject({
			ok: true,
			route: { tab: "certificates", certificateId: "cert_01" },
		});
	});

	test("an explicit tab is kept", () => {
		const { result } = resolveUrl(
			`device=${EDGE}&tab=metrics`,
			fleet({ attention: [certificateItem(EDGE)] }),
		);
		expect(result).toStrictEqual({
			ok: true,
			route: { screen: "device", deviceId: EDGE, tab: "metrics" },
			changed: false,
		});
	});
});

describe("certificate reminder link (E12)", () => {
	test("the new link resolves to the focused certificate", () => {
		const { result } = resolveUrl(
			`device=${EDGE}&tab=certificates&certificate=cert_01`,
			fleet({ certificates: { [EDGE]: ["cert_01", "cert_02"] } }),
		);
		expect(result).toStrictEqual({
			ok: true,
			route: {
				screen: "device",
				deviceId: EDGE,
				tab: "certificates",
				certificateId: "cert_01",
			},
			changed: false,
		});
	});

	test("a certificate no longer in the inventory shows a banner and the tab", () => {
		const { result } = resolveUrl(
			`device=${PI}&tab=certificates&certificate=cert_09`,
			fleet({ certificates: { [PI]: ["cert_01"] } }),
		);
		expect(result).toStrictEqual({
			ok: false,
			banner: {
				code: "certificate_not_found",
				params: { device: "Warehouse Pi", certificate: "cert_09" },
			},
			fallback: { screen: "device", deviceId: PI, tab: "certificates" },
		});
	});

	test("an unreadable inventory never reports a certificate missing", () => {
		const { result } = resolveUrl(
			`device=${PI}&tab=certificates&certificate=cert_09`,
			fleet({ certificates: { [EDGE]: [] } }),
		);
		expect(result).toMatchObject({ ok: true });
	});
});

describe("unknown devices", () => {
	test("not found on this hub", () => {
		const { result } = resolveUrl(`device=${GONE}&tab=keys`, fleet());
		expect(result).toStrictEqual({
			ok: false,
			banner: {
				code: "device_not_found",
				params: { device: GONE, hub: "api.flow-like.com" },
			},
			fallback: { screen: "fleet", view: "devices" },
		});
	});

	test("not shared with you any more", () => {
		const { result } = resolveUrl(
			`device=${GONE}`,
			fleet({ vaults: [{ deviceId: GONE, role: "shared" }] }),
		);
		expect(result).toMatchObject({
			ok: false,
			banner: { code: "device_not_shared", params: { device: GONE } },
		});
	});

	test("an owner vault for an unlisted device reads as not found", () => {
		const { result } = resolveUrl(
			`device=${GONE}`,
			fleet({ vaults: [{ deviceId: GONE, role: "owner" }] }),
		);
		expect(result).toMatchObject({ banner: { code: "device_not_found" } });
	});

	test("signed in as a different account", () => {
		const { result } = resolveUrl(
			`device=${GONE}&service=svc`,
			fleet({ otherAccountDeviceIds: new Set([GONE]) }),
		);
		expect(result).toStrictEqual({
			ok: false,
			banner: { code: "device_other_account", params: { device: GONE } },
			fallback: { screen: "fleet", view: "devices" },
		});
	});

	test("the cleaned URL drops the bad param", () => {
		const { scope, result } = resolveUrl(`device=${GONE}&tab=keys`, fleet());
		if (result.ok) throw new Error("expected a banner");
		expect(devicesHref(result.fallback, scope)).toBe("/settings/devices");
	});
});

describe("revoked devices", () => {
	test("the device page itself still opens", () => {
		const { result } = resolveUrl(`device=${OLD}&tab=overview`, fleet());
		expect(result).toMatchObject({ ok: true, changed: false });
	});

	test("revoke on a revoked device names the date and drops the action", () => {
		const { result } = resolveUrl(
			`device=${OLD}&tab=settings&action=revoke`,
			fleet(),
		);
		expect(result).toStrictEqual({
			ok: false,
			banner: {
				code: "device_revoked",
				params: { device: "old-kiosk", revokedAt: NOW - 3_600 },
			},
			fallback: { screen: "device", deviceId: OLD, tab: "settings" },
		});
	});

	test("a service on a revoked device opens the device", () => {
		const { result } = resolveUrl(`device=${OLD}&service=svc`, fleet());
		expect(result).toStrictEqual({
			ok: false,
			banner: {
				code: "device_revoked",
				params: { device: "old-kiosk", revokedAt: NOW - 3_600 },
			},
			fallback: { screen: "device", deviceId: OLD, tab: "overview" },
		});
	});

	test("an older hub without revoked_at omits the date", () => {
		const input = fleet({
			devices: [row({ device_id: OLD, name: "old-kiosk", status: "revoked" })],
		});
		const { result } = resolveUrl(`device=${OLD}&service=svc`, input);
		expect(result).toMatchObject({
			banner: { code: "device_revoked", params: { device: "old-kiosk" } },
		});
		if (!result.ok)
			expect(result.banner.params).not.toHaveProperty("revokedAt");
	});
});

describe("services", () => {
	test("a known placement opens", () => {
		const { result } = resolveUrl(
			`device=${EDGE}&service=support-bot&tab=activity`,
			fleet({ services: { [EDGE]: ["support-bot"] } }),
		);
		expect(result).toMatchObject({ ok: true, changed: false });
	});

	test("a placement missing from a readable plane shows a banner", () => {
		const { result } = resolveUrl(
			`device=${PI}&service=support-bot`,
			fleet({ services: { [PI]: ["invoice-extractor"] } }),
		);
		expect(result).toStrictEqual({
			ok: false,
			banner: {
				code: "service_not_found",
				params: { device: "Warehouse Pi", service: "support-bot" },
			},
			fallback: { screen: "device", deviceId: PI, tab: "services" },
		});
	});

	test("a locked device never reports a service missing", () => {
		const { result } = resolveUrl(`device=${PI}&service=support-bot`, fleet());
		expect(result).toMatchObject({ ok: true });
	});
});

describe("deploy targets", () => {
	test("unknown and revoked targets leave the plan, the first names the banner", () => {
		const route: DevicesRoute = {
			screen: "deploy",
			deviceIds: [EDGE, GONE, OLD],
			appId: "app_invoice",
			mode: "new",
		};
		expect(resolveTarget(route, fleet())).toStrictEqual({
			ok: false,
			banner: {
				code: "device_not_found",
				params: { device: GONE, hub: "api.flow-like.com" },
			},
			fallback: {
				screen: "deploy",
				deviceIds: [EDGE],
				appId: "app_invoice",
				mode: "new",
			},
		});
	});

	test("an unknown app is dropped so What shows the picker", () => {
		const { result } = resolveUrl(
			`flow=deploy&device=${EDGE}&app=app_gone`,
			fleet({ appIds: new Set(["app_invoice"]) }),
		);
		expect(result).toStrictEqual({
			ok: false,
			banner: { code: "app_not_found", params: { app: "app_gone" } },
			fallback: { screen: "deploy", deviceIds: [EDGE] },
		});
	});

	test("an update of a service the device does not run becomes a new deploy", () => {
		const { result } = resolveUrl(
			`flow=deploy&device=${EDGE}&app=app_crm&service=crm-sync&step=settings`,
			fleet({ services: { [EDGE]: ["support-bot"] } }),
		);
		expect(result).toStrictEqual({
			ok: false,
			banner: {
				code: "service_not_found",
				params: { device: "edge-berlin-01", service: "crm-sync" },
			},
			fallback: {
				screen: "deploy",
				deviceIds: [EDGE],
				appId: "app_crm",
				step: "settings",
			},
		});
	});

	test("the service goes with its only device", () => {
		const route: DevicesRoute = {
			screen: "deploy",
			deviceIds: [GONE],
			serviceId: "crm-sync",
		};
		expect(resolveTarget(route, fleet())).toMatchObject({
			ok: false,
			fallback: { screen: "deploy", deviceIds: [] },
		});
	});

	test("valid targets resolve unchanged", () => {
		const route: DevicesRoute = {
			screen: "deploy",
			deviceIds: [EDGE, PI],
			appId: "app_invoice",
			eventId: "evt_mail",
			from: "events",
		};
		expect(
			resolveTarget(route, fleet({ appIds: new Set(["app_invoice"]) })),
		).toStrictEqual({ ok: true, route, changed: false });
	});
});

describe("other targets", () => {
	test("a finished or expired setup opens the fleet", () => {
		const { result } = resolveUrl(
			"flow=setup&enrollment=enr_old",
			fleet({ pendingEnrollmentIds: new Set(["enr_new"]) }),
		);
		expect(result).toStrictEqual({
			ok: false,
			banner: { code: "setup_not_found", params: { enrollment: "enr_old" } },
			fallback: { screen: "fleet", view: "devices" },
		});
	});

	test("a pending setup resumes; unknown pending list never blocks", () => {
		expect(
			resolveUrl(
				"flow=setup&enrollment=enr_new",
				fleet({ pendingEnrollmentIds: new Set(["enr_new"]) }),
			).result,
		).toMatchObject({ ok: true });
		expect(
			resolveUrl("flow=setup&enrollment=enr_old", fleet()).result,
		).toMatchObject({ ok: true });
	});

	test("an unknown highlight is dropped without a banner", () => {
		expect(
			resolveTarget({ screen: "keys", focusDeviceId: GONE }, fleet()),
		).toStrictEqual({ ok: true, route: { screen: "keys" }, changed: true });
		expect(
			resolveTarget(
				{ screen: "app-devices", by: "event", focusDeviceId: GONE },
				fleet(),
			),
		).toStrictEqual({
			ok: true,
			route: { screen: "app-devices", by: "event" },
			changed: true,
		});
		expect(
			resolveTarget({ screen: "keys", focusDeviceId: PI }, fleet()),
		).toMatchObject({ ok: true, changed: false });
	});

	test("sections without objects always resolve", () => {
		for (const route of [
			{ screen: "hub" },
			{ screen: "fleet", view: "services", filter: "offline" },
			{ screen: "access", tab: "cloud" },
			{ screen: "certificates", tab: "reminders" },
		] satisfies DevicesRoute[]) {
			expect(resolveTarget(route, fleet())).toStrictEqual({
				ok: true,
				route,
				changed: false,
			});
		}
	});
});
