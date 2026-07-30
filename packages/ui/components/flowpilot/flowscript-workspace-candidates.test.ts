import { describe, expect, test } from "bun:test";
import {
	flowScriptWorkspaceDiagnostics,
	flowScriptWorkspaceRepairResolved,
	parseFlowScriptWorkspaceCandidate,
} from "./flowscript-workspace-candidates";

describe("FlowScript workspace diagnostics", () => {
	test("retains bounded structured diagnostics on the exact candidate", () => {
		const candidate = parseFlowScriptWorkspaceCandidate(
			JSON.stringify({
				source: "eventsSimple() {}",
				status: "validation_errors",
				diagnostics: [{ code: "LEGACY", message: "legacy" }],
				structured_diagnostics: Array.from({ length: 30 }, (_, index) => ({
					code: `FS-${index}`,
					message: "x".repeat(2_000),
					source_span: { start: 4, end: 12 },
					expected: "Struct",
					actual: "String",
					fix: "Use structMake before this call.",
					related_messages: ["Declaration returned a Struct input."],
					source: "source text is intentionally omitted",
				})),
			}),
		);

		if (!candidate) throw new Error("Expected a parsed workspace candidate.");
		const structuredDiagnostics = candidate.structured_diagnostics;
		if (!structuredDiagnostics) {
			throw new Error("Expected bounded structured diagnostics.");
		}
		expect(structuredDiagnostics).toHaveLength(20);
		expect(
			String((structuredDiagnostics[0] as Record<string, unknown>)?.message)
				.length,
		).toBe(600);
		expect(structuredDiagnostics[0]).not.toHaveProperty("source");
		expect(structuredDiagnostics[0]).toMatchObject({
			expected: "Struct",
			actual: "String",
			fix: "Use structMake before this call.",
		});
		expect(flowScriptWorkspaceDiagnostics(candidate)).toBe(
			structuredDiagnostics,
		);
	});

	test("falls back to legacy diagnostics and recognises repaired candidates", () => {
		const candidate = parseFlowScriptWorkspaceCandidate(
			JSON.stringify({
				source: "eventsSimple() {}",
				status: "queued",
				diagnostics: ["legacy diagnostic"],
			}),
		);

		if (!candidate) throw new Error("Expected a parsed workspace candidate.");
		expect(flowScriptWorkspaceDiagnostics(candidate)).toEqual([
			"legacy diagnostic",
		]);
		expect(flowScriptWorkspaceRepairResolved(candidate)).toBe(true);
		expect(
			flowScriptWorkspaceRepairResolved({
				source: "eventsSimple() {}",
				status: "validation_errors",
			}),
		).toBe(false);
	});
});
