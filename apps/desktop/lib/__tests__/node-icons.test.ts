import { describe, expect, test } from "vitest";
import {
	checkNodeIcons,
	formatNodeIconIssues,
} from "../../../../scripts/sync-node-icons";

describe("node icon assets", () => {
	test("cover every catalog /flow/icons reference in public folders", () => {
		const result = checkNodeIcons();

		expect(result.issues, formatNodeIconIssues(result.issues)).toEqual([]);
	});
});
