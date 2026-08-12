import { existsSync, statSync } from "node:fs";
import { join } from "node:path";
import {
	type JsonValue,
	type WidgetContract,
	canonicalizeContract,
} from "../contract-types";
import { type ExtractResult, extractContract } from "../extract";
import { type PropsFormField, derivePropsFormModel } from "./form-model";

export interface ContractEndpointPayload {
	contract: WidgetContract;
	fixtures: Record<string, JsonValue>;
	formModel: PropsFormField[];
	warnings: string[];
}

/** Pure part of the contract endpoint: extraction result → response body. */
export function buildContractPayload(
	extracted: ExtractResult,
): ContractEndpointPayload {
	const contract = canonicalizeContract(extracted.contract);
	return {
		contract,
		fixtures: extracted.config.fixtures ?? {},
		formModel: derivePropsFormModel(contract),
		warnings: extracted.warnings,
	};
}

export interface ContractGroupRef {
	name: string;
	dir: string;
}

const WIDGET_ID_RE = /^[a-z0-9-]+$/;

/**
 * Resolve `GET /api/contract/<group>/<id>` to the widget's config path.
 * Returns `null` for unknown groups or ids that are not plain widget ids
 * (which also guards the endpoint against path traversal).
 */
export function resolveWidgetConfigPath(
	groups: readonly ContractGroupRef[],
	groupName: string,
	widgetId: string,
): string | null {
	if (!WIDGET_ID_RE.test(widgetId)) return null;
	const group = groups.find((candidate) => candidate.name === groupName);
	if (!group) return null;
	return join(group.dir, "src", "widgets", widgetId, "widget.config.ts");
}

/**
 * Contract extraction cached by `widget.config.ts` mtime: each request
 * re-checks the file so config edits are picked up, without re-running the
 * TypeScript program when nothing changed. (Edits to imported sibling type
 * files require touching the config file — same limitation as the Vite
 * plugin's dev cache.)
 */
export class ContractCache {
	private readonly cache = new Map<
		string,
		{ mtimeMs: number; result: ExtractResult }
	>();

	get(configPath: string): ExtractResult {
		const mtimeMs = statSync(configPath).mtimeMs;
		const cached = this.cache.get(configPath);
		if (cached && cached.mtimeMs === mtimeMs) return cached.result;
		const result = extractContract(configPath);
		this.cache.set(configPath, { mtimeMs, result });
		return result;
	}
}

export interface ContractEndpointResponse {
	status: number;
	body: ContractEndpointPayload | { error: string };
}

export function handleContractRequest(
	cache: ContractCache,
	groups: readonly ContractGroupRef[],
	groupName: string,
	widgetId: string,
): ContractEndpointResponse {
	const configPath = resolveWidgetConfigPath(groups, groupName, widgetId);
	if (configPath === null || !existsSync(configPath)) {
		return {
			status: 404,
			body: { error: `Unknown widget '${groupName}/${widgetId}'` },
		};
	}
	try {
		return { status: 200, body: buildContractPayload(cache.get(configPath)) };
	} catch (e) {
		return {
			status: 500,
			body: { error: e instanceof Error ? e.message : String(e) },
		};
	}
}
