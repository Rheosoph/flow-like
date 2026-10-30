import {
	ArrowUpRight,
	FolderClosed,
	Layers3,
	Plus,
	Search,
} from "lucide-react";
import { createRoot } from "react-dom/client";
import { RunningTasksIndicator } from "../../packages/ui/components/execution-indicator";
import "../../packages/ui/global.css";
import { FixtureEngineContext } from "./execution-context";
import { MockExecutionEngine } from "./mock-engine";
import { usePathname, useRouter } from "./next-navigation";

const params = new URLSearchParams(location.search);
document.documentElement.classList.toggle("dark", params.has("dark"));
const worker = params.has("worker")
	? await (await import("./worker-engine")).createWorkerFixtureEngine(
			params.get("scope") || "execution-browser",
		)
	: undefined;
const engine = worker?.engine ?? new MockExecutionEngine();
Object.assign(window, { executionQa: { engine, ...worker } });

function Fixture() {
	const path = usePathname();
	const router = useRouter();
	return (
		<FixtureEngineContext.Provider value={engine}>
			<div className="min-h-screen bg-background text-foreground">
				<header className="flex h-16 items-center gap-3 border-b border-border/60 px-6 sm:px-10">
					<div className="flex size-7 items-center justify-center rounded-lg bg-primary text-primary-foreground">
						<Layers3 className="size-4" />
					</div>
					<span className="text-sm font-semibold tracking-tight">
						Flow-Like
					</span>
					<span className="mx-2 text-border">/</span>
					<span className="text-xs text-muted-foreground">Workspace</span>
					<div className="ml-auto flex size-7 items-center justify-center rounded-full border border-border bg-muted text-[10px] font-medium">
						AM
					</div>
				</header>
				<main className="mx-auto max-w-5xl px-6 pb-28 pt-12 sm:px-10 sm:pt-20">
					<p className="text-[11px] font-medium tracking-[0.12em] text-muted-foreground uppercase">
						Your workspace
					</p>
					<h1 className="mt-3 text-3xl font-semibold tracking-tight">
						{path.startsWith("/chat/")
							? "Conversation"
							: "Good work starts here."}
					</h1>
					<p className="mt-3 max-w-md text-sm leading-relaxed text-muted-foreground">
						Keep your everyday work moving with the apps your team has built.
					</p>
					<div className="mt-10 flex items-center gap-3 border-b border-border/70 pb-4">
						<h2 className="text-sm font-semibold">Your apps</h2>
						<span className="text-xs text-muted-foreground">4</span>
						<button
							type="button"
							className="ml-auto flex size-7 items-center justify-center rounded-md text-muted-foreground"
							aria-label="Search apps"
						>
							<Search className="size-4" />
						</button>
						<button
							type="button"
							className="flex items-center gap-1.5 rounded-md border border-border px-2.5 py-1.5 text-xs font-medium"
						>
							<Plus className="size-3.5" />
							New app
						</button>
					</div>
					<div className="grid gap-4 pt-5 sm:grid-cols-2">
						{[
							"Operations assistant",
							"Knowledge chat",
							"Customer insights",
							"Weekly briefing",
						].map((title, i) => (
							<button
								key={title}
								type="button"
								onClick={() => router.push(`/workspace/${i}`)}
								className="flex items-center gap-4 rounded-xl border border-border/60 bg-card px-5 py-5 text-left transition-colors hover:bg-muted/30"
							>
								<div className="flex size-10 shrink-0 items-center justify-center rounded-lg bg-muted">
									<FolderClosed className="size-4 text-muted-foreground" />
								</div>
								<div>
									<p className="text-[13px] font-medium">{title}</p>
									<p className="mt-1 text-xs text-muted-foreground">
										{
											[
												"Invoices, approvals, and daily checks",
												"Answers across your team’s documents",
												"Turn feedback into next steps",
												"A clear view of the week ahead",
											][i]
										}
									</p>
								</div>
								<ArrowUpRight className="ml-auto size-4 shrink-0 text-muted-foreground/60" />
							</button>
						))}
					</div>
				</main>
				<RunningTasksIndicator />
			</div>
		</FixtureEngineContext.Provider>
	);
}

const root = document.getElementById("root");
if (!root) throw new Error("Fixture root missing");
createRoot(root).render(<Fixture />);
