"use client";

import {
	Alert,
	AlertDescription,
	Badge,
	Button,
	Label,
	ScrollArea,
	Select,
	SelectContent,
	SelectItem,
	SelectTrigger,
	SelectValue,
	Skeleton,
	Tabs,
	TabsContent,
	TabsList,
	TabsTrigger,
} from "@flow-like/flow-like-ui";
import { getErrorMessage } from "@flow-like/flow-like-ui/lib/error-message";
import { useTranslation } from "@flow-like/locales";
import {
	AlertCircle,
	CheckCircle2,
	ChevronDown,
	ChevronRight,
	Loader2,
	Package,
	Play,
	RefreshCw,
	Shield,
	ShieldCheck,
} from "lucide-react";
import { useState } from "react";
import { LintPanel } from "./lint-panel";
import { NodeCard } from "./node-card";
import {
	NodePermissionBadges,
	NodePermissionsDetail,
	NodePermissionsSummary,
	PermissionsBadges,
	PermissionsDetail,
} from "./permissions";
import { DataTypeBadge, OutputValue, PinInput } from "./pin-inputs";
import type { NodeDebuggerSession } from "./use-node-debugger";

export type NodeDebuggerView = "run" | "nodes" | "lint" | "permissions";

export interface NodeDebuggerProps {
	session: NodeDebuggerSession;
	view: NodeDebuggerView;
	/** Selecting a node from the nodes or lint view asks for the run view. */
	onOpenView?: (view: NodeDebuggerView) => void;
}

export function NodeDebugger({
	session,
	view,
	onOpenView,
}: Readonly<NodeDebuggerProps>) {
	const openRun = (index: number) => {
		session.selectNode(index);
		onOpenView?.("run");
	};
	switch (view) {
		case "nodes":
			return <NodesView session={session} onSelect={openRun} />;
		case "lint":
			return (
				<LintPanel
					issues={session.lintIssues}
					counts={session.lintCounts}
					onJumpToNode={openRun}
				/>
			);
		case "permissions":
			return <PermissionsView session={session} />;
		case "run":
			return <RunView session={session} />;
	}
}

/** Loading, failure and "nothing built" states; null once nodes are known. */
export function NodeDebuggerStatus({
	session,
	emptyHint,
}: Readonly<{ session: NodeDebuggerSession; emptyHint?: string }>) {
	const { t } = useTranslation("common");
	if (session.isInspecting) {
		return (
			<div className="space-y-3">
				<Skeleton className="h-20 w-full rounded-xl" />
				<Skeleton className="h-48 w-full rounded-xl" />
			</div>
		);
	}
	if (session.error) {
		return (
			<Alert variant="destructive" className="rounded-xl">
				<AlertCircle className="h-4 w-4" />
				<AlertDescription className="flex flex-wrap items-center justify-between gap-3">
					<span className="min-w-0 wrap-break-word">
						{t(
							"inspectionFailedMessage",
							"Couldn't inspect the package: {{message}}",
							{
								message: getErrorMessage(session.error),
							},
						)}
					</span>
					<ReinspectButton session={session} />
				</AlertDescription>
			</Alert>
		);
	}
	if (session.hasTarget && session.nodes.length === 0 && emptyHint) {
		return (
			<div className="flex flex-wrap items-center justify-between gap-3 rounded-xl border border-dashed border-border/60 p-4 text-sm text-muted-foreground">
				<span>{emptyHint}</span>
				<ReinspectButton session={session} />
			</div>
		);
	}
	return null;
}

export function ReinspectButton({
	session,
}: Readonly<{ session: NodeDebuggerSession }>) {
	const { t } = useTranslation("common");
	return (
		<Button
			variant="outline"
			size="sm"
			onClick={session.reinspect}
			disabled={session.isFetching}
		>
			<RefreshCw className={session.isFetching ? "animate-spin" : undefined} />
			{t("reinspect", "Re-inspect")}
		</Button>
	);
}

/** The `/developer/debug` layout: one tab per view. */
export function NodeDebuggerTabs({
	session,
	view,
	onViewChange,
}: Readonly<{
	session: NodeDebuggerSession;
	view: NodeDebuggerView;
	onViewChange: (view: NodeDebuggerView) => void;
}>) {
	const { t } = useTranslation("common");
	const { lintCounts, nodes, selectedNode } = session;
	return (
		<Tabs
			value={view}
			onValueChange={(value) => onViewChange(value as NodeDebuggerView)}
			className="space-y-4"
		>
			<TabsList>
				<TabsTrigger value="run" className="gap-1.5">
					<Play className="h-3.5 w-3.5" />
					{t("debug", "Debug")}
				</TabsTrigger>
				<TabsTrigger value="nodes" className="gap-1.5">
					<Package className="h-3.5 w-3.5" />
					{t("nodesLength", "Nodes ({{length}})", {
						length: nodes.length,
					})}
				</TabsTrigger>
				<TabsTrigger value="lint" className="gap-1.5">
					<ShieldCheck className="h-3.5 w-3.5" />
					{t("lint", "Lint")}
					{lintCounts.errors > 0 ? (
						<Badge
							variant="destructive"
							className="text-[10px] ml-1 px-1.5 py-0 h-4"
						>
							{lintCounts.errors}
						</Badge>
					) : lintCounts.warnings > 0 ? (
						<Badge className="text-[10px] ml-1 px-1.5 py-0 h-4 bg-amber-500/10 text-amber-600 border-amber-500/20">
							{lintCounts.warnings}
						</Badge>
					) : (
						<Badge
							variant="outline"
							className="text-[10px] ml-1 px-1.5 py-0 h-4 text-green-600"
						>
							0
						</Badge>
					)}
				</TabsTrigger>
				{selectedNode && (
					<TabsTrigger value="permissions" className="gap-1.5">
						<Shield className="h-3.5 w-3.5" />
						{t("permissions", "Permissions")}
					</TabsTrigger>
				)}
			</TabsList>
			{(["run", "nodes", "lint", "permissions"] as const).map((tab) => (
				<TabsContent key={tab} value={tab} className="space-y-3">
					<NodeDebugger
						session={session}
						view={tab}
						onOpenView={onViewChange}
					/>
				</TabsContent>
			))}
		</Tabs>
	);
}

function NodesView({
	session,
	onSelect,
}: Readonly<{
	session: NodeDebuggerSession;
	onSelect: (index: number) => void;
}>) {
	const { t } = useTranslation("common");
	return (
		<div className="rounded-xl border border-border/20 bg-card/50 p-4">
			<div className="flex items-center gap-2 mb-3">
				<span className="text-xs font-medium uppercase tracking-widest text-muted-foreground/60">
					{t("packageNodes", "Package Nodes")}
				</span>
				{session.isPackage && (
					<Badge variant="secondary" className="text-[10px]">
						{t("multinode", "Multi-node")}
					</Badge>
				)}
			</div>
			<div className="grid grid-cols-1 sm:grid-cols-2 lg:grid-cols-3 gap-3">
				{session.nodes.map((node, i) => (
					<NodeCard
						key={node.name}
						node={node}
						isSelected={i === session.selectedIndex}
						onSelect={() => onSelect(i)}
					/>
				))}
			</div>
		</div>
	);
}

function PermissionsView({
	session,
}: Readonly<{ session: NodeDebuggerSession }>) {
	const { t } = useTranslation("common");
	const { selectedNode, manifest } = session;
	if (!selectedNode) return null;
	return (
		<>
			<div className="rounded-xl border border-border/20 bg-card/50 p-4 space-y-4">
				<div className="flex items-center gap-2">
					<Shield className="h-3.5 w-3.5 text-muted-foreground/60" />
					<span className="text-xs font-medium uppercase tracking-widest text-muted-foreground/60">
						{t("nodePermissions", "Node Permissions")}
					</span>
					<Badge variant="outline" className="text-[10px]">
						{t("runtimeenforced", "Runtime-enforced")}
					</Badge>
				</div>
				<p className="text-sm text-muted-foreground/70">
					{t(
						"theseCapabilitiesComeFromTheSelectedNodeDefinition",
						"These capabilities come from the selected node definition and are applied during debug execution.",
					)}
				</p>
				<NodePermissionBadges
					permissions={selectedNode.permissions}
					showEmpty
				/>
				<div className="border-t border-border/10" />
				<NodePermissionsDetail permissions={selectedNode.permissions} />
			</div>
			{manifest && (
				<div className="rounded-xl border border-border/20 bg-card/50 p-4 space-y-4">
					<div className="flex items-center gap-2">
						<Shield className="h-3.5 w-3.5 text-muted-foreground/60" />
						<span className="text-xs font-medium uppercase tracking-widest text-muted-foreground/60">
							{t("packageResourceTiers", "Package Resource Tiers")}
						</span>
					</div>
					<PermissionsBadges manifest={manifest} />
					<div className="border-t border-border/10" />
					<PermissionsDetail manifest={manifest} />
				</div>
			)}
		</>
	);
}

function NodePicker({ session }: Readonly<{ session: NodeDebuggerSession }>) {
	const { t } = useTranslation("common");
	const { nodes, selectedIndex, selectNode } = session;
	if (nodes.length < 2) return null;
	return (
		<Select
			value={String(selectedIndex)}
			onValueChange={(value) => selectNode(Number(value))}
		>
			<SelectTrigger
				aria-label={t("selectNodeToRun", "Node to run")}
				className="h-8 w-56 max-w-full"
			>
				<SelectValue />
			</SelectTrigger>
			<SelectContent>
				{nodes.map((node, index) => (
					<SelectItem key={node.name} value={String(index)}>
						{`${index + 1}/${nodes.length} · ${node.friendly_name}`}
					</SelectItem>
				))}
			</SelectContent>
		</Select>
	);
}

function RunView({ session }: Readonly<{ session: NodeDebuggerSession }>) {
	const { t } = useTranslation("common");
	const [outputsExpanded, setOutputsExpanded] = useState(true);
	const {
		selectedNode,
		inputPins,
		outputPins,
		inputValues,
		result,
		running,
		missingModelInput,
	} = session;
	const selectedNodeRequiresModels =
		selectedNode?.permissions.includes("models") ?? false;

	return (
		<>
			{selectedNode && (
				<div className="rounded-xl bg-muted/5 p-4">
					<div className="flex flex-wrap items-center justify-between gap-3">
						<div className="flex min-w-0 items-center gap-3">
							{selectedNode.icon && (
								<span className="text-2xl">{selectedNode.icon}</span>
							)}
							<div className="min-w-0">
								<h2 className="text-lg font-semibold tracking-tight">
									{selectedNode.friendly_name}
								</h2>
								<p className="text-sm text-muted-foreground/70">
									{selectedNode.description}
								</p>
							</div>
						</div>
						<div className="flex flex-wrap items-center gap-2">
							<Badge variant="secondary">{selectedNode.category}</Badge>
							<NodePicker session={session} />
						</div>
					</div>
					<div className="mt-3 flex flex-wrap items-center gap-2">
						<NodePermissionsSummary
							permissions={selectedNode.permissions}
							title={t("runtimePermissions", "Runtime Permissions")}
							description={t(
								"theseCapabilitiesAreGrantedToThisNodeForDebugExecution",
								"These capabilities are granted to this node for debug execution.",
							)}
							className="w-full"
						/>
					</div>
				</div>
			)}

			{selectedNode && (
				<div className="rounded-xl border border-border/20 bg-card/50 p-4 space-y-4">
					<div className="flex items-center gap-2">
						<Shield className="h-3.5 w-3.5 text-muted-foreground/60" />
						<span className="text-xs font-medium uppercase tracking-widest text-muted-foreground/60">
							{t("executionPermissions", "Execution Permissions")}
						</span>
						<Badge variant="outline" className="text-[10px]">
							{t("appliedOnRun", "Applied on Run")}
						</Badge>
					</div>
					<p className="text-sm text-muted-foreground/70">
						{t(
							"theDebugRunnerUsesTheseNodedeclaredCapabilitiesWhenInstantiatingTheWasmSandbox",
							"The debug runner uses these node-declared capabilities when instantiating the WASM sandbox.",
						)}
					</p>
					<NodePermissionsDetail permissions={selectedNode.permissions} />
				</div>
			)}

			<div className="grid grid-cols-1 lg:grid-cols-2 gap-3">
				<div className="rounded-xl border border-border/20 bg-card/50 p-4">
					<div className="flex items-center justify-between mb-3">
						<div className="flex items-center gap-2">
							<span className="text-xs font-medium uppercase tracking-widest text-muted-foreground/60">
								{t("inputPins", "Input Pins")}
							</span>
							<Badge variant="outline" className="text-[10px]">
								{inputPins.length}
							</Badge>
						</div>
						<Button
							size="sm"
							onClick={session.runNode}
							disabled={
								running ||
								!session.wasmPath ||
								!selectedNode ||
								missingModelInput
							}
							className="h-8 rounded-full gap-1.5 px-4"
						>
							{running ? (
								<Loader2 className="h-3.5 w-3.5 animate-spin" />
							) : (
								<Play className="h-3.5 w-3.5" />
							)}
							{t("run", "Run")}
						</Button>
					</div>
					{inputPins.length === 0 ? (
						<p className="text-sm text-muted-foreground/60 text-center py-4">
							{t("noInputPins", "No input pins")}
						</p>
					) : (
						<ScrollArea className="max-h-125">
							<div className="space-y-4 pr-3">
								{selectedNodeRequiresModels && missingModelInput && (
									<div className="rounded-lg border border-amber-500/20 bg-amber-500/5 p-3 text-xs text-amber-700">
										{t(
											"selectAnLlmOrVlmBitForThisNodeBeforeRunningIt",
											"Select an LLM or VLM bit for this node before running it.",
										)}
									</div>
								)}
								{inputPins.map((pin) => (
									<div key={pin.name} className="space-y-1.5">
										<div className="flex items-center justify-between gap-2">
											<Label className="text-sm font-medium">
												{pin.friendly_name}
											</Label>
											<DataTypeBadge dataType={pin.data_type} />
										</div>
										{pin.description && (
											<p className="text-xs text-muted-foreground/60">
												{pin.description}
											</p>
										)}
										<PinInput
											pin={pin}
											value={inputValues[pin.name]}
											onChange={(v) => session.setInputValue(pin.name, v)}
											availableModelBits={session.availableModelBits}
										/>
									</div>
								))}
							</div>
						</ScrollArea>
					)}
				</div>

				<div className="rounded-xl border border-border/20 bg-card/50 p-4">
					<button
						type="button"
						className="flex items-center justify-between w-full mb-3"
						onClick={() => setOutputsExpanded(!outputsExpanded)}
					>
						<div className="flex items-center gap-2">
							<span className="text-xs font-medium uppercase tracking-widest text-muted-foreground/60">
								{t("output", "Output")}
							</span>
							{result &&
								(result.error ? (
									<AlertCircle className="h-3.5 w-3.5 text-destructive" />
								) : (
									<CheckCircle2 className="h-3.5 w-3.5 text-green-500" />
								))}
						</div>
						{outputsExpanded ? (
							<ChevronDown className="h-4 w-4 text-muted-foreground/60" />
						) : (
							<ChevronRight className="h-4 w-4 text-muted-foreground/60" />
						)}
					</button>
					{outputsExpanded &&
						(!result ? (
							<div className="text-center py-8">
								<Play className="h-8 w-8 text-muted-foreground/20 mx-auto mb-2" />
								<p className="text-sm text-muted-foreground/60">
									{t(
										"runTheNodeToSeeOutputValues",
										"Run the node to see output values",
									)}
								</p>
							</div>
						) : result.error ? (
							<div className="bg-destructive/10 text-destructive rounded-lg p-3 text-sm">
								<p className="font-medium mb-1">{t("error", "Error")}</p>
								<pre className="text-xs whitespace-pre-wrap font-mono">
									{result.error}
								</pre>
							</div>
						) : (
							<ScrollArea className="max-h-125">
								<div className="space-y-4 pr-3">
									{outputPins.map((pin) => (
										<OutputValue
											key={pin.name}
											name={pin.friendly_name}
											value={result.outputs[pin.name] ?? "(no value)"}
										/>
									))}
									{Object.keys(result.outputs).length === 0 && (
										<p className="text-sm text-muted-foreground/60 text-center py-4">
											{t("noOutputValues", "No output values")}
										</p>
									)}
									{result.activate_exec.length > 0 && (
										<div className="pt-2">
											<div className="border-t border-border/10 mb-3" />
											<Label className="text-xs text-muted-foreground/60">
												{t(
													"activatedExecutionPins",
													"Activated Execution Pins",
												)}
											</Label>
											<div className="flex gap-1 mt-1">
												{result.activate_exec.map((e) => (
													<Badge key={e} variant="outline" className="text-xs">
														{e}
													</Badge>
												))}
											</div>
										</div>
									)}
								</div>
							</ScrollArea>
						))}
				</div>
			</div>
		</>
	);
}
