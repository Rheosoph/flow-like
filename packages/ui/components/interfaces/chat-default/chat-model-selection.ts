import { createStore, del, get, set } from "idb-keyval";
import {
	type LocalModelHostCapabilities,
	selectProfileLlmModels,
} from "../../../lib/bit/local-model-filter";
import { type IBit, IBitTypes } from "../../../lib/schema/bit/bit";

let modelStore: ReturnType<typeof createStore> | undefined;

function selectionStore() {
	modelStore ??= createStore("flow-like-chat-models", "app-models");
	return modelStore;
}

export async function readChatModel(
	appId: string,
): Promise<string | undefined> {
	try {
		const value = await get<unknown>(appId, selectionStore());
		return typeof value === "string" && value.trim() ? value : undefined;
	} catch {
		return undefined;
	}
}

export async function storeChatModel(
	appId: string,
	modelId: string | undefined,
): Promise<void> {
	try {
		if (modelId) await set(appId, modelId, selectionStore());
		else await del(appId, selectionStore());
	} catch {
		// Blocked browser storage still allows a selection for this session.
	}
}

export function chatModelOptions(
	profileBits: readonly IBit[] | undefined,
	profileBitRefs: readonly string[] | undefined,
	capabilities: LocalModelHostCapabilities,
	requireVlm: boolean,
): IBit[] {
	return selectProfileLlmModels(
		profileBits,
		[],
		profileBitRefs,
		capabilities,
	).filter((bit) => !requireVlm || bit.type === IBitTypes.Vlm);
}
