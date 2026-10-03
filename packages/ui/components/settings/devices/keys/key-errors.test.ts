import { describe, expect, test } from "bun:test";
import {
	DeviceLockHeldError,
	DeviceVaultExistsError,
} from "../../../../lib/device-management/storage";
import { KeySessionError } from "../../../../lib/device-management/workspace/keys";
import type { DevicesT } from "../primitives/area-context";
import { KeyFlowError, classifyKeyFailure, keyFailureCopy } from "./key-errors";

/** The English default of every `t()` call, with its params filled in. */
const t = ((_key: string, fallback: string, params?: Record<string, unknown>) =>
	fallback.replace(/\{\{(\w+)[^}]*\}\}/g, (_match, name: string) =>
		String(params?.[name] ?? ""),
	)) as unknown as DevicesT;

const status = (code: number) =>
	Object.assign(new Error(`HTTP ${code}`), { status: code, code: "X" });

describe("classifyKeyFailure", () => {
	test.each([
		[new KeyFlowError("limit"), "limit"],
		[new KeySessionError({ code: "wrong_password" }), "wrong_password"],
		[new KeySessionError({ code: "no_vault" }), "no_keys"],
		[new KeySessionError({ code: "held_elsewhere" }), "held_elsewhere"],
		[new DeviceVaultExistsError("d1"), "exists"],
		[new DeviceLockHeldError(), "held_elsewhere"],
		[new DOMException("Aborted", "AbortError"), "cancelled"],
		[new SyntaxError("Unexpected token"), "not_a_backup"],
		[status(403), "not_allowed"],
		[status(404), "no_backup"],
		[status(409), "conflict"],
		[status(503), "hub"],
		[new TypeError("Failed to fetch"), "hub"],
		[
			new Error(
				"The downloaded account backup is older than this app has seen.",
			),
			"older_than_seen",
		],
		[
			new Error(
				"The pending backup has not reached the account yet. Save the account backup again to finish its upload.",
			),
			"pending_first",
		],
		[
			new Error("This backup belongs to another device or hub."),
			"other_device",
		],
		[new Error("Controller backups must be smaller than 1 MiB."), "too_large"],
		[new Error("Choose a different password."), "same_password"],
		[
			new Error(
				"This app has a different controller. Export its backup before replacing local keys.",
			),
			"different_keys",
		],
	] as const)("%p is filed as %s", (error, code) => {
		expect(classifyKeyFailure(error).code).toBe(code);
	});

	test("an unexplained failure takes the step's meaning, so crypto texts never surface", () => {
		const crypto = new Error("aead::Error at vault.rs:132");
		expect(classifyKeyFailure(crypto, "wrong_backup_password").code).toBe(
			"wrong_backup_password",
		);
		expect(classifyKeyFailure(crypto).code).toBe("unknown");
		expect(
			keyFailureCopy(t, classifyKeyFailure(crypto, "wrong_password"), "edge"),
		).not.toContain("aead");
	});
});

describe("keyFailureCopy", () => {
	test("names the device and stays in plain words", () => {
		expect(
			keyFailureCopy(t, { code: "wrong_password" }, "edge-berlin-01"),
		).toBe(
			"That password doesn't open the keys for edge-berlin-01 on this computer.",
		);
		expect(keyFailureCopy(t, { code: "exists" }, "edge-berlin-01")).toBe(
			"Other keys for edge-berlin-01 already exist on this computer.",
		);
		expect(keyFailureCopy(t, { code: "cancelled" }, "edge-berlin-01")).toBe("");
	});

	test("a hub failure says the hub's part and that nothing changed here", () => {
		const text = keyFailureCopy(
			t,
			{ code: "hub", hub: "server_error" },
			"edge-berlin-01",
		);
		expect(text).toContain("Nothing was changed on this computer");
	});
});
