import { readFileSync } from "node:fs";
import { createRequire } from "node:module";
import { describe, expect, test } from "vitest";
import desktop from "../../src-tauri/capabilities/desktop.json";
import mobile from "../../src-tauri/capabilities/mobile.json";
import config from "../../src-tauri/tauri.conf.json";

const require = createRequire(import.meta.url);
type Permission = string | { identifier: string; allow?: unknown[] };
const identifier = (permission: Permission) =>
	typeof permission === "string" ? permission : permission.identifier;

describe("native capability boundaries", () => {
	test("desktop can open configured storage paths with their default application", () => {
		const pathPermission = desktop.permissions.find(
			(permission) => identifier(permission) === "opener:allow-open-path",
		);
		const storageScope = desktop.permissions.find(
			(permission) => identifier(permission) === "fs:scope",
		);
		expect(pathPermission).toBeTypeOf("object");
		expect(storageScope).toBeTypeOf("object");
		if (
			typeof pathPermission !== "object" ||
			typeof storageScope !== "object"
		) {
			throw new Error("Storage needs a scoped file opener permission");
		}
		expect(pathPermission.allow).toEqual(storageScope.allow);
		for (const entry of pathPermission.allow ?? []) {
			expect(entry).not.toHaveProperty("app");
		}
	});

	test("mobile does not grant external file opening", () => {
		expect(mobile.permissions.map(identifier)).not.toContain(
			"opener:allow-open-path",
		);
	});

	test("desktop updates can release resources and show dialogs; mobile cannot update", () => {
		expect(desktop.platforms).toEqual(["macOS", "windows", "linux"]);
		expect(desktop.permissions.map(identifier)).toEqual(
			expect.arrayContaining([
				"updater:default",
				"core:resources:default",
				"core:app:default",
				"core:event:default",
				"dialog:allow-message",
			]),
		);
		expect(
			mobile.permissions
				.map(identifier)
				.some((id) => id.startsWith("updater:")),
		).toBe(false);
	});

	test("the release signer supports the required signed updater version", () => {
		expect(config.plugins.updater.requireSignedVersion).toBe(true);
		expect(config.plugins.updater.allowDowngrades).toBe(false);
		expect(config.bundle.createUpdaterArtifacts).toBe(true);
		const cli = JSON.parse(
			readFileSync(require.resolve("@tauri-apps/cli/package.json"), "utf8"),
		) as { version: string };
		const [major, minor, patch] = cli.version.split(".").map(Number);
		// CLI 2.12.1 first bound the app version into `tauri build` signatures.
		expect(
			major > 2 ||
				(major === 2 && (minor > 12 || (minor === 12 && patch >= 1))),
		).toBe(true);
	});
});
