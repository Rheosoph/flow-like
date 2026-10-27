import { z } from "zod";

import {
	appSpecSchema,
	jsonObjectSchema,
	logicalKeySchema,
	serializedJsonByteLength,
} from "./contract";
import type { CompiledAppSpec } from "./compiler";

export const APP_BUILD_STATE_SCHEMA_VERSION = 1 as const;
/** Kept below the Rust store's 1 MiB limit to absorb serializer differences. */
export const MAX_APP_BUILD_STATE_BYTES = 1_000_000;

export const buildResourceStatusSchema = z.enum([
	"pending",
	"applying",
	"applied",
	"partial",
	"unknown",
	"failed",
]);

export type BuildResourceStatus = z.infer<typeof buildResourceStatusSchema>;

export const resourceReceiptStatusSchema = z.enum([
	"applied",
	"partial",
	"unknown",
	"failed",
]);

export const appBuildResourceReceiptSchema = z
	.object({
		receipt_id: z.string().trim().min(1).max(256),
		operation_id: z.string().trim().min(1).max(256),
		status: resourceReceiptStatusSchema,
		physical_id: z.string().trim().min(1).max(256),
		desired_fingerprint: z.string().trim().min(1).max(128),
		observed_fingerprint: z.string().trim().min(1).max(128).optional(),
		resource_revision: z.string().trim().min(1).max(256).optional(),
		recorded_at_ms: z.number().int().nonnegative(),
		message: z.string().trim().min(1).max(4_000).optional(),
		details: jsonObjectSchema.optional(),
	})
	.strict();

export type AppBuildResourceReceipt = z.infer<
	typeof appBuildResourceReceiptSchema
>;

export const appBuildResourceInspectionSchema = z
	.object({
		inspection_id: z.string().trim().min(1).max(256),
		status: z.enum(["matches", "missing", "drifted", "unknown"]),
		physical_id: z.string().trim().min(1).max(256),
		desired_fingerprint: z.string().trim().min(1).max(128),
		observed_fingerprint: z.string().trim().min(1).max(128).optional(),
		inspected_at_ms: z.number().int().nonnegative(),
		resource_revision: z.string().trim().min(1).max(256).optional(),
		message: z.string().trim().min(1).max(4_000).optional(),
		details: jsonObjectSchema.optional(),
	})
	.strict();

export type AppBuildResourceInspection = z.infer<
	typeof appBuildResourceInspectionSchema
>;

export const appBuildOperationLeaseSchema = z
	.object({
		operation_id: z.string().trim().min(1).max(256),
		owner_id: z.string().trim().min(1).max(256),
		started_at_ms: z.number().int().nonnegative(),
		expires_at_ms: z.number().int().positive(),
	})
	.strict()
	.refine((lease) => lease.expires_at_ms > lease.started_at_ms, {
		message: "expires_at_ms must be later than started_at_ms",
		path: ["expires_at_ms"],
	});

export type AppBuildOperationLease = z.infer<
	typeof appBuildOperationLeaseSchema
>;

export const appBuildResourceStateSchema = z
	.object({
		key: logicalKeySchema,
		kind: z.enum(["board", "page", "widget", "table", "event"]),
		physical_id: z.string().trim().min(1).max(256),
		desired_fingerprint: z.string().trim().min(1).max(128),
		status: buildResourceStatusSchema,
		attempts: z.number().int().nonnegative(),
		receipts: z.array(appBuildResourceReceiptSchema).max(32),
		last_inspection: appBuildResourceInspectionSchema.optional(),
		active_operation: appBuildOperationLeaseSchema.optional(),
		last_error: z.string().trim().min(1).max(4_000).optional(),
	})
	.strict()
	.superRefine((resource, context) => {
		if ((resource.status === "applying") !== !!resource.active_operation) {
			context.addIssue({
				code: z.ZodIssueCode.custom,
				message: "Only applying resources must hold an active operation lease.",
				path: ["active_operation"],
			});
		}
	});

export type AppBuildResourceState = z.infer<typeof appBuildResourceStateSchema>;

export const retiredResourceStateSchema = z
	.object({
		retirement_id: z.string().trim().min(1).max(256),
		key: logicalKeySchema,
		kind: z.enum(["board", "page", "widget", "table", "event"]),
		physical_id: z.string().trim().min(1).max(256),
		desired_fingerprint: z.string().trim().min(1).max(128),
		cleanup_status: z.enum(["required", "cleaned", "retained"]),
		retired_at_revision: z.number().int().nonnegative(),
		last_receipt: appBuildResourceReceiptSchema.optional(),
		cleanup_receipt: appBuildResourceReceiptSchema.optional(),
	})
	.strict();

export type RetiredResourceState = z.infer<typeof retiredResourceStateSchema>;

export const structuralEvidenceSchema = z
	.object({
		evidence_id: z.string().trim().min(1).max(256),
		status: z.enum(["passed", "failed"]),
		build_revision: z.number().int().nonnegative(),
		spec_fingerprint: z.string().trim().min(1).max(128),
		resource_fingerprints: z.record(z.string(), z.string().trim().min(1).max(128)),
		recorded_at_ms: z.number().int().nonnegative(),
		issues: z.array(z.string().trim().min(1).max(2_000)).max(128),
	})
	.strict();

export type StructuralEvidence = z.infer<typeof structuralEvidenceSchema>;

export const scenarioEvidenceSchema = z
	.object({
		evidence_id: z.string().trim().min(1).max(256),
		scenario_id: logicalKeySchema,
		status: z.enum(["passed", "failed", "blocked"]),
		certification: z.enum(["none", "contract_only", "behavioral"]),
		build_revision: z.number().int().nonnegative(),
		spec_fingerprint: z.string().trim().min(1).max(128),
		scenario_fingerprint: z.string().trim().min(1).max(128),
		recorded_at_ms: z.number().int().nonnegative(),
		message: z.string().trim().min(1).max(4_000).optional(),
		details: jsonObjectSchema.optional(),
	})
	.strict();

export type ScenarioEvidence = z.infer<typeof scenarioEvidenceSchema>;

export const appBuildScenarioStateSchema = z
	.object({
		id: logicalKeySchema,
		desired_fingerprint: z.string().trim().min(1).max(128),
		status: z.enum(["pending", "passed", "failed", "blocked"]),
		evidence: scenarioEvidenceSchema.optional(),
	})
	.strict();

export type AppBuildScenarioState = z.infer<typeof appBuildScenarioStateSchema>;

export const promotionReceiptSchema = z
	.object({
		receipt_id: z.string().trim().min(1).max(256),
		operation_id: z.string().trim().min(1).max(256),
		status: z.enum(["applied", "partial", "unknown", "failed"]),
		build_revision: z.number().int().nonnegative(),
		spec_fingerprint: z.string().trim().min(1).max(128),
		recorded_at_ms: z.number().int().nonnegative(),
		message: z.string().trim().min(1).max(4_000).optional(),
		details: jsonObjectSchema.optional(),
	})
	.strict();

export type PromotionReceipt = z.infer<typeof promotionReceiptSchema>;

export const promotionStateSchema = z
	.object({
		status: z.enum([
			"staged",
			"promoting",
			"promoted",
			"partial",
			"unknown",
			"failed",
		]),
		attempts: z.number().int().nonnegative(),
		requested_build_revision: z.number().int().nonnegative().optional(),
		active_operation: appBuildOperationLeaseSchema.optional(),
		receipt: promotionReceiptSchema.optional(),
	})
	.strict()
	.superRefine((promotion, context) => {
		if ((promotion.status === "promoting") !== !!promotion.active_operation) {
			context.addIssue({
				code: z.ZodIssueCode.custom,
				message: "Only a promoting build must hold an active operation lease.",
				path: ["active_operation"],
			});
		}
	});

export type PromotionState = z.infer<typeof promotionStateSchema>;

const safeSegmentSchema = z
	.string()
	.trim()
	.min(1)
	.max(128)
	.regex(/^[A-Za-z0-9_-]+$/, "must be a safe path segment");

export const appBuildStateSchema = z
	.object({
		schema_version: z.literal(APP_BUILD_STATE_SCHEMA_VERSION),
		build_id: safeSegmentSchema,
		app_id: safeSegmentSchema,
		revision: z.number().int().nonnegative(),
		original_request: z.string().min(1).max(64_000).optional(),
		original_request_fingerprint: z.string().trim().min(1).max(128).optional(),
		spec: appSpecSchema,
		spec_fingerprint: z.string().trim().min(1).max(128),
		resources: z.record(z.string(), appBuildResourceStateSchema),
		retired_resources: z.record(z.string(), retiredResourceStateSchema),
		scenarios: z.record(z.string(), appBuildScenarioStateSchema),
		validation_revision: z.number().int().nonnegative().optional(),
		structural_evidence: structuralEvidenceSchema.optional(),
		promotion: promotionStateSchema,
		created_at_ms: z.number().int().nonnegative(),
		updated_at_ms: z.number().int().nonnegative(),
	})
	.strict()
	.superRefine((build, context) => {
		if (!!build.original_request !== !!build.original_request_fingerprint) {
			context.addIssue({
				code: z.ZodIssueCode.custom,
				message:
					"original_request and original_request_fingerprint must be stored together.",
				path: ["original_request_fingerprint"],
			});
		}
		const activeResourceLeases = Object.values(build.resources)
			.map((resource) => resource.active_operation)
			.filter((lease): lease is AppBuildOperationLease => lease !== undefined);
		if (activeResourceLeases.length > 0 && build.promotion.active_operation) {
			context.addIssue({
				code: z.ZodIssueCode.custom,
				message:
					"Resource application and promotion operations cannot be active at the same time.",
				path: ["promotion", "active_operation"],
			});
		}
		const firstResourceLease = activeResourceLeases[0];
		if (
			firstResourceLease &&
			activeResourceLeases.some(
				(lease) =>
					lease.operation_id !== firstResourceLease.operation_id ||
					lease.owner_id !== firstResourceLease.owner_id ||
					lease.started_at_ms !== firstResourceLease.started_at_ms ||
					lease.expires_at_ms !== firstResourceLease.expires_at_ms,
			)
		) {
			context.addIssue({
				code: z.ZodIssueCode.custom,
				message:
					"All resources in an applying wave must share the exact same operation lease.",
				path: ["resources"],
			});
		}
		const actual = serializedJsonByteLength(build);
		if (actual > MAX_APP_BUILD_STATE_BYTES) {
			context.addIssue({
				code: z.ZodIssueCode.custom,
				message: `AppBuildState is ${actual} bytes; the durable checkpoint maximum is ${MAX_APP_BUILD_STATE_BYTES} bytes.`,
				path: [],
			});
		}
	});

export type AppBuildState = z.infer<typeof appBuildStateSchema>;

export interface BuildStateIssue {
	readonly code: string;
	readonly path: readonly (string | number)[];
	readonly message: string;
}

export type BuildValidation =
	| {
			readonly ok: true;
			readonly build: AppBuildState;
			readonly compiled: CompiledAppSpec;
	  }
	| { readonly ok: false; readonly issues: readonly BuildStateIssue[] };

export interface BuildReadinessBlocker {
	readonly code:
		| "invalid_build"
		| "resource_not_applied"
		| "retired_resource_cleanup_required"
		| "validation_snapshot_missing"
		| "structural_evidence_missing"
		| "structural_evidence_failed"
		| "structural_evidence_stale"
		| "scenario_contract_missing"
		| "scenario_evidence_missing"
		| "scenario_failed"
		| "scenario_blocked"
		| "scenario_contract_only"
		| "scenario_evidence_stale"
		| "requirement_behavioral_evidence_missing"
		| "promotion_in_doubt";
	readonly message: string;
	readonly resource_key?: string;
	readonly scenario_id?: string;
	readonly requirement_id?: string;
}

export interface BuildReadiness {
	readonly level:
		| "invalid"
		| "building"
		| "awaiting_validation"
		| "structural_preview"
		| "ready"
		| "promoted";
	readonly structural_preview: boolean;
	readonly behaviorally_ready: boolean;
	readonly can_promote: boolean;
	readonly validation_revision?: number;
	readonly blockers: readonly BuildReadinessBlocker[];
}
