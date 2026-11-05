import { describe, expect, test } from "bun:test";
import {
	ROUTES,
	configBytes,
} from "../../../../lib/device-management/model/__fixtures__/apps";
import type { AppEventInput } from "../../../../lib/device-management/model/app-plan";
import type {
	DeployPlan,
	PlanException,
	PlanTarget,
} from "../../../../lib/device-management/model/deploy-plan";
import { endpointCalls } from "./endpoint-calls";

const event = (
	id: string,
	type: string,
	config?: unknown,
	page?: string,
): AppEventInput => ({
	id,
	name: `Event ${id}`,
	active: true,
	event_type: type,
	event_version: [1, 0, 0],
	board_version: [1, 0, 0],
	default_page_id: page ?? null,
	...(config ? { config: configBytes(config) } : {}),
});

const EVENTS = [
	event("orders", "api", ROUTES.orders),
	event("notes", "http", ROUTES.notes),
	event("chat", "simple_chat"),
	event("page", "http", ROUTES.support, "page-1"),
];

function target(change: Partial<PlanTarget["endpoint"]> = {}): PlanTarget {
	return {
		deviceId: "pi",
		name: "Pi",
		gate: null,
		locked: false,
		runsAsAgent: false,
		endpoint: { host: "127.0.0.1", port: 8080, certificateId: null, ...change },
		services: [
			{
				key: "a",
				serviceId: "shop",
				kind: "new",
				events: ["orders", "chat", "page"],
				leftOut: [],
				removedEvents: [],
				addedSchedules: [],
				addedBots: [],
			},
			{
				key: "b",
				serviceId: "notes",
				kind: "new",
				events: ["notes"],
				leftOut: [],
				removedEvents: [],
				addedSchedules: [],
				addedBots: [],
				port: 8081,
			},
		],
	};
}

function plan(
	token: DeployPlan["draft"]["endpoint"]["token"] = "per_device",
	change: Partial<PlanTarget["endpoint"]> = {},
): DeployPlan {
	return {
		draft: {
			endpoint: { host: null, port: null, token, tokenValue: "" },
		},
		app: { id: "app", name: "Shop", visibility: "Private", events: EVENTS },
		mode: "online",
		services: [],
		targets: [target(change)],
	} as unknown as DeployPlan;
}

const ROWS = [
	{ target: "pi/shop", deviceId: "pi", serviceId: "shop", state: "done" },
	{ target: "pi/notes", deviceId: "pi", serviceId: "notes", state: "done" },
] as const;
const DETAILS = {
	"pi/shop": { deviceName: "Pi", serviceId: "shop", token: "t".repeat(40) },
	"pi/notes": { deviceName: "Pi", serviceId: "notes" },
};

describe("endpointCalls", () => {
	test("each Endpoint of a finished service, at its own port, with the token this page still holds", () => {
		expect(endpointCalls(plan(), ROWS, DETAILS)).toEqual([
			{
				key: "pi/shop/orders",
				target: "pi/shop",
				deviceId: "pi",
				deviceName: "Pi",
				serviceId: "shop",
				eventName: "Event orders",
				route: { method: "GET", path: "/orders" },
				where: { origin: "http://127.0.0.1:8080", scheme: "http", port: 8080 },
				requiresToken: true,
				token: "t".repeat(40),
			},
			{
				key: "pi/notes/notes",
				target: "pi/notes",
				deviceId: "pi",
				deviceName: "Pi",
				serviceId: "notes",
				eventName: "Event notes",
				route: { method: "GET", path: "/notes" },
				where: { origin: "http://127.0.0.1:8081", scheme: "http", port: 8081 },
				requiresToken: true,
			},
		]);
	});

	test("no token without authentication, https with a certificate, no host on all networks", () => {
		const [call] = endpointCalls(
			plan("none", { host: "0.0.0.0", certificateId: "cert" }),
			ROWS,
			DETAILS,
		);
		expect(call.requiresToken).toBe(false);
		expect(call.token).toBeUndefined();
		expect(call.where).toEqual({
			origin: undefined,
			scheme: "https",
			port: 8080,
		});
	});

	test("an update that keeps the address, certificate or token, and an unfinished row, have none", () => {
		expect(endpointCalls(plan("keep"), ROWS, DETAILS)).toEqual([]);
		expect(
			endpointCalls(plan("per_device", { host: null }), ROWS, DETAILS),
		).toEqual([]);
		expect(
			endpointCalls(
				plan("per_device", { certificateId: undefined }),
				ROWS,
				DETAILS,
			),
		).toEqual([]);
		expect(
			endpointCalls(plan(), [{ ...ROWS[0], state: "failed" }], DETAILS),
		).toEqual([]);
	});

	test("a port Docker doesn't forward says which one it does", () => {
		const exception: PlanException = {
			code: "port_unpublished",
			step: "endpoint",
			deviceId: "pi",
			tone: "info",
			params: { port: 8081, service: "notes", published: 8080 },
		};
		const calls = endpointCalls(plan(), ROWS, DETAILS, {
			exceptions: [exception],
		});
		expect(calls.map((call) => call.publishedPort)).toEqual([undefined, 8080]);
	});
});
