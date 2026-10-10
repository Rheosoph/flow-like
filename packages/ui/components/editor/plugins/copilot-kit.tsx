"use client";

import { CopilotPlugin } from "@platejs/ai/react";
import { serializeMd, stripMarkdown } from "@platejs/markdown";
import { RangeApi, type TElement } from "platejs";
import { IRole } from "../../../lib/schema/llm/history";
import { useBackendStore } from "../../../state/backend-state";
import { completeEditorChat } from "../ai-transport";
import { GhostText } from "../ui/ghost-text";
import { MarkdownKit } from "./markdown-kit";

const SYSTEM_PROMPT = `You are an advanced AI writing assistant, similar to VSCode Copilot but for general text. Your task is to predict and generate the next part of the text based on the given context.

  Rules:
  - Continue the text naturally up to the next punctuation mark (., ,, ;, :, ?, or !).
  - Maintain style and tone. Don't repeat given text.
  - For unclear context, provide the most likely continuation.
  - Handle code snippets, lists, or structured text if needed.
  - Don't include """ in your response.
  - CRITICAL: Always end with a punctuation mark.
  - CRITICAL: Avoid starting a new block. Do not use block formatting like >, #, 1., 2., -, etc. The suggestion should continue in the same block as the context.
  - If no context is provided or you can't generate a continuation, return "0" without explanation.`;

export const createCopilotKit = (appId?: string) => [
	...MarkdownKit,
	CopilotPlugin.configure(({ api, editor }) => ({
		options: {
			completeOptions: {
				api: "/api/ai/copilot",
				body: {
					system: SYSTEM_PROMPT,
				},
				// Plate only calls `fetch(api, init)`; Bun's `typeof fetch` also declares `preconnect`.
				fetch: (async (_request, init) => {
					init?.signal?.throwIfAborted();
					const backend = useBackendStore.getState().backend;
					const profileId = backend?.profile?.id;
					const body = JSON.parse(init?.body?.toString() ?? "{}") || {};
					const value = editor.children;
					const selection = editor.selection;

					if (!backend) {
						throw new Error("Backend not initialized");
					}
					const response = await completeEditorChat(
						backend.aiState,
						[
							{
								role: IRole.System,
								content: body.system,
							},
							{
								role: IRole.User,
								content: body.prompt,
							},
						],
						appId,
						init?.signal,
					);

					init?.signal?.throwIfAborted();
					// A native completion can finish after the user edits or moves the cursor.
					if (
						backend !== useBackendStore.getState().backend ||
						profileId !== backend.profile?.id ||
						value !== editor.children ||
						!selection ||
						!editor.selection ||
						!RangeApi.equals(selection, editor.selection)
					) {
						return new Response(JSON.stringify({ text: "0" }));
					}

					const text = response.choices[0]?.message?.content || "0";
					return new Response(JSON.stringify({ text }));
				}) as typeof fetch,
				onFinish: (_, completion) => {
					if (!completion.trim() || completion.trim() === "0") return;

					api.copilot.setBlockSuggestion({
						text: stripMarkdown(completion),
					});
				},
			},
			debounceDelay: 500,
			renderGhostText: GhostText,
			getPrompt: ({ editor }) => {
				const contextEntry = editor.api.block({ highest: true });

				if (!contextEntry) return "";

				const prompt = serializeMd(editor, {
					value: [contextEntry[0] as TElement],
				});

				return `Continue the text up to the next punctuation mark:
  """
  ${prompt}
  """`;
			},
		},
		shortcuts: {
			accept: {
				keys: "tab",
			},
			acceptNextWord: {
				keys: "mod+right",
			},
			reject: {
				keys: "escape",
			},
			triggerSuggestion: {
				keys: "ctrl+space",
			},
		},
	})).extendApi(({ api }) => {
		const reject = api.copilot.reject;
		return {
			reject: () => {
				// Plate only stops on rejection after ghost text exists.
				api.copilot.stop();
				return reject();
			},
		};
	}),
];

export const CopilotKit = createCopilotKit();
