import { describe, expect, test } from "bun:test";
import {
	SAMPLE_IDS,
	SAMPLE_NOW,
	keySession,
	sampleFleet,
	vault,
} from "../__fixtures__/sample-fleet";
import type { AttentionInputExt } from "../attention";
import type { AttentionKey } from "../types";
import { KEY_RULES } from "./keys";

const evaluate = (key: AttentionKey, input: AttentionInputExt) =>
	KEY_RULES.find((rule) => rule.key === key)?.evaluate(input) ?? [];

const deviceIds = (key: AttentionKey, input: AttentionInputExt) =>
	evaluate(key, input).map((item) =>
		item.subject.kind === "keys" ? (item.subject.deviceId ?? "*") : "?",
	);

function withoutVault(input: AttentionInputExt, deviceId: string) {
	input.local.vaults = input.local.vaults.filter(
		(entry) => entry.deviceId !== deviceId,
	);
	input.keys = input.keys.filter((entry) => entry.deviceId !== deviceId);
	return input;
}

describe("keys on this computer", () => {
	test("missing keys: Warning for owners, Notice for recipients, nothing when the relationship is unknown", () => {
		const input = withoutVault(
			withoutVault(sampleFleet(), SAMPLE_IDS.edge),
			SAMPLE_IDS.lab,
		);
		const items = evaluate("keys_missing_here", input);
		expect(
			items.map((item) => [
				item.subject.kind === "keys" && item.subject.deviceId,
				item.severity,
			]),
		).toEqual([
			[SAMPLE_IDS.edge, "warning"],
			[SAMPLE_IDS.lab, "notice"],
		]);
		expect(items[0].action?.target).toEqual({
			kind: "restore_keys",
			deviceId: SAMPLE_IDS.edge,
		});
		for (const row of input.devices) row.relationship = undefined;
		expect(deviceIds("keys_missing_here", { ...input })).toEqual([
			SAMPLE_IDS.edge,
		]);
	});

	test("keys not backed up: revision 0, or absent from a complete BG25 list", () => {
		const input = sampleFleet();
		expect(deviceIds("keys_not_backed_up_to_account", input)).toEqual([
			SAMPLE_IDS.cold,
		]);
		delete input.accountBackups[SAMPLE_IDS.cold];
		expect(deviceIds("keys_not_backed_up_to_account", { ...input })).toEqual([
			SAMPLE_IDS.cold,
		]);
		expect(
			deviceIds("keys_not_backed_up_to_account", {
				...input,
				accountBackupSlots: undefined,
			}),
		).toEqual([]);
	});

	test("a pending upload replaces 'not backed up'", () => {
		const input = sampleFleet();
		input.local.backups[SAMPLE_IDS.cold] = { localRevision: 1, pending: true };
		expect(deviceIds("keys_not_backed_up_to_account", input)).toEqual([]);
		expect(deviceIds("account_backup_upload_pending", input).sort()).toEqual(
			[SAMPLE_IDS.cold, SAMPLE_IDS.studio].sort(),
		);
	});

	test("old password (W1-KEYS flag) and a newer hub backup", () => {
		const input = sampleFleet();
		input.local.backups[SAMPLE_IDS.edge] = {
			localRevision: 2,
			pending: false,
			passwordChangedSinceBackup: true,
		};
		expect(deviceIds("account_backup_old_password", input)).toEqual([
			SAMPLE_IDS.edge,
		]);
		const [newer] = evaluate("account_backup_hub_newer", input);
		expect(newer.severity).toBe("warning");
		expect(newer.copy.params).toMatchObject({
			hubRevision: 3,
			localRevision: 2,
		});
	});

	test("browser storage that may be cleared (web only, known state only)", () => {
		const input = sampleFleet();
		input.local.persistence = "denied";
		expect(evaluate("storage_not_persistent", input)).toEqual([]);
		input.local.platform = "web";
		expect(evaluate("storage_not_persistent", input)[0]?.copy.params).toEqual({
			count: 7,
		});
		input.local.persistence = "unknown";
		expect(evaluate("storage_not_persistent", input)).toEqual([]);
	});

	test("a pending request's keys exist only here (Info)", () => {
		const input = sampleFleet();
		expect(deviceIds("request_keys_unbacked", input)).toEqual([
			SAMPLE_IDS.miraRender,
		]);
		input.accessRequests = [];
		expect(deviceIds("request_keys_unbacked", input)).toEqual([]);
	});

	test("backup slots: 240 of 256", () => {
		const input = sampleFleet();
		input.accountBackupSlots = { used: 239, max: 256 };
		expect(evaluate("backup_slots_nearly_full", input)).toEqual([]);
		input.accountBackupSlots = { used: 240, max: 256 };
		expect(evaluate("backup_slots_nearly_full", input)[0]?.severity).toBe(
			"notice",
		);
	});

	test("stale keys: a revoked device, or a request abandoned for 30 days", () => {
		const input = sampleFleet();
		expect(deviceIds("stale_local_keys", input)).toEqual([SAMPLE_IDS.oldKiosk]);
		const request = input.accessRequests?.[0];
		if (!request) throw new Error("fixture");
		request.createdAt = SAMPLE_NOW - 31 * 86_400;
		expect(deviceIds("stale_local_keys", { ...input })).toEqual([
			SAMPLE_IDS.oldKiosk,
			SAMPLE_IDS.miraRender,
		]);
	});

	test("an approved request is no longer pending", () => {
		const input = sampleFleet();
		input.devices.push({
			...input.devices[3],
			device_id: SAMPLE_IDS.miraRender,
			name: "mira-render-01",
		});
		input.keys.push(keySession(SAMPLE_IDS.miraRender, "locked", "shared", "g"));
		input.local.vaults.push(vault(SAMPLE_IDS.miraRender, "shared", "g"));
		expect(deviceIds("request_keys_unbacked", input)).toEqual([]);
	});
});
