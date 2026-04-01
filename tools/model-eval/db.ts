import { PrismaClient, Prisma } from "@prisma/client";
import { PrismaPg } from "@prisma/adapter-pg";
import pg from "pg";
import type { AAModel, ModelClassification } from "./types";

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
	const provider = existingParams?.provider as Record<string, unknown> | undefined;
	const params = provider?.params as Record<string, unknown> | undefined;
	const currentTier = params?.tier;
	if (typeof currentTier !== "string") return null;
	if (currentTier.toLowerCase() === "free") return null;

	const newTier = costScore >= 0.6 ? "PREMIUM" : "PRO";
	return { newTier, oldTier: currentTier };
}

export async function updateBitParameters(
	bitId: string,
	classification: ModelClassification,
	existingParams: Record<string, unknown> | null,
	releaseDate?: string | null,
): Promise<void> {
	const db = getPrisma();
	const merged: Record<string, unknown> = {
		...(existingParams ?? {}),
		model_classification: classification,
	};

	// Update tier if applicable
	const tierResult = computeTier(existingParams, classification.cost);
	if (tierResult) {
		const provider = { ...(merged.provider as Record<string, unknown>) };
		const params = { ...(provider.params as Record<string, unknown>) };
		params.tier = tierResult.newTier;
		provider.params = params;
		merged.provider = provider;
	}

	const dateFields: Record<string, Date> = {};
	if (releaseDate) {
		const d = new Date(releaseDate);
		if (!isNaN(d.getTime())) {
			dateFields.createdAt = d;
			dateFields.updatedAt = d;
		}
	}

	await db.bit.update({
		where: { id: bitId },
		data: {
			parameters: merged as unknown as Prisma.InputJsonValue,
			...dateFields,
		},
	});
}

export async function upsertLlmModel(aaModel: AAModel): Promise<void> {
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

	await db.llmModel.upsert({
		where: { slug: aaModel.slug },
		create: { slug: aaModel.slug, ...data },
		update: data,
	});
}
