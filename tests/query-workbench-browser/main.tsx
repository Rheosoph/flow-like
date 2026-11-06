import { createI18n } from "@flow-like/locales";
import { loader } from "@monaco-editor/react";
import { QueryClient, QueryClientProvider } from "@tanstack/react-query";
import { Boxes, Layers, Moon, Sun } from "lucide-react";
import { ThemeProvider, useTheme } from "next-themes";
import { useState } from "react";
import { createRoot } from "react-dom/client";
import { Toaster } from "sonner";
import { QueryWorkbench } from "../../packages/ui/components/settings/data-studio/query-workbench/query-workbench";
import { TooltipProvider } from "../../packages/ui/components/ui/tooltip";
import {
	BackendContext,
	type IBackendState,
	useBackend,
} from "../../packages/ui/state/backend-state";
import type { SavedQuery } from "../../packages/ui/state/backend-state/query-state";
import {
	appId,
	columns,
	execute,
	ontology,
	remoteImport,
	rows,
	savedQueries,
} from "./data";
import "./fixture.css";

declare const __QUERY_MONACO_PATH__: string;
loader.config({ paths: { vs: __QUERY_MONACO_PATH__ } });
createI18n({ language: "en" });

const params = new URLSearchParams(location.search);
const fixtureRows = params.has("many")
	? Array.from({ length: 400 }, (_, index) => ({
			...rows[index % rows.length],
			id: `QA_${String(index + 1).padStart(4, "0")}`,
		}))
	: rows;
const calls: { method: string; args: unknown[] }[] = [];
const library: SavedQuery[] = params.has("empty-library")
	? []
	: [...savedQueries];
Object.assign(window, { queryWorkbenchQa: { calls, library } });
const pause = () =>
	new Promise((resolve) =>
		setTimeout(resolve, params.has("slow") ? 3000 : 350),
	);
const client = new QueryClient({
	defaultOptions: { queries: { retry: false, refetchOnWindowFocus: false } },
});

function Fixture() {
	const base = useBackend();
	const { resolvedTheme, setTheme } = useTheme();
	const [userScoped, setUserScoped] = useState(false);
	const [backend] = useState<IBackendState>(() => ({
		...base,
		roleState: {
			...base.roleState,
			async getOwnRole() {
				return {
					role_id: "owner",
					role_name: "Owner",
					permissions: 1,
					is_owner: true,
					can_leave: false,
				};
			},
		},
		dbState: {
			...base.dbState,
			async getSchema() {
				return {
					fields: columns.map((column) => ({
						name: column.name,
						data_type: column.type_name,
						nullable: true,
					})),
				};
			},
		},
		storageState: {
			...base.storageState,
			async downloadStorageItems(_appId, paths) {
				return paths.map((path) => ({
					prefix: path,
					url: URL.createObjectURL(
						new Blob(
							[
								"Inspection complete. All measurements are within the specified tolerances.",
							],
							{ type: "text/plain" },
						),
					),
				}));
			},
		},
		graphState: {
			...base.graphState,
			async queryRemoteImport(_appId, importId, payload) {
				calls.push({ method: "queryRemoteImport", args: [importId, payload] });
				await pause();
				return execute(payload, fixtureRows);
			},
		},
		queryState: {
			...base.queryState,
			async executeSql(_appId, payload, scoped) {
				calls.push({ method: "executeSql", args: [payload, scoped] });
				await pause();
				return execute(payload, fixtureRows);
			},
			async listSavedQueries(_appId, scoped) {
				calls.push({ method: "listSavedQueries", args: [scoped] });
				return scoped ? [] : [...library];
			},
			async getSavedQuery(_appId, id) {
				const query = library.find((item) => item.id === id);
				if (!query) throw new Error("Query not found");
				return query;
			},
			async createSavedQuery(_appId, payload) {
				const query: SavedQuery = {
					...payload,
					id: crypto.randomUUID(),
					app_id: appId,
					created_at: new Date().toISOString(),
					updated_at: new Date().toISOString(),
				};
				library.push(query);
				return query;
			},
			async updateSavedQuery(_appId, id, payload) {
				const index = library.findIndex((item) => item.id === id);
				if (index < 0) throw new Error("Query not found");
				const query = {
					...library[index],
					...payload,
					updated_at: new Date().toISOString(),
				} as SavedQuery;
				library[index] = query;
				return query;
			},
			async deleteSavedQuery(_appId, id) {
				const index = library.findIndex((item) => item.id === id);
				if (index >= 0) library.splice(index, 1);
			},
		},
	}));
	return (
		<BackendContext.Provider value={backend}>
			<TooltipProvider>
				<div className="flex min-h-dvh flex-col bg-background text-foreground">
					<div className="border-b px-4 py-1.5 text-center text-[11px] text-muted-foreground">
						Local preview · Synthetic process documents · Actual Queries
						components
					</div>
					<main className="mx-auto flex min-h-0 w-full max-w-[1600px] flex-1 flex-col px-3 py-4 sm:px-6">
						<header className="mb-4 flex items-center justify-between gap-4">
							<div className="flex items-center gap-3">
								<div className="rounded-xl bg-primary/10 p-3 text-primary">
									<Layers className="h-6 w-6" />
								</div>
								<div>
									<p className="text-xs text-muted-foreground">
										VMV Expert / Project data
									</p>
									<h1 className="mt-1 text-2xl font-semibold tracking-tight">
										Data Studio
									</h1>
								</div>
							</div>
							<button
								type="button"
								aria-label="Toggle color theme"
								className="rounded-md border p-2"
								onClick={() =>
									setTheme(resolvedTheme === "dark" ? "light" : "dark")
								}
							>
								{resolvedTheme === "dark" ? (
									<Sun className="h-4 w-4" />
								) : (
									<Moon className="h-4 w-4" />
								)}
							</button>
						</header>
						<div
							className="mb-4 flex items-center gap-5 overflow-x-auto border-b text-sm text-muted-foreground"
							aria-label="Data Studio context"
						>
							<span className="flex items-center gap-1.5 pb-2">
								<Boxes className="h-4 w-4" />
								Explore
							</span>
							<span className="pb-2">Model</span>
							<span className="pb-2">Sources</span>
							<span className="border-b-2 border-primary pb-2 font-medium text-foreground">
								Queries
							</span>
							<span className="pb-2">Forecast</span>
						</div>
						<div
							className="@container/ds min-h-[560px] shrink-0 overflow-hidden rounded-xl border bg-card"
							style={{ height: "calc(100dvh - 185px)" }}
						>
							<QueryWorkbench
								appId={appId}
								ontologies={[ontology]}
								remoteImports={[remoteImport]}
								resolveSourceName={() => "Supplier workspace"}
								projectTables={["data", "process_metrics"]}
								userTables={["my_documents"]}
								userScoped={userScoped}
								onScopeChange={setUserScoped}
							/>
						</div>
					</main>
				</div>
				<Toaster />
			</TooltipProvider>
		</BackendContext.Provider>
	);
}

const root = document.getElementById("root");
if (!root) throw new Error("Fixture root is missing");
createRoot(root).render(
	<QueryClientProvider client={client}>
		<ThemeProvider
			attribute="class"
			defaultTheme={params.has("light") ? "light" : "dark"}
			enableSystem={false}
		>
			<Fixture />
		</ThemeProvider>
	</QueryClientProvider>,
);
