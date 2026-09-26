import { describe, expect, test } from "bun:test";
import type {
	ILogGroup,
	IRunLogSummary,
} from "../../../lib/schema/flow/log-query";
import {
	FOLD_LIMIT,
	foldHead,
	groupLabel,
	groupsByFingerprint,
	planFold,
	slotLabel,
	templateParts,
} from "./fold-model";
import { makeLog } from "./test-fixtures";

function group(
	fingerprint: string,
	count: number,
	first_start = 100,
): ILogGroup {
	return {
		fingerprint,
		log_level: 3,
		template: `t ${fingerprint}`,
		slots: [],
		count,
		first_start,
		last_start: first_start + count,
		sample: "",
	};
}

function summary(groups: ILogGroup[], fingerprinted = true): IRunLogSummary {
	return {
		version: 1,
		fingerprinted,
		partial: false,
		total: groups.reduce((sum, g) => sum + g.count, 0),
		levels: [0, 0, 0, 0, 0],
		nodes: {},
		groups,
		groups_truncated: false,
	};
}

const OPTIONS = {
	enabled: true,
	unfolded: new Set<string>(),
	textActive: false,
};

describe("planFold", () => {
	test("folds repeated groups, most frequent first", () => {
		const plan = planFold(
			summary([group("a", 3), group("b", 986, 50), group("c", 1)]),
			OPTIONS,
		);
		expect(plan).toEqual({
			fold: [
				{ fingerprint: "b", first_start: 50 },
				{ fingerprint: "a", first_start: 100 },
			],
			paused: false,
		});
	});

	test("caps the list and skips unfolded and scoped groups", () => {
		const many = Array.from({ length: FOLD_LIMIT + 10 }, (_, i) =>
			group(`g${i}`, 1000 - i),
		);
		expect(planFold(summary(many), OPTIONS).fold).toHaveLength(FOLD_LIMIT);
		const plan = planFold(
			summary([group("a", 5), group("b", 4), group("c", 3)]),
			{
				...OPTIONS,
				unfolded: new Set(["a"]),
				onlyGroups: ["b"],
			},
		);
		expect(plan.fold.map((f) => f.fingerprint)).toEqual(["c"]);
	});

	test("is empty when off, unfingerprinted or searching", () => {
		const groups = [group("a", 5)];
		expect(planFold(summary(groups), { ...OPTIONS, enabled: false })).toEqual({
			fold: [],
			paused: false,
		});
		expect(planFold(summary(groups, false), OPTIONS)).toEqual({
			fold: [],
			paused: false,
		});
		expect(planFold(null, OPTIONS)).toEqual({ fold: [], paused: false });
		expect(planFold(summary(groups), { ...OPTIONS, textActive: true })).toEqual(
			{
				fold: [],
				paused: true,
			},
		);
	});
});

describe("foldHead", () => {
	test("only the first occurrence of a folded group is a head", () => {
		const groups = groupsByFingerprint(summary([group("fp", 986, 5_000_000)]));
		const folded = new Set(["fp"]);
		const head = makeLog({ fingerprint: "fp", start: 5_000_000 });
		expect(foldHead(head, folded, groups)?.count).toBe(986);
		expect(
			foldHead(
				makeLog({ fingerprint: "fp", start: 5_000_001 }),
				folded,
				groups,
			),
		).toBeUndefined();
		expect(foldHead(head, new Set(), groups)).toBeUndefined();
		expect(
			foldHead(makeLog({ start: 5_000_000 }), folded, groups),
		).toBeUndefined();
	});
});

describe("templates", () => {
	test("labels number slots with their range and keeps ids generic", () => {
		expect(
			templateParts(
				'Error: ExecutionFailed("x") in iteration ⟨n⟩ of ⟨n⟩ [⟨id⟩]',
				[
					{ min: 0, max: 985 },
					{ min: 986, max: 986 },
				],
			),
		).toEqual([
			{ kind: "text", text: 'Error: ExecutionFailed("x") in iteration ' },
			{ kind: "slot", label: "⟨0…985⟩" },
			{ kind: "text", text: " of " },
			{ kind: "slot", label: "⟨986⟩" },
			{ kind: "text", text: " [" },
			{ kind: "slot", label: "⟨id⟩" },
			{ kind: "text", text: "]" },
		]);
		expect(slotLabel(null)).toBe("⟨n⟩");
	});

	test("group labels are one short line", () => {
		expect(groupLabel("short")).toBe("short");
		expect(groupLabel("a\nb")).toBe("a");
		expect(groupLabel("x".repeat(50), 10)).toBe(`${"x".repeat(9)}…`);
	});
});
