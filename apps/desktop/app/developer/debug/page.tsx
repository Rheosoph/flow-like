"use client";

import { Button, Input } from "@flow-like/flow-like-ui";
import { useTranslation } from "@flow-like/locales";
import { open } from "@tauri-apps/plugin-dialog";
import {
	ArrowUpRight,
	Bug,
	FileCode2,
	FolderOpen,
	Loader2,
	Package,
	Search,
} from "lucide-react";
import Link from "next/link";
import { useSearchParams } from "next/navigation";
import { Suspense, useCallback, useEffect, useState } from "react";
import { projectRoutes } from "../../../components/packages/local-projects";
import {
	NodeDebuggerStatus,
	NodeDebuggerTabs,
	type NodeDebuggerTarget,
	type NodeDebuggerView,
	useNodeDebugger,
} from "../../../components/packages/workspace/node-debugger";
import {
	ToolPageHeader,
	toolBackHref,
} from "../../../components/packages/workspace/tool-page-header";

function DebugPageContent() {
	const { t } = useTranslation("common");
	const initialProject = useSearchParams().get("project") ?? "";
	const [target, setTarget] = useState<NodeDebuggerTarget>(() =>
		initialProject ? { projectPath: initialProject } : {},
	);
	const [wasmInput, setWasmInput] = useState("");
	const [view, setView] = useState<NodeDebuggerView>("run");
	const session = useNodeDebugger(target);

	useEffect(() => {
		if (initialProject) setTarget({ projectPath: initialProject });
	}, [initialProject]);

	useEffect(() => {
		if (session.wasmPath) setWasmInput(session.wasmPath);
	}, [session.wasmPath]);

	const selectWasm = useCallback(async () => {
		const selected = await open({
			multiple: false,
			filters: [{ name: t("wasmFiles", "WASM Files"), extensions: ["wasm"] }],
		});
		if (typeof selected === "string") setWasmInput(selected);
	}, [t]);

	const selectProject = useCallback(async () => {
		const selected = await open({ directory: true, multiple: false });
		if (typeof selected === "string") setTarget({ projectPath: selected });
	}, []);

	const { reinspect } = session;
	const inspectWasm = useCallback(() => {
		if (!wasmInput) return;
		if (!target.projectPath && target.wasmPath === wasmInput) reinspect();
		else setTarget({ wasmPath: wasmInput });
	}, [wasmInput, target, reinspect]);

	return (
		<div className="flex flex-col h-full">
			<ToolPageHeader
				icon={Bug}
				title={t("debugNode", "Debug Node")}
				description={t(
					"inspectPackageNodesPermissionsAndTestExecution",
					"Inspect package nodes, permissions, and test execution",
				)}
				backHref={toolBackHref(target.projectPath)}
				actions={
					target.projectPath && (
						<Button variant="outline" size="sm" asChild>
							<Link
								href={projectRoutes.workspace({
									project: target.projectPath,
									tab: "test",
								})}
							>
								<ArrowUpRight />
								{t("openInWorkspace", "Open in workspace")}
							</Link>
						</Button>
					)
				}
			/>

			<div className="flex-1 overflow-y-auto py-4 space-y-4">
				<div className="rounded-xl bg-muted/10 border border-border/20 p-3">
					<div className="flex flex-wrap items-center gap-3">
						<FileCode2 className="h-4 w-4 text-muted-foreground/60 shrink-0" />
						<Input
							value={wasmInput}
							onChange={(e) => setWasmInput(e.target.value)}
							placeholder={t("pathToWasmFile", "Path to .wasm file...")}
							className="min-w-48 flex-1 h-9 rounded-full bg-muted/30 border-transparent focus:border-border/40 focus:bg-muted/50"
						/>
						<Button
							variant="ghost"
							size="sm"
							onClick={selectWasm}
							className="h-8 rounded-full text-muted-foreground/60 hover:text-foreground/80 hover:bg-muted/30 gap-1.5 px-3"
						>
							<FolderOpen className="h-3.5 w-3.5" />
							{"WASM"}
						</Button>
						<Button
							variant="ghost"
							size="sm"
							onClick={selectProject}
							className="h-8 rounded-full text-muted-foreground/60 hover:text-foreground/80 hover:bg-muted/30 gap-1.5 px-3"
						>
							<Package className="h-3.5 w-3.5" />
							{t("project", "Project")}
						</Button>
						<Button
							size="sm"
							onClick={inspectWasm}
							disabled={!wasmInput || session.isFetching}
							className="h-8 rounded-full gap-1.5 px-4"
						>
							{session.isFetching ? (
								<Loader2 className="h-3.5 w-3.5 animate-spin" />
							) : (
								<>
									<Search className="h-3.5 w-3.5" />
									{t("inspect", "Inspect")}
								</>
							)}
						</Button>
					</div>
				</div>

				<NodeDebuggerStatus
					session={session}
					emptyHint={t(
						"noNodesInThisBuild",
						"This build exports no nodes. Build the package and inspect it again.",
					)}
				/>
				{session.nodes.length > 0 && (
					<NodeDebuggerTabs
						session={session}
						view={view}
						onViewChange={setView}
					/>
				)}
			</div>
		</div>
	);
}

export default function DebugPage() {
	return (
		<Suspense
			fallback={
				<div className="flex items-center justify-center h-full">
					<Loader2 className="h-6 w-6 animate-spin text-muted-foreground/60" />
				</div>
			}
		>
			<DebugPageContent />
		</Suspense>
	);
}
