import { describe, expect, test } from "bun:test";
import type { IBotSkill } from "../../state/backend-state/bot-state";
import {
	canApprove,
	canEdit,
	draftProblem,
	sortSkills,
	sourceKind,
} from "./bot-skill-model";

function skill(patch: Partial<IBotSkill>): IBotSkill {
	return {
		id: "s1",
		bot_id: "b1",
		name: "pdf",
		description: "Fill PDF forms",
		body: "1. Do it",
		source: "owner",
		status: "active",
		tools: [],
		...patch,
	};
}

describe("sortSkills", () => {
	test("skills that wait for the owner come first, then active, then off", () => {
		const sorted = sortSkills([
			skill({ id: "a", name: "zeta", status: "disabled" }),
			skill({ id: "b", name: "beta", status: "active" }),
			skill({ id: "c", name: "alpha", status: "active" }),
			skill({ id: "d", name: "omega", status: "pending" }),
		]);
		expect(sorted.map((s) => s.id)).toEqual(["d", "c", "b", "a"]);
	});
});

describe("draftProblem", () => {
	const draft = { name: "weekly-report", description: "d", body: "b" };

	test("names follow the agentskills rules", () => {
		expect(draftProblem(draft, false)).toBeNull();
		for (const name of ["Weekly", "-a", "a-", "a--b", "a b", "a".repeat(65)]) {
			expect(draftProblem({ ...draft, name }, false)).toBe("name");
		}
		expect(draftProblem({ ...draft, name: "Anything" }, true)).toBeNull();
	});

	test("description is one line and the sizes are capped", () => {
		expect(draftProblem({ ...draft, description: "a\nb" }, false)).toBe(
			"description",
		);
		expect(
			draftProblem({ ...draft, description: "x".repeat(1025) }, false),
		).toBe("length");
		expect(
			draftProblem({ ...draft, body: "x".repeat(32 * 1024 + 1) }, false),
		).toBe("length");
	});
});

describe("skill rows", () => {
	test("a package skill can't be edited, only an active one needs no approval", () => {
		expect(canEdit(skill({ source: "package:pkg@1.0.0" }))).toBe(false);
		expect(canEdit(skill({ source: "bot_proposed" }))).toBe(true);
		expect(canApprove(skill({ status: "active" }))).toBe(false);
		expect(canApprove(skill({ status: "pending" }))).toBe(true);
		expect(canApprove(skill({ status: "disabled" }))).toBe(true);
		expect(sourceKind("import")).toBe("import");
		expect(sourceKind("bot_proposed")).toBe("bot");
	});
});
