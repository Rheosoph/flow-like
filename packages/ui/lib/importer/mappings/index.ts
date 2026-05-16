// Auto-generated barrel. To add a new built-in mapping, create matching files in n8n/
// and flow/ then add the import pair below. Use overrides.ts for manual remaps.
import { N8N_MAPPING_OVERRIDES } from "./overrides";
import type {
	N8nManualMappingOverrides,
	NodeMappingDef,
	ResolvedN8nMappingDef,
} from "./types";
export { N8N_MAPPING_OVERRIDES } from "./overrides";
export type {
	N8nNodeDef,
	FlowNodeDef,
	NodeMappingDef,
	N8nManualMappingOverride,
	N8nManualMappingOverrides,
	ResolvedN8nMappingDef,
} from "./types";

import chat_trigger_n8n from "./n8n/chat_trigger";
import code_n8n from "./n8n/code";
import discord_n8n from "./n8n/discord";
import gmail_n8n from "./n8n/gmail";
import google_sheets_n8n from "./n8n/google_sheets";
import http_request_n8n from "./n8n/http_request";
import if_n8n from "./n8n/if";
// ── N8n imports ──
import manual_trigger_n8n from "./n8n/manual_trigger";
import merge_n8n from "./n8n/merge";
import model_anthropic_n8n from "./n8n/model_anthropic";
import model_azure_openai_n8n from "./n8n/model_azure_openai";
import model_cohere_n8n from "./n8n/model_cohere";
import model_deepseek_n8n from "./n8n/model_deepseek";
import model_gemini_n8n from "./n8n/model_gemini";
import model_groq_n8n from "./n8n/model_groq";
import model_huggingface_n8n from "./n8n/model_huggingface";
import model_mistral_n8n from "./n8n/model_mistral";
import model_ollama_n8n from "./n8n/model_ollama";
import model_openai_n8n from "./n8n/model_openai";
import no_op_n8n from "./n8n/no_op";
import respond_to_webhook_n8n from "./n8n/respond_to_webhook";
import schedule_trigger_n8n from "./n8n/schedule_trigger";
import set_n8n from "./n8n/set";
import split_in_batches_n8n from "./n8n/split_in_batches";
import switch_n8n from "./n8n/switch";
import telegram_n8n from "./n8n/telegram";
import wait_n8n from "./n8n/wait";
import webhook_n8n from "./n8n/webhook";

import chat_trigger_flow from "./flow/chat_trigger";
import code_flow from "./flow/code";
import discord_flow from "./flow/discord";
import gmail_flow from "./flow/gmail";
import google_sheets_flow from "./flow/google_sheets";
import http_request_flow from "./flow/http_request";
import if_flow from "./flow/if";
// ── Flow imports ──
import manual_trigger_flow from "./flow/manual_trigger";
import merge_flow from "./flow/merge";
import model_anthropic_flow from "./flow/model_anthropic";
import model_azure_openai_flow from "./flow/model_azure_openai";
import model_cohere_flow from "./flow/model_cohere";
import model_deepseek_flow from "./flow/model_deepseek";
import model_gemini_flow from "./flow/model_gemini";
import model_groq_flow from "./flow/model_groq";
import model_huggingface_flow from "./flow/model_huggingface";
import model_mistral_flow from "./flow/model_mistral";
import model_ollama_flow from "./flow/model_ollama";
import model_openai_flow from "./flow/model_openai";
import no_op_flow from "./flow/no_op";
import respond_to_webhook_flow from "./flow/respond_to_webhook";
import schedule_trigger_flow from "./flow/schedule_trigger";
import set_flow from "./flow/set";
import split_in_batches_flow from "./flow/split_in_batches";
import switch_flow from "./flow/switch";
import telegram_flow from "./flow/telegram";
import wait_flow from "./flow/wait";
import webhook_flow from "./flow/webhook";

/** All mapping definitions, keyed by mapping name. */
export const MAPPING_DEFS: Record<string, NodeMappingDef> = {
	manual_trigger: { n8n: manual_trigger_n8n, flow: manual_trigger_flow },
	schedule_trigger: { n8n: schedule_trigger_n8n, flow: schedule_trigger_flow },
	webhook: { n8n: webhook_n8n, flow: webhook_flow },
	chat_trigger: { n8n: chat_trigger_n8n, flow: chat_trigger_flow },
	if: { n8n: if_n8n, flow: if_flow },
	switch: { n8n: switch_n8n, flow: switch_flow },
	split_in_batches: { n8n: split_in_batches_n8n, flow: split_in_batches_flow },
	wait: { n8n: wait_n8n, flow: wait_flow },
	no_op: { n8n: no_op_n8n, flow: no_op_flow },
	merge: { n8n: merge_n8n, flow: merge_flow },
	http_request: { n8n: http_request_n8n, flow: http_request_flow },
	respond_to_webhook: {
		n8n: respond_to_webhook_n8n,
		flow: respond_to_webhook_flow,
	},
	set: { n8n: set_n8n, flow: set_flow },
	code: { n8n: code_n8n, flow: code_flow },
	gmail: { n8n: gmail_n8n, flow: gmail_flow },
	google_sheets: { n8n: google_sheets_n8n, flow: google_sheets_flow },
	telegram: { n8n: telegram_n8n, flow: telegram_flow },
	discord: { n8n: discord_n8n, flow: discord_flow },
	model_gemini: { n8n: model_gemini_n8n, flow: model_gemini_flow },
	model_ollama: { n8n: model_ollama_n8n, flow: model_ollama_flow },
	model_openai: { n8n: model_openai_n8n, flow: model_openai_flow },
	model_azure_openai: {
		n8n: model_azure_openai_n8n,
		flow: model_azure_openai_flow,
	},
	model_anthropic: { n8n: model_anthropic_n8n, flow: model_anthropic_flow },
	model_groq: { n8n: model_groq_n8n, flow: model_groq_flow },
	model_mistral: { n8n: model_mistral_n8n, flow: model_mistral_flow },
	model_huggingface: {
		n8n: model_huggingface_n8n,
		flow: model_huggingface_flow,
	},
	model_cohere: { n8n: model_cohere_n8n, flow: model_cohere_flow },
	model_deepseek: { n8n: model_deepseek_n8n, flow: model_deepseek_flow },
};

/** Lookup: n8n type string → mapping name. */
export const N8N_TYPE_INDEX: Record<string, string> = {};
for (const [name, def] of Object.entries(MAPPING_DEFS)) {
	N8N_TYPE_INDEX[def.n8n.type] = name;
}

export function resolveN8nMappingDefs(
	overrides: N8nManualMappingOverrides = N8N_MAPPING_OVERRIDES,
): ResolvedN8nMappingDef[] {
	const resolved = new Map<string, ResolvedN8nMappingDef>();

	for (const [name, def] of Object.entries(MAPPING_DEFS)) {
		resolved.set(def.n8n.type, {
			name,
			source: "built-in",
			n8n: def.n8n,
			flow: def.flow,
		});
	}

	for (const [type, override] of Object.entries(overrides)) {
		const base = resolved.get(type);
		resolved.set(type, {
			name: override.name ?? base?.name ?? type,
			source: "override",
			category: override.category ?? (base ? undefined : "Custom"),
			n8n: {
				...(base?.n8n ?? { type }),
				...(override.n8n ?? {}),
				type,
			},
			flow: override.flow,
		});
	}

	return Array.from(resolved.values());
}
