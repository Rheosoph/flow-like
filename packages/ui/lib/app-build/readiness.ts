import { compileAppSpec, type AppSpecIssue } from "./compiler";
import { appBuildFingerprint } from "./fingerprint";
import {
	appBuildStateSchema,
	type AppBuildResourceState,
	type AppBuildState,
	type BuildReadiness,
	type BuildReadinessBlocker,
	type BuildStateIssue,
	type BuildValidation,
} from "./state-contract";

function latestAppliedReceipt(state: AppBuildResourceState) {
	return [...state.receipts]
		.reverse()
		.find((receipt) => receipt.status === "applied");
}

/** Validate both the persisted shape and every derived binding/evidence invariant. */
export function validateBuild(input: unknown): BuildValidation {
	const parsed = appBuildStateSchema.safeParse(input);
	if (!parsed.success) {
		return {
			ok: false,
			issues: parsed.error.issues.map((entry) => ({
				code: "invalid_state_schema",
				path: entry.path.map((key) =>
					typeof key === "symbol" ? String(key) : key,
				),
				message: entry.message,
			})),
		};
	}
	const build = parsed.data;
	let compiled;
	try {
		compiled = compileAppSpec(build.spec, {
			app_id: build.app_id,
			build_id: build.build_id,
		});
	} catch (error) {
		const issues =
			error && typeof error === "object" && "issues" in error
				? (error.issues as readonly AppSpecIssue[]).map((entry) => ({
						code: entry.code,
						path: ["spec", ...entry.path],
						message: entry.message,
					}))
				: [{ code: "invalid_spec", path: ["spec"], message: String(error) }];
		return { ok: false, issues };
	}
	const issues: BuildStateIssue[] = [];
	if (build.spec_fingerprint !== compiled.spec_fingerprint) {
		issues.push({
			code: "spec_fingerprint_mismatch",
			path: ["spec_fingerprint"],
			message: "Stored spec_fingerprint does not match the canonical AppSpec.",
		});
	}
	if (
		build.original_request &&
		build.original_request_fingerprint !==
			appBuildFingerprint("original-request", build.original_request)
	) {
		issues.push({
			code: "original_request_fingerprint_mismatch",
			path: ["original_request_fingerprint"],
			message:
				"Stored original_request_fingerprint does not match the immutable request.",
		});
	}
	const compiledKeys = new Set(
		compiled.resources.map((resource) => resource.key),
	);
	for (const key of Object.keys(build.resources)) {
		if (!compiledKeys.has(key)) {
			issues.push({
				code: "unexpected_resource_state",
				path: ["resources", key],
				message: `Resource state '${key}' is absent from AppSpec.`,
			});
		}
	}
	for (const resource of compiled.resources) {
		const state = build.resources[resource.key];
		if (!state) {
			issues.push({
				code: "missing_resource_state",
				path: ["resources", resource.key],
				message: `Resource '${resource.key}' has no build state.`,
			});
			continue;
		}
		if (
			state.key !== resource.key ||
			state.kind !== resource.kind ||
			state.physical_id !== resource.physical_id ||
			state.desired_fingerprint !== resource.desired_fingerprint
		) {
			issues.push({
				code: "resource_binding_mismatch",
				path: ["resources", resource.key],
				message: `Resource '${resource.key}' does not match its compiled reservation.`,
			});
		}
		if (
			state.status === "applying" &&
			resource.depends_on.some(
				(dependency) => build.resources[dependency]?.status !== "applied",
			)
		) {
			issues.push({
				code: "applying_dependency_not_applied",
				path: ["resources", resource.key, "status"],
				message: `Applying resource '${resource.key}' has a dependency that is not applied.`,
			});
		}
		if (state.status === "applied") {
			const receipt = latestAppliedReceipt(state);
			if (
				receipt?.status !== "applied" ||
				receipt.physical_id !== state.physical_id ||
				receipt.desired_fingerprint !== state.desired_fingerprint ||
				!receipt.observed_fingerprint
			) {
				issues.push({
					code: "applied_receipt_missing",
					path: ["resources", resource.key, "receipts"],
					message: `Applied resource '${resource.key}' lacks an exact authoritative receipt.`,
				});
			}
		}
	}
	const hasCompleteResourceSnapshot =
		Object.values(build.resources).every(
			(resource) => resource.status === "applied",
		) &&
		!Object.values(build.retired_resources).some(
			(resource) => resource.cleanup_status === "required",
		);
	if (build.validation_revision !== undefined && !hasCompleteResourceSnapshot) {
		issues.push({
			code: "invalid_validation_snapshot",
			path: ["validation_revision"],
			message:
				"validation_revision requires every planned resource applied and every retired resource resolved.",
		});
	}
	if (build.structural_evidence?.status === "passed") {
		for (const resource of compiled.resources) {
			const state = build.resources[resource.key];
			const observed = state
				? latestAppliedReceipt(state)?.observed_fingerprint
				: undefined;
			if (
				!observed ||
				build.structural_evidence.resource_fingerprints[resource.key] !==
					observed
			) {
				issues.push({
					code: "structural_evidence_resource_mismatch",
					path: ["structural_evidence", "resource_fingerprints", resource.key],
					message: `Structural evidence does not match the latest observed artifact for '${resource.key}'.`,
				});
			}
		}
	}
	const compiledScenarioIds = new Set(
		compiled.scenarios.map((scenario) => scenario.id),
	);
	for (const id of Object.keys(build.scenarios)) {
		if (!compiledScenarioIds.has(id)) {
			issues.push({
				code: "unexpected_scenario_state",
				path: ["scenarios", id],
				message: `Scenario state '${id}' is absent from AppSpec.`,
			});
		}
	}
	for (const scenario of compiled.scenarios) {
		const state = build.scenarios[scenario.id];
		if (
			!state ||
			state.id !== scenario.id ||
			state.desired_fingerprint !== scenario.desired_fingerprint
		) {
			issues.push({
				code: "scenario_binding_mismatch",
				path: ["scenarios", scenario.id],
				message: `Scenario '${scenario.id}' does not match its compiled contract.`,
			});
			continue;
		}
		if (state.evidence) {
			if (
				state.status !== state.evidence.status ||
				state.evidence.scenario_id !== scenario.id ||
				state.evidence.scenario_fingerprint !== scenario.desired_fingerprint ||
				state.evidence.spec_fingerprint !== build.spec_fingerprint ||
				state.evidence.build_revision !== build.validation_revision
			) {
				issues.push({
					code: "scenario_evidence_mismatch",
					path: ["scenarios", scenario.id, "evidence"],
					message: `Scenario '${scenario.id}' evidence does not match its exact validation snapshot.`,
				});
			}
		} else if (state.status !== "pending") {
			issues.push({
				code: "scenario_evidence_missing",
				path: ["scenarios", scenario.id, "evidence"],
				message: `Scenario '${scenario.id}' cannot be ${state.status} without host evidence.`,
			});
		}
	}
	if (
		build.structural_evidence &&
		(build.structural_evidence.spec_fingerprint !== build.spec_fingerprint ||
			build.structural_evidence.build_revision !== build.validation_revision)
	) {
		issues.push({
			code: "structural_evidence_snapshot_mismatch",
			path: ["structural_evidence"],
			message:
				"Structural evidence does not match the exact validation snapshot.",
		});
	}
	if (
		build.promotion.status === "promoted" ||
		build.promotion.status === "partial" ||
		build.promotion.status === "failed" ||
		(build.promotion.status === "unknown" && build.promotion.receipt)
	) {
		const receipt = build.promotion.receipt;
		const expectedReceiptStatus =
			build.promotion.status === "promoted"
				? "applied"
				: build.promotion.status;
		if (
			!receipt ||
			receipt.status !== expectedReceiptStatus ||
			receipt.build_revision !== build.promotion.requested_build_revision ||
			receipt.spec_fingerprint !== build.spec_fingerprint
		) {
			issues.push({
				code: "promotion_receipt_mismatch",
				path: ["promotion", "receipt"],
				message: "Promotion state lacks an exact matching host receipt.",
			});
		}
	}
	if (
		build.validation_revision !== undefined &&
		build.validation_revision > build.revision
	) {
		issues.push({
			code: "future_validation_revision",
			path: ["validation_revision"],
			message: "validation_revision cannot be newer than the state revision.",
		});
	}
	if (build.updated_at_ms < build.created_at_ms) {
		issues.push({
			code: "invalid_timestamps",
			path: ["updated_at_ms"],
			message: "updated_at_ms cannot precede created_at_ms.",
		});
	}
	return issues.length > 0
		? { ok: false, issues }
		: { ok: true, build, compiled };
}

/** Derive preview, behavioral, and promotion readiness solely from validated host evidence. */
export function readiness(build: AppBuildState): BuildReadiness {
	const validation = validateBuild(build);
	if (!validation.ok) {
		return {
			level: "invalid",
			structural_preview: false,
			behaviorally_ready: false,
			can_promote: false,
			blockers: validation.issues.map((entry) => ({
				code: "invalid_build",
				message: entry.message,
			})),
		};
	}
	const blockers: BuildReadinessBlocker[] = [];
	for (const resource of Object.values(build.resources)) {
		if (resource.status !== "applied") {
			blockers.push({
				code: "resource_not_applied",
				resource_key: resource.key,
				message: `Resource '${resource.key}' is ${resource.status}.`,
			});
		}
	}
	for (const retired of Object.values(build.retired_resources)) {
		if (retired.cleanup_status === "required") {
			blockers.push({
				code: "retired_resource_cleanup_required",
				resource_key: retired.key,
				message: `Retired ${retired.kind} '${retired.key}' still requires cleanup or an explicit retain decision.`,
			});
		}
	}
	if (build.validation_revision === undefined) {
		blockers.push({
			code: "validation_snapshot_missing",
			message: "No complete resource revision is available for validation.",
		});
	}
	const structural = build.structural_evidence;
	let structuralPreview = false;
	if (!structural) {
		blockers.push({
			code: "structural_evidence_missing",
			message:
				"Host structural validation has not passed for this build revision.",
		});
	} else if (
		structural.build_revision !== build.validation_revision ||
		structural.spec_fingerprint !== build.spec_fingerprint
	) {
		blockers.push({
			code: "structural_evidence_stale",
			message: "Structural evidence belongs to another build revision or spec.",
		});
	} else if (structural.status !== "passed") {
		blockers.push({
			code: "structural_evidence_failed",
			message: "Host structural validation failed.",
		});
	} else {
		structuralPreview = true;
	}

	if (validation.compiled.scenarios.length === 0) {
		blockers.push({
			code: "scenario_contract_missing",
			message:
				"The AppSpec has no behavioral scenario. Structural evidence supports preview only.",
		});
	}
	let allScenariosPassed = validation.compiled.scenarios.length > 0;
	for (const scenario of validation.compiled.scenarios) {
		const state = build.scenarios[scenario.id];
		if (!state?.evidence) {
			allScenariosPassed = false;
			blockers.push({
				code: "scenario_evidence_missing",
				scenario_id: scenario.id,
				message: `Scenario '${scenario.id}' has no host-issued evidence.`,
			});
			continue;
		}
		if (
			state.evidence.build_revision !== build.validation_revision ||
			state.evidence.spec_fingerprint !== build.spec_fingerprint ||
			state.evidence.scenario_fingerprint !== scenario.desired_fingerprint
		) {
			allScenariosPassed = false;
			blockers.push({
				code: "scenario_evidence_stale",
				scenario_id: scenario.id,
				message: `Scenario '${scenario.id}' evidence is stale.`,
			});
			continue;
		}
		if (
			state.evidence.status === "passed" &&
			state.evidence.certification !== "behavioral"
		) {
			allScenariosPassed = false;
			blockers.push({
				code: "scenario_contract_only",
				scenario_id: scenario.id,
				message: `Scenario '${scenario.id}' passed only a contract/readback check, not isolated runtime behavior.`,
			});
		} else if (state.evidence.status === "blocked") {
			allScenariosPassed = false;
			blockers.push({
				code: "scenario_blocked",
				scenario_id: scenario.id,
				message:
					state.evidence.message ?? `Scenario '${scenario.id}' was blocked.`,
			});
		} else if (state.evidence.status !== "passed") {
			allScenariosPassed = false;
			blockers.push({
				code: "scenario_failed",
				scenario_id: scenario.id,
				message: state.evidence.message ?? `Scenario '${scenario.id}' failed.`,
			});
		}
	}
	for (const requirement of validation.compiled.requirement_coverage) {
		const hasBehavioralEvidence = requirement.scenario_ids.some(
			(scenarioId) => {
				const evidence = build.scenarios[scenarioId]?.evidence;
				return (
					evidence?.status === "passed" &&
					evidence.certification === "behavioral" &&
					evidence.build_revision === build.validation_revision &&
					evidence.spec_fingerprint === build.spec_fingerprint
				);
			},
		);
		if (!hasBehavioralEvidence) {
			allScenariosPassed = false;
			blockers.push({
				code: "requirement_behavioral_evidence_missing",
				requirement_id: requirement.requirement_id,
				message: `Requirement '${requirement.requirement_id}' has no passed isolated behavioral scenario.`,
			});
		}
	}
	if (
		build.promotion.status === "partial" ||
		build.promotion.status === "unknown" ||
		build.promotion.status === "promoting"
	) {
		blockers.push({
			code: "promotion_in_doubt",
			message: `Promotion is ${build.promotion.status}; reconcile it before another attempt.`,
		});
	}
	const resourcesApplied = Object.values(build.resources).every(
		(resource) => resource.status === "applied",
	);
	const behaviorallyReady = structuralPreview && allScenariosPassed;
	const promoted = build.promotion.status === "promoted";
	const canPromote =
		behaviorallyReady &&
		!promoted &&
		(build.promotion.status === "staged" ||
			build.promotion.status === "failed") &&
		!Object.values(build.retired_resources).some(
			(resource) => resource.cleanup_status === "required",
		);
	let level: BuildReadiness["level"] = "building";
	if (promoted) level = "promoted";
	else if (behaviorallyReady) level = "ready";
	else if (structuralPreview) level = "structural_preview";
	else if (resourcesApplied) level = "awaiting_validation";
	return {
		level,
		structural_preview: structuralPreview,
		behaviorally_ready: behaviorallyReady,
		can_promote: canPromote,
		validation_revision: build.validation_revision,
		blockers,
	};
}
