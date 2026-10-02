import { describe, expect, test } from "bun:test";
import type {
	PlanIssue,
	PlanIssueCode,
} from "../../../../lib/device-management/model/deploy-plan";
import {
	APPROVAL_ISSUE_CODES,
	PLAN_ISSUE_CODES,
} from "../../../../lib/device-management/model/deploy-plan";
import type {
	DeployRoute,
	DevicesScope,
} from "../../../../lib/device-management/model/types";
import type { DevicesT } from "../primitives/area-context";
import { deployExitHref, devicesHref } from "../routing/devices-href";
import {
	currentStep,
	deploySteps,
	exitRoute,
	issueText,
	prefilledSteps,
	stepLabel,
	stepTitle,
} from "./deploy-copy";

/** English defaults with their `{{param}}` slots filled, as i18next renders them before extraction. */
const t = ((
	_key: string,
	fallback: string,
	params: Record<string, unknown> = {},
) =>
	fallback.replace(/\{\{(\w+)(?:, \w+)?\}\}/g, (_, name) =>
		String(params[name] ?? ""),
	)) as unknown as DevicesT;

const APP: DevicesScope = { kind: "app", appId: "app_visitor_checkin" };
const ACCOUNT: DevicesScope = { kind: "account" };

describe("steps", () => {
	test("step 6 follows the app's mode", () => {
		expect(deploySteps("online")[5]).toBe("access_cost");
		expect(deploySteps("offline")[5]).toBe("copy_upload");
		expect(deploySteps(null)).toHaveLength(8);
	});

	test("labels: step 5 reads Limits when nothing is served by the device", () => {
		expect(stepLabel(t, "endpoint")).toBe("Endpoint & limits");
		expect(stepLabel(t, "endpoint", true)).toBe("Limits");
		expect(stepTitle(t, "what")).toBe("What to run");
		expect(stepTitle(t, "where")).toBe("Where it runs");
		expect(stepTitle(t, "copy_upload")).toBe("Copy & upload");
	});

	test("a route opens its own step; the other mode's step 6 is corrected", () => {
		const fresh = { resumed: false, reached: 0 };
		const online = deploySteps("online");
		expect(currentStep({ step: "settings" }, online, fresh)).toBe("settings");
		expect(currentStep({ step: "copy_upload" }, online, fresh)).toBe(
			"access_cost",
		);
		expect(
			currentStep({ step: "access_cost" }, deploySteps("offline"), fresh),
		).toBe("copy_upload");
		expect(currentStep({}, online, fresh)).toBe("what");
	});

	test("a resumed draft lands on its furthest step, Review at most", () => {
		const online = deploySteps("online");
		expect(currentStep({}, online, { resumed: true, reached: 4 })).toBe(
			"endpoint",
		);
		expect(currentStep({}, online, { resumed: true, reached: 7 })).toBe(
			"review",
		);
	});

	test("prefilled steps come from the entry", () => {
		expect([...prefilledSteps({ deviceIds: [] }, APP)]).toEqual(["what"]);
		expect([...prefilledSteps({ deviceIds: ["d1"] }, ACCOUNT)]).toEqual([
			"where",
		]);
		expect([
			...prefilledSteps({ deviceIds: ["d1"], appId: "app_x" }, ACCOUNT),
		]).toEqual(["what", "where"]);
	});
});

describe("exit", () => {
	const routes: [Omit<DeployRoute, "screen">, DevicesScope][] = [
		[{ deviceIds: [], mode: "new" }, APP],
		[{ deviceIds: ["d1", "d2"], mode: "update" }, APP],
		[{ deviceIds: ["d1"] }, ACCOUNT],
		[{ deviceIds: ["d1"], appId: "app_x", serviceId: "svc" }, ACCOUNT],
		[{ deviceIds: [] }, ACCOUNT],
		[{ deviceIds: ["d1", "d2"] }, ACCOUNT],
	];
	for (const [route, scope] of routes)
		test(`${scope.kind} · ${JSON.stringify(route)} matches deployExitHref`, () => {
			const full: DeployRoute = { screen: "deploy", ...route };
			expect(devicesHref(exitRoute(full, scope), scope)).toBe(
				deployExitHref(full, scope),
			);
		});
});

describe("issue sentences (R3)", () => {
	const codes: PlanIssueCode[] = [
		...PLAN_ISSUE_CODES,
		...APPROVAL_ISSUE_CODES.map((code) => `approval.${code}` as const),
	];
	const names = { app: "CRM Sync", device: () => "edge-berlin-01" };

	test("every check code has a sentence without codes in it", () => {
		for (const code of codes) {
			const issue: PlanIssue = {
				code,
				step: "what",
				severity: "error",
				deviceId: "d1",
				params: {
					service: "crm-webhook",
					variable: "Batch size",
					port: 8081,
					count: 2,
				},
			};
			const sentence = issueText(t, issue, names);
			expect(sentence.length).toBeGreaterThan(10);
			expect(sentence).not.toMatch(/\{\{|undefined|\b[a-z]+[_.][a-z_.]+\b/);
		}
	});

	test("a port in use names the service that holds it, when the device says", () => {
		const issue: PlanIssue = {
			code: "port_in_use",
			step: "endpoint",
			severity: "error",
			deviceId: "d1",
			params: { port: 8081, service: "invoice-extractor" },
		};
		expect(issueText(t, issue, names)).toBe(
			"Port 8081 is used by invoice-extractor on edge-berlin-01.",
		);
		expect(issueText(t, { ...issue, params: { port: 80 } }, names)).toBe(
			"Port 80 is already in use on edge-berlin-01.",
		);
	});

	test("the same id twice reads differently for the plan and for one device", () => {
		const base: PlanIssue = {
			code: "service_id_twice",
			step: "what",
			severity: "error",
		};
		expect(issueText(t, base, names)).toBe("Each service needs its own ID.");
		expect(
			issueText(
				t,
				{
					...base,
					step: "where",
					deviceId: "d1",
					params: { service: "nightly-sync" },
				},
				names,
			),
		).toContain("would both become nightly-sync on edge-berlin-01");
	});
});
