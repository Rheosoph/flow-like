import { PrismaPg } from "@prisma/adapter-pg";
import { type Prisma, PrismaClient } from "@prisma/client";
import pg from "pg";
import {
	type AAModel,
	type ModelClassification,
	type ModelClassificationEstimates,
	isCatalogEstimateSlug,
} from "./types";

let prisma: PrismaClient | null = null;

export function getPrisma(): PrismaClient {
	if (!prisma) {
		const connectionString = process.env.DATABASE_URL;
		if (!connectionString) throw new Error("DATABASE_URL is not set");
		const pool = new pg.Pool({ connectionString });
		const adapter = new PrismaPg(pool);
		prisma = new PrismaClient({ adapter });
	}
	return prisma;
}

export async function disconnect(): Promise<void> {
	if (prisma) {
		await prisma.$disconnect();
		prisma = null;
	}
}

export interface BitWithModel {
	id: string;
	modelSlug: string | null;
	parameters: Record<string, unknown> | null;
	downloadLink: string | null;
}

export async function fetchBitsWithModel(): Promise<BitWithModel[]> {
	const db = getPrisma();
	const bits = await db.bit.findMany({
		where: { modelSlug: { not: null } },
		select: {
			id: true,
			modelSlug: true,
			parameters: true,
			downloadLink: true,
		},
	});

	return bits.map((b) => ({
		id: b.id,
		modelSlug: b.modelSlug,
		parameters: b.parameters as Record<string, unknown> | null,
		downloadLink: b.downloadLink,
	}));
}

/** Derive tier from cost score. Returns null if no tier field exists or tier is free. */
export function computeTier(
	existingParams: Record<string, unknown> | null,
	costScore: number,
): { newTier: string; oldTier: string } | null {
	const provider = existingParams?.provider as
		| Record<string, unknown>
		| undefined;
	const params = provider?.params as Record<string, unknown> | undefined;
	const currentTier = params?.tier;
	if (typeof currentTier !== "string") return null;
	if (currentTier.toLowerCase() === "free") return null;

	const newTier = costScore >= 0.6 ? "PREMIUM" : "PRO";
	return { newTier, oldTier: currentTier };
}

export function mergeBitParameters(
	classification: ModelClassification,
	existingParams: Record<string, unknown> | null,
	classificationEstimates?: ModelClassificationEstimates,
): Record<string, unknown> {
	const merged: Record<string, unknown> = {
		...(existingParams ?? {}),
		model_classification: classification,
	};
	if (classificationEstimates) {
		merged.model_classification_estimates = classificationEstimates;
	}

	// Update tier if applicable
	const tierResult = computeTier(existingParams, classification.cost);
	if (tierResult) {
		const provider = { ...(merged.provider as Record<string, unknown>) };
		const params = { ...(provider.params as Record<string, unknown>) };
		params.tier = tierResult.newTier;
		provider.params = params;
		merged.provider = provider;
	}
	return merged;
}

export async function updateBitParameters(
	bitId: string,
	classification: ModelClassification,
	existingParams: Record<string, unknown> | null,
	releaseDate?: string | null,
	classificationEstimates?: ModelClassificationEstimates,
): Promise<void> {
	const db = getPrisma();
	const merged = mergeBitParameters(
		classification,
		existingParams,
		classificationEstimates,
	);

	const dateFields: Record<string, Date> = {};
	if (releaseDate) {
		const d = new Date(releaseDate);
		if (!Number.isNaN(d.getTime())) {
			dateFields.createdAt = d;
			dateFields.updatedAt = d;
		}
	}

	// `authors` and `dependencies` are text arrays in the database but Json in the
	// Prisma schema, so returning the whole row fails to deserialize. The update
	// only needs to happen, not to hand the row back.
	await db.bit.update({
		where: { id: bitId },
		data: {
			parameters: merged as unknown as Prisma.InputJsonValue,
			...dateFields,
		},
		select: { id: true },
	});
}

function intelligenceIndexVersion(evaluations: unknown): {
	label: string;
	parts: number[];
} | null {
	if (!evaluations || typeof evaluations !== "object") return null;
	const value = (evaluations as Record<string, unknown>)
		.artificial_analysis_intelligence_index_version;
	if (typeof value !== "string" && typeof value !== "number") return null;
	const label = String(value).trim();
	const match = /^v?(\d+)(?:\.(\d+))?(?:\.(\d+))?$/i.exec(label);
	if (!match) return null;
	const parts = match.slice(1).map((part) => Number(part ?? 0));
	return parts.every(Number.isSafeInteger) ? { label, parts } : null;
}

/** Keep older feeds from replacing a current score or its provenance. */
export function assertIntelligenceIndexVersionUpdate(
	existingEvaluations: unknown,
	incomingEvaluations: unknown,
	slug: string,
): void {
	const current = intelligenceIndexVersion(existingEvaluations);
	if (!current || current.parts[0] < 5) return;
	const incoming = intelligenceIndexVersion(incomingEvaluations);
	const downgradeError = new Error(
		`Refusing to replace ${slug} Intelligence Index v${current.label.replace(/^v/i, "")} with ${incoming ? `v${incoming.label.replace(/^v/i, "")}` : "an unversioned or invalid evaluation"}. Refresh current scores through refresh-aa-v5.ts.`,
	);
	if (!incoming) throw downgradeError;
	for (let index = 0; index < current.parts.length; index++) {
		if (incoming.parts[index] > current.parts[index]) return;
		if (incoming.parts[index] < current.parts[index]) throw downgradeError;
	}
}

export async function upsertLlmModel(aaModel: AAModel): Promise<void> {
	// These catalog records are maintained from research, outside the AA feed.
	if (isCatalogEstimateSlug(aaModel.slug)) return;
	const db = getPrisma();
	const data = {
		name: aaModel.name,
		releaseDate: aaModel.release_date ? new Date(aaModel.release_date) : null,
		creatorName: aaModel.model_creator.name,
		creatorSlug: aaModel.model_creator.slug,
		evaluations: aaModel.evaluations as unknown as Prisma.InputJsonValue,
		pricing: aaModel.pricing as unknown as Prisma.InputJsonValue,
		medianOutputTokensPerSecond: aaModel.median_output_tokens_per_second,
		medianTimeToFirstTokenSeconds: aaModel.median_time_to_first_token_seconds,
		medianTimeToFirstAnswerToken: aaModel.median_time_to_first_answer_token,
	};

	// A concurrent refresh must not insert a newer score between the check and write.
	await db.$transaction(
		async (transaction) => {
			const existing = await transaction.llmModel.findUnique({
				where: { slug: aaModel.slug },
				select: { evaluations: true },
			});
			assertIntelligenceIndexVersionUpdate(
				existing?.evaluations,
				aaModel.evaluations,
				aaModel.slug,
			);
			await transaction.llmModel.upsert({
				where: { slug: aaModel.slug },
				create: { slug: aaModel.slug, ...data },
				update: data,
			});
		},
		{ isolationLevel: "Serializable" },
	);
}
