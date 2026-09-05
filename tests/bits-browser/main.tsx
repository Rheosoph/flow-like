import { QueryClient, QueryClientProvider } from "@tanstack/react-query";
import { useState } from "react";
import { createRoot } from "react-dom/client";
import { Toaster } from "sonner";
import { BitEditorDialog } from "../../packages/ui/components/bits/bit-editor-dialog";
import { AdminBitsPage } from "../../packages/ui/components/pages/admin/bits/admin-bits-page";
import { emptyMetadata } from "../../packages/ui/components/bits/bit-editor-model";
import {
	useBackend,
	useBackendStore,
} from "../../packages/ui/state/backend-state";
import { IBitTypes, type IBit } from "../../packages/ui/lib/schema/bit/bit";
import "../../packages/ui/global.css";
const params = new URLSearchParams(location.search);
const bit: IBit = {
	id: "bit-review-1",
	type: IBitTypes.Llm,
	authors: ["Example team"],
	dependencies: ["hub:tokenizer"],
	hash: "stable-hash",
	dependency_tree_hash: "stable-tree",
	hub: "example.invalid",
	created: "2026-09-05",
	updated: "2026-09-05",
	version: "1.2",
	model_slug: "research-model",
	license: "Apache-2.0",
	parameters: {
		context_length: 128000,
		provider: {
			provider_name: params.has("custom") ? "OpenAI" : "Hosted",
			model_id: "research-model",
			api_surface: null,
			params: { endpoint: "https://api.example.invalid/v1", stream: true },
		},
		model_classification: { reasoning: 0.8, coding: 0.7, creativity: 0.6 },
		custom_options: { retained: "do not drop", enabled: true, attempts: 2 },
	},
	meta: {
		en: {
			...emptyMetadata(),
			name: "Research assistant",
			description:
				"A versatile model for thoughtful analysis, clear writing, and working with long documents.",
			tags: ["Research", "Writing"],
			website: "https://example.invalid",
		},
		de: {
			...emptyMetadata(),
			name: "Recherche-Assistent",
			description: "Ein Modell für Recherche und Analyse.",
			tags: ["Recherche"],
		},
	},
};
const client = new QueryClient({
	defaultOptions: { queries: { retry: false, refetchOnWindowFocus: false } },
});
function Fixture() {
	const backend = useBackend();
	const [open, setOpen] = useState(true);
	const [state] = useState(() => {
		const state = {
			saved: structuredClone(bit),
			calls: [] as { kind: string; data?: unknown; path?: string }[],
			fail: params.has("fail"),
			hold: false,
			release: null as null | (() => void),
		};
		(window as any).bitQa = state;
		useBackendStore.getState().setBackend({
			...backend,
			userState: {
				...backend.userState,
				getProfile: async function getProfile() {
					return { id: "fixture-profile" };
				},
			},
			bitState: {
				...backend.bitState,
				listCustomBits: async function listCustomBits() {
					return [state.saved];
				},
				upsertCustomBit: async function upsertCustomBit(
					next: IBit,
					secrets: unknown,
				) {
					state.calls.push({ kind: "custom", data: { bit: next, secrets } });
					if (state.hold)
						await new Promise<void>((resolve) => {
							state.release = resolve;
						});
					if (state.fail) throw new Error("Fixture save failed. Try again.");
					state.saved = structuredClone(next);
					return state.saved;
				},
			},
			apiState: {
				...backend.apiState,
				post: async (_: unknown, path: string, query: any) => {
					if (params.has("list-error")) throw new Error("Fixture list failure");
					const choices = [
						state.saved,
						{
							...state.saved,
							id: "bit-embedding",
							type: IBitTypes.Embedding,
							parameters: {
								vector_length: 768,
								input_length: 512,
								languages: ["en"],
								provider: { provider_name: "Local" },
							},
							meta: {
								en: {
									...emptyMetadata(),
									name: "Document embeddings",
									description: "Turn documents into searchable vectors.",
								},
							},
						},
						{
							...state.saved,
							id: "bit-missing-meta",
							type: IBitTypes.File,
							meta: {},
							parameters: null,
						},
					];
					return choices.filter(
						(b) =>
							(!query.search ||
								JSON.stringify(b.meta)
									.toLowerCase()
									.includes(query.search.toLowerCase())) &&
							(!query.bit_types || query.bit_types.includes(b.type)),
					);
				},
				get: async (_: unknown, path: string) =>
					path.endsWith("bit-embedding")
						? {
								...state.saved,
								id: "bit-embedding",
								type: IBitTypes.Embedding,
								parameters: {
									vector_length: 768,
									input_length: 512,
									languages: ["en"],
								},
							}
						: path.endsWith("bit-missing-meta")
							? {
									...state.saved,
									id: "bit-missing-meta",
									type: IBitTypes.File,
									meta: {},
									parameters: null,
								}
							: state.saved,
				stream: async (
					_: unknown,
					path: string,
					options: RequestInit,
					callback: (data: unknown) => void,
				) => {
					state.calls.push({
						kind: "core",
						path,
						data: JSON.parse(options.body as string),
					});
					if (params.has("stream-error")) {
						callback({ message: "Fixture stream failure" });
						return;
					}
					state.saved = {
						...JSON.parse(options.body as string),
						meta: state.saved.meta,
					};
					callback(state.saved);
				},
				put: async (_: unknown, path: string, data: any) => {
					state.calls.push({ kind: "metadata", path, data });
					if (state.fail)
						throw new Error("Fixture metadata save failed. Try again.");
					state.saved = {
						...state.saved,
						meta: { ...state.saved.meta, [path.split("/").pop()!]: data },
					};
				},
				del: async () => {
					state.calls.push({ kind: "delete" });
				},
			},
		} as any);
		return state;
	});
	return (
		<div className="h-screen bg-background text-foreground">
			<div className="border-b px-5 py-2 text-xs text-muted-foreground">
				Local verification · fixture data
			</div>
			{params.has("custom") ? (
				<>
					<button type="button" onClick={() => setOpen(true)}>
						Edit custom model
					</button>
					<BitEditorDialog
						bit={state.saved}
						open={open}
						scope="custom"
						onOpenChange={setOpen}
					/>
				</>
			) : (
				<AdminBitsPage />
			)}
			<Toaster />
		</div>
	);
}
createRoot(document.getElementById("root")!).render(
	<QueryClientProvider client={client}>
		<Fixture />
	</QueryClientProvider>,
);
