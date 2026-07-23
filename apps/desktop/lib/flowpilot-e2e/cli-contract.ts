import type {
	FlowPilotE2EArtifact,
	FlowPilotE2ECaseId,
	FlowPilotE2ECliEnvelope,
} from "./types";

export interface FlowPilotE2ECliExpectation {
	runId: string;
	caseIds: readonly FlowPilotE2ECaseId[];
	repeat: number;
	minFlowScriptNonWhitespaceChars?: number;
	failFast: boolean;
}

function isRecord(value: unknown): value is Record<string, unknown> {
	return Boolean(value) && typeof value === "object" && !Array.isArray(value);
}

function isNonNegativeInteger(value: unknown): value is number {
	return Number.isSafeInteger(value) && Number(value) >= 0;
}

function isArtifact(value: unknown): value is FlowPilotE2EArtifact {
	if (!isRecord(value)) return false;
	if (
		value.schema !== "flowpilot.app-creation-e2e-artifact/v1" ||
		typeof value.caseId !== "string" ||
		typeof value.expectedAppName !== "string" ||
		typeof value.prompt !== "string" ||
		typeof value.generatedAt !== "string" ||
		typeof value.durationMs !== "number" ||
		!isRecord(value.runner) ||
		!Array.isArray(value.runner.suppressedNavigations) ||
		!Array.isArray(value.runner.issues)
	) {
		return false;
	}
	if (value.error !== undefined && typeof value.error !== "string")
		return false;
	if (value.report === undefined) return true;
	return (
		isRecord(value.report) &&
		value.report.schema === "flowpilot.app-creation-e2e-report/v1" &&
		typeof value.report.passed === "boolean" &&
		Array.isArray(value.report.checks) &&
		Array.isArray(value.report.failures)
	);
}

/**
 * Structural check performed before the controller trusts or prints a webview callback.
 * The nonce authenticates the sender; this guard catches stale runner/controller contracts.
 */
export function isFlowPilotE2ECliEnvelope(
	value: unknown,
	runId: string,
): value is FlowPilotE2ECliEnvelope {
	if (!isRecord(value)) return false;
	if (
		value.schema !== "flowpilot.app-creation-e2e-cli-result/v1" ||
		value.runId !== runId ||
		typeof value.startedAt !== "string" ||
		typeof value.completedAt !== "string" ||
		typeof value.durationMs !== "number" ||
		typeof value.passed !== "boolean" ||
		!isRecord(value.selection) ||
		!Array.isArray(value.selection.caseIds) ||
		!value.selection.caseIds.every((caseId) => typeof caseId === "string") ||
		!isNonNegativeInteger(value.selection.repeat) ||
		typeof value.selection.failFast !== "boolean" ||
		!Array.isArray(value.artifacts) ||
		!value.artifacts.every(isArtifact) ||
		!isRecord(value.summary) ||
		!isNonNegativeInteger(value.summary.requestedRuns) ||
		!isNonNegativeInteger(value.summary.completedRuns) ||
		!isNonNegativeInteger(value.summary.passed) ||
		!isNonNegativeInteger(value.summary.failed) ||
		!isNonNegativeInteger(value.summary.skipped)
	) {
		return false;
	}
	if (
		value.selection.minFlowScriptNonWhitespaceChars !== undefined &&
		!isNonNegativeInteger(value.selection.minFlowScriptNonWhitespaceChars)
	) {
		return false;
	}
	return value.error === undefined || typeof value.error === "string";
}

export function flowPilotE2EArtifactPassed(
	artifact: FlowPilotE2EArtifact,
): boolean {
	return !artifact.error && artifact.report?.passed === true;
}

function sameCaseIds(
	actual: readonly FlowPilotE2ECaseId[],
	expected: readonly FlowPilotE2ECaseId[],
): boolean {
	return (
		actual.length === expected.length &&
		actual.every((caseId, index) => caseId === expected[index])
	);
}

/** Revalidates selection/order and derives every acceptance field on the CLI side. */
export function normalizeFlowPilotE2ECliEnvelope(
	envelope: FlowPilotE2ECliEnvelope,
	expected: FlowPilotE2ECliExpectation,
): FlowPilotE2ECliEnvelope {
	if (envelope.runId !== expected.runId) {
		throw new Error(
			"FlowPilot E2E callback run id does not match the controller run.",
		);
	}
	if (
		!sameCaseIds(envelope.selection.caseIds, expected.caseIds) ||
		envelope.selection.repeat !== expected.repeat ||
		envelope.selection.minFlowScriptNonWhitespaceChars !==
			expected.minFlowScriptNonWhitespaceChars ||
		envelope.selection.failFast !== expected.failFast
	) {
		throw new Error(
			"FlowPilot E2E callback selection does not match the request.",
		);
	}

	const expectedOrder = Array.from({ length: expected.repeat }, () => [
		...expected.caseIds,
	]).flat();
	if (envelope.artifacts.length > expectedOrder.length) {
		throw new Error(
			"FlowPilot E2E callback returned more artifacts than requested.",
		);
	}
	for (const [index, artifact] of envelope.artifacts.entries()) {
		if (artifact.caseId !== expectedOrder[index]) {
			throw new Error(
				`FlowPilot E2E artifact ${index + 1} is for ${artifact.caseId}; expected ${expectedOrder[index]}.`,
			);
		}
	}

	const error = envelope.error?.trim() || undefined;
	const passedRuns = envelope.artifacts.filter(
		flowPilotE2EArtifactPassed,
	).length;
	const requestedRuns = expectedOrder.length;
	const firstFailedArtifact = envelope.artifacts.findIndex(
		(artifact) => !flowPilotE2EArtifactPassed(artifact),
	);
	if (
		expected.failFast &&
		firstFailedArtifact >= 0 &&
		firstFailedArtifact !== envelope.artifacts.length - 1
	) {
		throw new Error(
			"FlowPilot E2E callback returned artifacts after a fail-fast failure.",
		);
	}
	const shortRun = envelope.artifacts.length < requestedRuns;
	if (shortRun && !error) {
		const lastArtifact = envelope.artifacts.at(-1);
		if (
			!expected.failFast ||
			!lastArtifact ||
			flowPilotE2EArtifactPassed(lastArtifact)
		) {
			throw new Error(
				"FlowPilot E2E callback ended before all requested runs completed.",
			);
		}
	}

	const passed =
		!error &&
		requestedRuns > 0 &&
		envelope.artifacts.length === requestedRuns &&
		passedRuns === requestedRuns;
	return {
		...envelope,
		selection: {
			caseIds: [...expected.caseIds],
			repeat: expected.repeat,
			minFlowScriptNonWhitespaceChars: expected.minFlowScriptNonWhitespaceChars,
			failFast: expected.failFast,
		},
		passed,
		summary: {
			requestedRuns,
			completedRuns: envelope.artifacts.length,
			passed: passedRuns,
			failed: envelope.artifacts.length - passedRuns,
			skipped: requestedRuns - envelope.artifacts.length,
		},
		error,
	};
}

export function flowPilotE2ECliExitCode(
	envelope: FlowPilotE2ECliEnvelope,
): 0 | 1 | 2 {
	if (envelope.error) return 2;
	return envelope.passed ? 0 : 1;
}
