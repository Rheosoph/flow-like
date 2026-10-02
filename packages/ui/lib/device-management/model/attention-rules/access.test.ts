import { describe, expect, test } from "bun:test";
import {
	SAMPLE_IDS,
	SAMPLE_NOW,
	SAMPLE_PEOPLE,
	ed25519,
	sampleFleet,
} from "../__fixtures__/sample-fleet";
import type { AttentionInputExt } from "../attention";
import type { AttentionKey } from "../types";
import { ACCESS_RULES, pendingAccessRequests } from "./access";

const evaluate = (key: AttentionKey, input: AttentionInputExt) =>
	ACCESS_RULES.find((rule) => rule.key === key)?.evaluate(input) ?? [];

function edgePolicy(input: AttentionInputExt) {
	const policy = input.policies[SAMPLE_IDS.edge].policy;
	if (!policy) throw new Error("fixture: edge policy");
	return policy;
}

describe("my access as a recipient", () => {
	test("ending within 6 h is a Warning: BG1 row, else BG22 my-access, else the verified policy", () => {
		const input = sampleFleet();
		expect(evaluate("shared_access_expiring", input)).toEqual([]);
		const lab = input.devices.find((row) => row.device_id === SAMPLE_IDS.lab);
		if (!lab) throw new Error("fixture");
		lab.access_expires_at = SAMPLE_NOW + 3_600;
		const [item] = evaluate("shared_access_expiring", { ...input });
		expect(item.severity).toBe("warning");
		expect(item.action?.target).toEqual({
			kind: "ask_to_renew",
			deviceId: SAMPLE_IDS.lab,
		});
		lab.access_expires_at = undefined;
		const grant = input.myAccess?.[SAMPLE_IDS.lab]?.grants[0];
		if (!grant) throw new Error("fixture");
		grant.expires_at = SAMPLE_NOW + 7_200;
		expect(evaluate("shared_access_expiring", { ...input })).toHaveLength(1);
		input.myAccess = undefined;
		expect(evaluate("shared_access_expiring", { ...input })).toEqual([]);
		input.fleet[SAMPLE_IDS.lab].policy = {
			version: 3,
			myGrant: {
				grant_id: "g",
				user_id: input.me,
				controller_key: ed25519("me"),
				scope: { kind: "device" },
				capabilities: ["status"],
				expires_at: SAMPLE_NOW + 60,
				group_id: null,
				group_version: null,
			},
		};
		expect(evaluate("shared_access_expiring", { ...input })).toHaveLength(1);
	});

	test("ended: BG1 expiry passed, or a shared vault the hub no longer lists", () => {
		const input = sampleFleet();
		expect(evaluate("shared_access_ended", input)).toEqual([]);
		const lab = input.devices.find((row) => row.device_id === SAMPLE_IDS.lab);
		if (!lab) throw new Error("fixture");
		lab.access_expires_at = SAMPLE_NOW - 1;
		const [listed] = evaluate("shared_access_ended", { ...input });
		expect(listed.secondary?.code).toBe("remove_from_computer");
		input.devices = input.devices.filter(
			(row) => row.device_id !== SAMPLE_IDS.lab,
		);
		const gone = evaluate("shared_access_ended", { ...input });
		expect(
			gone.map(
				(item) => item.subject.kind === "access" && item.subject.deviceId,
			),
		).toEqual([SAMPLE_IDS.lab]);
		expect(gone[0].copy.params?.device).toBe(SAMPLE_IDS.lab.slice(0, 8));
		expect(
			evaluate("shared_access_ended", { ...input, accessRequests: undefined }),
		).toEqual([]);
	});

	test("a device list that hasn't loaded, or can't be read, ends nobody's access", () => {
		const input = sampleFleet();
		const ended = (change: Partial<typeof input>) =>
			evaluate("shared_access_ended", { ...input, ...change }).length;
		expect(ended({ devices: [] })).toBe(0);
		expect(ended({ devices: [], devicesLoaded: false })).toBe(0);
		// The hub answered with no devices: the shared vault's device really is gone.
		expect(ended({ devices: [], devicesLoaded: true })).toBe(1);
		const withoutLab = input.devices.filter(
			(row) => row.device_id !== SAMPLE_IDS.lab,
		);
		expect(ended({ devices: withoutLab })).toBe(1);
		expect(ended({ devices: withoutLab, devicesLoaded: false })).toBe(0);
	});

	test("pending requests are Info until the hub lists the device", () => {
		const input = sampleFleet();
		const [item] = evaluate("access_request_pending", input);
		expect(item.copy.params).toEqual({
			device: "mira-render-01",
			owner: SAMPLE_PEOPLE.mira,
		});
		input.devices.push({
			...input.devices[3],
			device_id: SAMPLE_IDS.miraRender,
			name: "mira-render-01",
		});
		expect(pendingAccessRequests({ ...input })).toEqual([]);
	});
});

describe("access I gave as the owner", () => {
	test("a person's access ending within 6 h is a Notice per person", () => {
		const items = evaluate("grant_expiring", sampleFleet());
		expect(items).toHaveLength(1);
		expect(items[0].subject).toEqual({
			kind: "access",
			deviceId: SAMPLE_IDS.edge,
			personId: SAMPLE_PEOPLE.mira,
		});
		expect(items[0].copy.params).toMatchObject({
			person: SAMPLE_PEOPLE.mira,
			expiresAt: 1_790_791_200,
		});
	});

	test("a saved change waiting for the device dwells 2 min, then escalates after 10 min online", () => {
		const input = sampleFleet();
		const [waiting] = evaluate("sharing_policy_waiting_for_device", input);
		expect(waiting).toMatchObject({
			severity: "notice",
			dwellS: 120,
			since: 1_790_769_120,
		});
		const now = 1_790_769_120 + 601;
		const studio = input.devices.find(
			(row) => row.device_id === SAMPLE_IDS.studio,
		);
		if (!studio) throw new Error("fixture");
		studio.last_seen_at = now - 30;
		expect(
			evaluate("sharing_policy_waiting_for_device", { ...input, now })[0]
				.severity,
		).toBe("warning");
		studio.last_seen_at = now - 900;
		expect(
			evaluate("sharing_policy_waiting_for_device", { ...input, now })[0]
				.severity,
		).toBe("notice");
	});

	test("rules expiring within 7 days matter only with grants or history readers", () => {
		const input = sampleFleet();
		edgePolicy(input).expires_at = SAMPLE_NOW + 3 * 86_400;
		expect(evaluate("sharing_policy_expiring", input)[0]?.severity).toBe(
			"warning",
		);
		edgePolicy(input).grants = [];
		input.live[SAMPLE_IDS.edge].history = [];
		expect(evaluate("sharing_policy_expiring", { ...input })).toEqual([]);
	});

	test("expired rules: Critical when they carried grants, otherwise a Notice", () => {
		const input = sampleFleet();
		edgePolicy(input).expires_at = SAMPLE_NOW - 60;
		expect(evaluate("sharing_policy_expired", input)[0]?.severity).toBe(
			"critical",
		);
		expect(evaluate("grant_expiring", input)).toEqual([]);
		edgePolicy(input).grants = [];
		input.live[SAMPLE_IDS.edge].history = [];
		expect(evaluate("sharing_policy_expired", { ...input })[0]?.severity).toBe(
			"notice",
		);
	});

	test("20 of 24 grants is nearly full", () => {
		const input = sampleFleet();
		const policy = edgePolicy(input);
		policy.grants = Array.from({ length: 20 }, (_, index) => ({
			...policy.grants[0],
			grant_id: `g-${index}`,
			user_id: `user-${index}`,
		}));
		expect(
			evaluate("access_slots_nearly_full", input)[0]?.copy.params,
		).toMatchObject({ used: 20, max: 24 });
	});

	test("code-running access without a required sandbox, per person, needs the live isolation fact", () => {
		const input = sampleFleet();
		expect(evaluate("code_running_access_without_sandbox", input)).toEqual([]);
		const inspection = input.live[SAMPLE_IDS.edge].inspection;
		if (!inspection) throw new Error("fixture");
		inspection.value.hostIsolation = "optional";
		const items = evaluate("code_running_access_without_sandbox", { ...input });
		expect(
			items.map(
				(item) => item.subject.kind === "access" && item.subject.personId,
			),
		).toEqual([SAMPLE_PEOPLE.jonas]);
		inspection.value.hostIsolation = undefined;
		expect(
			evaluate("code_running_access_without_sandbox", { ...input }),
		).toEqual([]);
	});
});
