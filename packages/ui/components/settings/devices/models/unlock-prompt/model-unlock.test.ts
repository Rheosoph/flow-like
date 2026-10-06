import { describe, expect, test } from "bun:test";
import type {
	DeviceAccountScope,
	LocalDeviceVault,
} from "../../../../../lib/device-management/storage";
import type { IProfile } from "../../../../../types";
import {
	type ModelUnlockPrompt,
	deviceUnlockRequest,
	lookupVault,
	unlockFailure,
	vaultDeviceName,
	withPrompts,
	withoutPrompt,
} from "./model-unlock";

const OWNER_KEY = { kty: "OKP", crv: "Ed25519", x: "A".repeat(43) } as const;

function vault(overrides: Partial<LocalDeviceVault> = {}): LocalDeviceVault {
	return {
		deviceId: "device-1",
		controllerPublic: {} as LocalDeviceVault["controllerPublic"],
		controllerVault: Uint8Array.of(1, 2, 3),
		manifestJws: "header.e30.signature",
		grantId: "grant-1",
		ownerControllerKey: OWNER_KEY,
		...overrides,
	};
}

const profile = { id: "profile-1", hub: "api.flow-like.test" } as IProfile;
const signedIn = { issuer: "https://issuer.test", account: "user-1" };

function prompt(id: string): ModelUnlockPrompt {
	return { id, deviceId: "device-1", modelName: "Qwen3 8B", expiresAt: 0 };
}

describe("lookupVault", () => {
	test("reads the vault under the scope of the device area", async () => {
		const scopes: DeviceAccountScope[] = [];
		const stored = vault();
		const lookup = await lookupVault(
			async (scope) => {
				scopes.push(scope);
				return stored;
			},
			signedIn,
			async () => profile,
			"device-1",
		);
		expect(scopes).toEqual([
			{
				issuer: "https://issuer.test",
				account: "user-1",
				apiOrigin: "https://api.flow-like.test",
				profileId: "profile-1",
			},
		]);
		expect(lookup).toEqual({ kind: "ready", scope: scopes[0], vault: stored });
	});

	test("blocks without an account, without keys and before a fresh endpoint", async () => {
		const read = async () => vault({ requiresFreshEndpoint: true });
		expect(
			await lookupVault(read, undefined, async () => profile, "device-1"),
		).toEqual({ kind: "blocked", block: "signed_out" });
		expect(
			await lookupVault(
				async () => undefined,
				signedIn,
				async () => profile,
				"device-1",
			),
		).toEqual({ kind: "blocked", block: "no_vault" });
		expect(
			await lookupVault(read, signedIn, async () => profile, "device-1"),
		).toEqual({ kind: "blocked", block: "fresh_endpoint" });
	});
});

describe("deviceUnlockRequest", () => {
	test("carries the encrypted vault and its public fields, never more", () => {
		const scope = {
			issuer: "https://issuer.test",
			account: "user-1",
			apiOrigin: "https://api.flow-like.test",
			profileId: "default",
		};
		const request = deviceUnlockRequest(scope, vault(), "secret", false);
		expect(request).toEqual({
			deviceId: "device-1",
			password: "secret",
			controllerVault: [1, 2, 3],
			manifestJws: "header.e30.signature",
			grantId: "grant-1",
			ownerControllerKey: OWNER_KEY,
			apiOrigin: "https://api.flow-like.test",
			account: "user-1",
			keepUnlocked: false,
		});
		const owner = deviceUnlockRequest(
			scope,
			vault({ grantId: "owner", ownerControllerKey: undefined }),
			"secret",
			true,
		);
		expect("ownerControllerKey" in owner).toBe(false);
	});
});

describe("prompt helpers", () => {
	test("refusals map to their codes and anything else to failed", () => {
		expect(unlockFailure("wrong_password: the vault did not open")).toBe(
			"wrong_password",
		);
		expect(unlockFailure(new Error("device_unavailable: HTTP 404"))).toBe(
			"device_unavailable",
		);
		expect(unlockFailure("invalid: main window only")).toBe("failed");
		expect(unlockFailure(undefined)).toBe("failed");
	});

	test("the queue keeps arrival order and drops repeats", () => {
		const queue = withPrompts([prompt("a")], [prompt("a"), prompt("b")]);
		expect(queue.map((entry) => entry.id)).toEqual(["a", "b"]);
		expect(withoutPrompt(queue, "a").map((entry) => entry.id)).toEqual(["b"]);
	});

	test("an unreadable manifest has no device name", () => {
		expect(vaultDeviceName(vault({ manifestJws: "not-a-jws" }))).toBe(
			undefined,
		);
	});
});
