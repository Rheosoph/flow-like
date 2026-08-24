import { beforeAll, describe, expect, test } from "bun:test";
import { loadFlowScriptNamesTable } from "../../lib/flowscript/names";
import {
	flowScriptWorkspaceDiagnostics,
	flowScriptWorkspaceRepairResolved,
	normalizeCallName,
	parseFlowScriptWorkspaceCandidate,
	profileFlowScriptCandidate,
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

describe("FlowScript candidate profiling across call spellings", () => {
	beforeAll(async () => {
		await loadFlowScriptNamesTable();
	});

	test("normalises flat, qualified and method spellings to one node id", () => {
		expect(normalizeCallName("logInfo", false)).toBe("log_info");
		expect(normalizeCallName("log::info", false)).toBe("log_info");
		expect(normalizeCallName("string :: format", false)).toBe("string_format");
		expect(normalizeCallName("trim", true)).toBe("string_trim");
		expect(normalizeCallName("format", true)).toBe(".format");
		expect(normalizeCallName("shout", true, new Set(["shout"]))).toBe("shout");
		expect(normalizeCallName("myHelper", false)).toBe("myhelper");
	});

	test("profiles qualified and method call sites like their legacy spellings", () => {
		const legacy = profileFlowScriptCandidate(`function enrich(item: Struct) {
	const text = stringTrim({ string: item })
	const hash = utilsHashMd5({ input: text })
	logInfo({ message: hash })
}
eventsSimple onLoad() {
	enrich({ item: payload })
}`);
		const sugared = profileFlowScriptCandidate(`use string::*

function enrich(item: Struct) {
	const text = \`\${item}\`.trim()
	const hash = hash::md5({ input: text })
	log::info(hash)
}
eventsSimple onLoad() {
	payload.enrich()
}`);
		expect(sugared.callSites).toBe(legacy.callSites);
		expect(sugared.callNames).toEqual(legacy.callNames);
		expect(sugared.helperDomainCallSites).toBe(legacy.helperDomainCallSites);
		expect(sugared.eventsCallingHelpers).toBe(1);
		expect(sugared.eventEntries).toBe(1);
		expect(legacy.callNames).toContain("utils_hash_md5");
		expect(legacy.callNames).not.toContain("events_simple");
	});
});
