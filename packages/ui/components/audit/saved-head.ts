import type { IAuditHeadCheckRequest } from "./types";

type Anchor = { seq: number; hash: string };

function anchorOf(value: unknown): Anchor | null {
	if (typeof value !== "object" || value === null) return null;
	const { seq, hash } = value as { seq?: unknown; hash?: unknown };
	if (!Number.isSafeInteger(seq) || typeof hash !== "string") return null;
	if (!/^[0-9a-f]{64}$/i.test(hash)) return null;
	return { seq: seq as number, hash: hash.toLowerCase() };
}

function chainIdOf(...candidates: unknown[]): string | null {
	for (const candidate of candidates) {
		if (typeof candidate === "string" && candidate.trim()) {
			return candidate.trim();
		}
	}
	return null;
}

/**
 * Accepts what a holder may have kept: an `AuditHead`, a daily `heads/` file
 * (`{ epoch, written_at_ms }`) or a bare epoch line. An `AuditHead` with a seal
 * also has its seal checked, so a malformed seal rejects the whole head.
 */
export function parseSavedHead(text: string): IAuditHeadCheckRequest | null {
	let parsed: unknown;
	try {
		parsed = JSON.parse(text);
	} catch {
		return null;
	}
	if (typeof parsed !== "object" || parsed === null) return null;
	const container = parsed as {
		epoch?: unknown;
		seal?: unknown;
		chain_id?: unknown;
	};
	const epoch = anchorOf(
		typeof container.epoch === "object" && container.epoch !== null
			? container.epoch
			: parsed,
	);
	if (!epoch) return null;
	if (typeof container.seal !== "object" || container.seal === null) {
		return epoch;
	}
	const seal = anchorOf(container.seal);
	const chainId = chainIdOf(
		container.chain_id,
		(container.seal as { chain_id?: unknown }).chain_id,
	);
	if (!seal || !chainId) return null;
	return {
		...epoch,
		chain_id: chainId,
		seal_seq: seal.seq,
		seal_hash: seal.hash,
	};
}
