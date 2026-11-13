import { describe, expect, test } from "bun:test";
import type { IBotSecretInfo } from "../../../../state/backend-state/bot-state";
import { secretFormProblem } from "./secrets-step";

const sealedSecret: IBotSecretInfo = {
	name: "API_KEY",
	encryption: "sealed",
	description: null,
	created_at: "2026-11-12T00:00:00Z",
	updated_at: "2026-11-12T00:00:00Z",
	last_used_at: null,
};

const replacing = {
	name: "API_KEY",
	value: "sk-new",
	description: null,
	replacing: true,
};

describe("secretFormProblem", () => {
	test("a sealed secret is replaced only where it can be sealed again", () => {
		expect(secretFormProblem(replacing, [sealedSecret])).toBe("sealed");
		expect(secretFormProblem(replacing, [sealedSecret], "checking")).toBe(
			"sealed",
		);
		expect(secretFormProblem(replacing, [sealedSecret], "available")).toBe(
			null,
		);
	});

	test("a server secret is replaced anywhere", () => {
		const server = { ...sealedSecret, encryption: "server" as const };
		expect(secretFormProblem(replacing, [server])).toBe(null);
	});
});
