"use client";

import { invoke } from "@tauri-apps/api/core";
import { open } from "@tauri-apps/plugin-dialog";
import {
	Badge,
	Button,
	Input,
	Label,
	ScrollArea,
	Select,
	SelectContent,
	SelectItem,
	SelectTrigger,
	SelectValue,
	Switch,
	Tabs,
	TabsContent,
	TabsList,
	TabsTrigger,
	Textarea,
	Tooltip,
	TooltipContent,
	TooltipTrigger,
	cn,
} from "@tm9657/flow-like-ui";
import type {
	PackageInspection,
	WasmExecutionResult,
	WasmNodeDefinition,
	WasmPinDefinition,
} from "@tm9657/flow-like-ui/lib/schema/developer";
import type { PackageManifest } from "@tm9657/flow-like-ui/lib/schema/wasm";
import { AnimatePresence, motion } from "framer-motion";
import {
	AlertCircle,
	ArrowLeft,
	Bug,
	CheckCircle2,
	ChevronDown,
	ChevronRight,
	Clock,
	FileCode2,
	FolderOpen,
	Globe,
	HardDrive,
	Loader2,
	Lock,
	Package,
	Play,
	Search,
	Shield,
	Sparkles,
	Zap,
} from "lucide-react";
import { useRouter, useSearchParams } from "next/navigation";
import { Suspense, useCallback, useEffect, useMemo, useState } from "react";
import { toast } from "sonner";

function PinInput({
	pin,
	value,
	onChange,
}: {
	pin: WasmPinDefinition;
	value: unknown;
	onChange: (val: unknown) => void;
}) {
	if (pin.valid_values && pin.valid_values.length > 0) {
		return (
			<Select value={String(value ?? "")} onValueChange={(v) => onChange(v)}>
				<SelectTrigger className="h-9">
					<SelectValue placeholder="Select..." />
				</SelectTrigger>
				<SelectContent>
					{pin.valid_values.map((v) => (
						<SelectItem key={v} value={v}>
							{v}
						</SelectItem>
					))}
				</SelectContent>
			</Select>
		);
	}

	switch (pin.data_type) {
		case "Boolean":
			return (
				<div className="flex items-center gap-2">
					<Switch
						checked={Boolean(value)}
						onCheckedChange={(v) => onChange(v)}
					/>
					<span className="text-sm text-muted-foreground/70">
						{value ? "true" : "false"}
					</span>
				</div>
			);
		case "Integer":
			return (
				<Input
					type="number"
					step={pin.range ? 1 : undefined}
					min={pin.range?.[0]}
					max={pin.range?.[1]}
					value={String(value ?? 0)}
					onChange={(e) => onChange(Number.parseInt(e.target.value) || 0)}
					className="h-9"
				/>
			);
		case "Float":
			return (
				<Input
					type="number"
					step={0.01}
					min={pin.range?.[0]}
					max={pin.range?.[1]}
					value={String(value ?? 0)}
					onChange={(e) => onChange(Number.parseFloat(e.target.value) || 0)}
					className="h-9"
				/>
			);
		case "Struct":
			return (
				<Textarea
					value={
						typeof value === "string"
							? value
							: JSON.stringify(value ?? {}, null, 2)
					}
					onChange={(e) => {
						try {
							onChange(JSON.parse(e.target.value));
						} catch {
							onChange(e.target.value);
						}
					}}
					rows={4}
					className="font-mono text-xs"
					placeholder="{}"
				/>
			);
		default:
			return (
				<Input
					value={String(value ?? "")}
					onChange={(e) => onChange(e.target.value)}
					className="h-9"
					placeholder={`Enter ${pin.data_type.toLowerCase()} value...`}
				/>
			);
	}
}

function DataTypeBadge({ dataType }: { dataType: string }) {
	const colorMap: Record<string, string> = {
		String: "bg-green-500/10 text-green-700 dark:text-green-400",
		Integer: "bg-blue-500/10 text-blue-700 dark:text-blue-400",
		Float: "bg-cyan-500/10 text-cyan-700 dark:text-cyan-400",
		Boolean: "bg-amber-500/10 text-amber-700 dark:text-amber-400",
		Struct: "bg-purple-500/10 text-purple-700 dark:text-purple-400",
		Execution: "bg-red-500/10 text-red-700 dark:text-red-400",
		Date: "bg-orange-500/10 text-orange-700 dark:text-orange-400",
		PathBuf: "bg-slate-500/10 text-slate-700 dark:text-slate-400",
		Byte: "bg-gray-500/10 text-gray-700 dark:text-gray-400",
		Generic: "bg-pink-500/10 text-pink-700 dark:text-pink-400",
	};

	return (
		<Badge
			variant="outline"
			className={cn("text-[10px] font-mono", colorMap[dataType])}
		>
			{dataType}
		</Badge>
	);
}

function OutputValue({ name, value }: { name: string; value: unknown }) {
	const formatted =
		typeof value === "object" ? JSON.stringify(value, null, 2) : String(value);

	return (
		<div className="space-y-1">
			<Label className="text-xs font-medium text-muted-foreground/70">
				{name}
			</Label>
			<pre className="bg-muted/30 rounded-lg p-3 text-xs font-mono whitespace-pre-wrap break-all">
				{formatted}
			</pre>
		</div>
	);
}

function getDefaultValue(pin: WasmPinDefinition): unknown {
	if (pin.default_value !== undefined && pin.default_value !== null) {
		return pin.default_value;
	}
	switch (pin.data_type) {
		case "Boolean":
			return false;
		case "Integer":
			return 0;
		case "Float":
			return 0.0;
		case "Struct":
			return {};
		default:
			return "";
	}
}

function PermissionsBadges({ manifest }: { manifest: PackageManifest }) {
	const p = manifest.permissions;
	return (
		<div className="flex flex-wrap gap-1.5">
			<Tooltip>
				<TooltipTrigger>
					<Badge variant="outline" className="gap-1 text-xs">
						<HardDrive className="h-3 w-3" />
						{p.memory}
					</Badge>
				</TooltipTrigger>
				<TooltipContent>Memory tier</TooltipContent>
			</Tooltip>
			<Tooltip>
				<TooltipTrigger>
					<Badge variant="outline" className="gap-1 text-xs">
						<Clock className="h-3 w-3" />
						{p.timeout}
					</Badge>
				</TooltipTrigger>
				<TooltipContent>Timeout tier</TooltipContent>
			</Tooltip>
			{p.network?.httpEnabled && (
				<Badge
					variant="outline"
					className="gap-1 text-xs text-amber-600 border-amber-500/30"
				>
					<Globe className="h-3 w-3" />
					HTTP
				</Badge>
			)}
			{(p.filesystem?.nodeStorage || p.filesystem?.userStorage) && (
				<Badge
					variant="outline"
					className="gap-1 text-xs text-blue-600 border-blue-500/30"
				>
					<HardDrive className="h-3 w-3" />
					Storage
				</Badge>
			)}
			{p.streaming && (
				<Badge variant="outline" className="gap-1 text-xs">
					<Zap className="h-3 w-3" />
					Streaming
				</Badge>
			)}
			{p.models && (
				<Badge
					variant="outline"
					className="gap-1 text-xs text-purple-600 border-purple-500/30"
				>
					<Sparkles className="h-3 w-3" />
					Models
				</Badge>
			)}
			{p.variables && (
				<Badge variant="outline" className="gap-1 text-xs">
					Variables
				</Badge>
			)}
			{p.cache && (
				<Badge variant="outline" className="gap-1 text-xs">
					Cache
				</Badge>
			)}
			{p.a2ui && (
				<Badge variant="outline" className="gap-1 text-xs">
					A2UI
				</Badge>
			)}
			{p.oauthScopes?.length > 0 && (
				<Tooltip>
					<TooltipTrigger>
						<Badge
							variant="outline"
							className="gap-1 text-xs text-orange-600 border-orange-500/30"
						>
							<Lock className="h-3 w-3" />
							OAuth ({p.oauthScopes.length})
						</Badge>
					</TooltipTrigger>
					<TooltipContent>
						{p.oauthScopes
							.map((s) => `${s.provider}: ${s.scopes.join(", ")}`)
							.join("\n")}
					</TooltipContent>
				</Tooltip>
			)}
		</div>
	);
}

function PermissionsDetail({ manifest }: { manifest: PackageManifest }) {
	const p = manifest.permissions;
	return (
		<div className="grid grid-cols-1 sm:grid-cols-2 gap-4 text-sm">
			<div className="space-y-2">
				<h4 className="font-medium flex items-center gap-1.5">
					<HardDrive className="h-3.5 w-3.5" /> Resources
				</h4>
				<div className="space-y-1 text-muted-foreground/70">
					<div className="flex justify-between">
						<span>Memory</span>
						<span className="font-mono">{p.memory}</span>
					</div>
					<div className="flex justify-between">
						<span>Timeout</span>
						<span className="font-mono">{p.timeout}</span>
					</div>
				</div>
			</div>
			<div className="space-y-2">
				<h4 className="font-medium flex items-center gap-1.5">
					<Globe className="h-3.5 w-3.5" /> Network
				</h4>
				<div className="space-y-1 text-muted-foreground/70">
					<div className="flex justify-between">
						<span>HTTP</span>
						<span>{p.network?.httpEnabled ? "Yes" : "No"}</span>
					</div>
					{p.network?.allowedHosts?.length > 0 && (
						<div>
							<span className="text-xs">Allowed hosts:</span>
							<div className="flex flex-wrap gap-1 mt-0.5">
								{p.network.allowedHosts.map((h) => (
									<Badge key={h} variant="outline" className="text-[10px]">
										{h}
									</Badge>
								))}
							</div>
						</div>
					)}
				</div>
			</div>
			<div className="space-y-2">
				<h4 className="font-medium flex items-center gap-1.5">
					<HardDrive className="h-3.5 w-3.5" /> Filesystem
				</h4>
				<div className="space-y-1 text-muted-foreground/70">
					{(
						[
							["Node Storage", p.filesystem?.nodeStorage],
							["User Storage", p.filesystem?.userStorage],
							["Upload Dir", p.filesystem?.uploadDir],
							["Cache Dir", p.filesystem?.cacheDir],
						] as const
					).map(([label, enabled]) => (
						<div key={label} className="flex justify-between">
							<span>{label}</span>
							<span>{enabled ? "Yes" : "No"}</span>
						</div>
					))}
				</div>
			</div>
			<div className="space-y-2">
				<h4 className="font-medium flex items-center gap-1.5">
					<Zap className="h-3.5 w-3.5" /> Capabilities
				</h4>
				<div className="space-y-1 text-muted-foreground/70">
					{(
						[
							["Variables", p.variables],
							["Cache", p.cache],
							["Streaming", p.streaming],
							["A2UI", p.a2ui],
							["Models/LLM", p.models],
						] as const
					).map(([label, enabled]) => (
						<div key={label} className="flex justify-between">
							<span>{label}</span>
							<span>{enabled ? "Yes" : "No"}</span>
						</div>
					))}
				</div>
			</div>
		</div>
	);
}

function NodeCard({
	node,
	isSelected,
	onSelect,
}: {
	node: WasmNodeDefinition;
	isSelected: boolean;
	onSelect: () => void;
}) {
	const inputCount = node.pins.filter(
		(p) => p.pin_type === "Input" && p.data_type !== "Execution",
	).length;
	const outputCount = node.pins.filter(
		(p) => p.pin_type === "Output" && p.data_type !== "Execution",
	).length;

	return (
		<button
			type="button"
			onClick={onSelect}
			className={cn(
				"w-full text-left rounded-lg border p-3 transition-colors",
				isSelected
					? "border-primary/40 bg-primary/5"
					: "border-border/20 hover:bg-muted/10",
			)}
		>
			<div className="flex items-start justify-between gap-2">
				<div className="min-w-0 flex-1">
					<div className="flex items-center gap-2">
						{node.icon && <span className="text-lg">{node.icon}</span>}
						<span className="font-medium text-sm truncate">
							{node.friendly_name}
						</span>
					</div>
					{node.description && (
						<p className="text-xs text-muted-foreground/60 mt-1 line-clamp-2">
							{node.description}
						</p>
					)}
				</div>
				<Badge variant="secondary" className="text-[10px] shrink-0">
					{node.category}
				</Badge>
			</div>
			<div className="flex items-center gap-2 mt-2">
				<span className="text-[10px] text-muted-foreground/60">
					{inputCount} in / {outputCount} out
				</span>
				{node.long_running && (
					<Badge variant="outline" className="text-[10px]">
						Long Running
					</Badge>
				)}
			</div>
		</button>
	);
}

function initInputDefaults(node: WasmNodeDefinition): Record<string, unknown> {
	const defaults: Record<string, unknown> = {};
	for (const pin of node.pins) {
		if (pin.pin_type === "Input" && pin.data_type !== "Execution") {
			defaults[pin.name] = getDefaultValue(pin);
		}
	}
	return defaults;
}

function DebugPageContent() {
	const router = useRouter();
	const searchParams = useSearchParams();
	const initialProject = searchParams.get("project") ?? "";
	const [wasmPath, setWasmPath] = useState("");
	const [nodes, setNodes] = useState<WasmNodeDefinition[]>([]);
	const [manifest, setManifest] = useState<PackageManifest | null>(null);
	const [isPackage, setIsPackage] = useState(false);
	const [selectedNodeIndex, setSelectedNodeIndex] = useState(0);
	const [loading, setLoading] = useState(false);
	const [running, setRunning] = useState(false);
	const [inputValues, setInputValues] = useState<Record<string, unknown>>({});
	const [result, setResult] = useState<WasmExecutionResult | null>(null);
	const [outputsExpanded, setOutputsExpanded] = useState(true);
	const [activeTab, setActiveTab] = useState("debug");

	const selectedNode = nodes[selectedNodeIndex] ?? null;

	const inputPins = useMemo(
		() =>
			selectedNode?.pins.filter(
				(p) => p.pin_type === "Input" && p.data_type !== "Execution",
			) ?? [],
		[selectedNode],
	);

	const outputPins = useMemo(
		() =>
			selectedNode?.pins.filter(
				(p) => p.pin_type === "Output" && p.data_type !== "Execution",
			) ?? [],
		[selectedNode],
	);

	const selectWasm = useCallback(async () => {
		const selected = await open({
			multiple: false,
			filters: [{ name: "WASM Files", extensions: ["wasm"] }],
		});
		if (selected) setWasmPath(selected);
	}, []);

	const inspectNode = useCallback(async () => {
		if (!wasmPath) return;
		setLoading(true);
		setResult(null);
		setSelectedNodeIndex(0);
		try {
			const defs = await invoke<WasmNodeDefinition[]>(
				"developer_inspect_node",
				{ wasmPath },
			);
			setNodes(defs);
			setIsPackage(defs.length > 1);
			setManifest(null);
			if (defs.length > 0) setInputValues(initInputDefaults(defs[0]));
		} catch (err) {
			toast.error(`Failed to inspect: ${err}`);
		} finally {
			setLoading(false);
		}
	}, [wasmPath]);

	const inspectProject = useCallback(async (projectPath?: string) => {
		const target =
			projectPath ?? (await open({ directory: true, multiple: false }));
		if (!target) return;
		setLoading(true);
		setResult(null);
		setSelectedNodeIndex(0);
		try {
			const inspection = await invoke<PackageInspection>(
				"developer_inspect_package",
				{ projectPath: target },
			);
			setNodes(inspection.nodes);
			setManifest(inspection.manifest);
			setIsPackage(inspection.isPackage);
			setWasmPath(inspection.wasmPath);
			if (inspection.nodes.length > 0)
				setInputValues(initInputDefaults(inspection.nodes[0]));
		} catch (err) {
			toast.error(`Failed to inspect project: ${err}`);
		} finally {
			setLoading(false);
		}
	}, []);

	useEffect(() => {
		if (initialProject) inspectProject(initialProject);
	}, [initialProject, inspectProject]);

	const selectNode = useCallback(
		(index: number) => {
			setSelectedNodeIndex(index);
			setResult(null);
			const node = nodes[index];
			if (node) setInputValues(initInputDefaults(node));
		},
		[nodes],
	);

	const runNode = useCallback(async () => {
		if (!wasmPath || !selectedNode) return;
		setRunning(true);
		try {
			const res = await invoke<WasmExecutionResult>("developer_run_node", {
				input: {
					wasmPath,
					inputs: inputValues,
					nodeName: selectedNode.name,
				},
			});
			setResult(res);
			toast[res.error ? "error" : "success"](
				res.error ? `Node error: ${res.error}` : "Node executed successfully",
			);
		} catch (err) {
			toast.error(`Execution failed: ${err}`);
		} finally {
			setRunning(false);
		}
	}, [wasmPath, selectedNode, inputValues]);

	const setInputValue = useCallback((name: string, value: unknown) => {
		setInputValues((prev) => ({ ...prev, [name]: value }));
	}, []);

	return (
		<div className="flex flex-col h-full">
			<div className="flex items-center gap-4 pb-4 border-b border-border/10">
				<Button
					variant="ghost"
					size="icon"
					className="h-8 w-8 rounded-full text-muted-foreground/60 hover:text-foreground/80 hover:bg-muted/30"
					onClick={() => router.push("/store/packages?tab=projects")}
				>
					<ArrowLeft className="h-4 w-4" />
				</Button>
				<div>
					<div className="flex items-center gap-2">
						<Bug className="h-4 w-4 text-muted-foreground/60" />
						<h1 className="text-2xl font-semibold tracking-tight">
							Debug Node
						</h1>
					</div>
					<p className="text-sm text-muted-foreground/70">
						Inspect package nodes, permissions, and test execution
					</p>
				</div>
			</div>

			<div className="flex-1 overflow-y-auto py-4 space-y-4">
				<div className="rounded-xl bg-muted/10 border border-border/20 p-3">
					<div className="flex items-center gap-3">
						<FileCode2 className="h-4 w-4 text-muted-foreground/60 shrink-0" />
						<Input
							value={wasmPath}
							onChange={(e) => setWasmPath(e.target.value)}
							placeholder="Path to .wasm file..."
							className="flex-1 h-9 rounded-full bg-muted/30 border-transparent focus:border-border/40 focus:bg-muted/50"
						/>
						<Button
							variant="ghost"
							size="sm"
							onClick={selectWasm}
							className="h-8 rounded-full text-muted-foreground/60 hover:text-foreground/80 hover:bg-muted/30 gap-1.5 px-3"
						>
							<FolderOpen className="h-3.5 w-3.5" />
							WASM
						</Button>
						<Button
							variant="ghost"
							size="sm"
							onClick={() => inspectProject()}
							className="h-8 rounded-full text-muted-foreground/60 hover:text-foreground/80 hover:bg-muted/30 gap-1.5 px-3"
						>
							<Package className="h-3.5 w-3.5" />
							Project
						</Button>
						<Button
							size="sm"
							onClick={inspectNode}
							disabled={!wasmPath || loading}
							className="h-8 rounded-full gap-1.5 px-4"
						>
							{loading ? (
								<Loader2 className="h-3.5 w-3.5 animate-spin" />
							) : (
								<>
									<Search className="h-3.5 w-3.5" />
									Inspect
								</>
							)}
						</Button>
					</div>
				</div>

				<AnimatePresence mode="wait">
					{nodes.length > 0 && (
						<motion.div
							key="package-view"
							initial={{ opacity: 0 }}
							animate={{ opacity: 1 }}
							exit={{ opacity: 0 }}
						>
							<Tabs
								value={activeTab}
								onValueChange={setActiveTab}
								className="space-y-4"
							>
								<TabsList>
									<TabsTrigger value="debug" className="gap-1.5">
										<Play className="h-3.5 w-3.5" />
										Debug
									</TabsTrigger>
									<TabsTrigger value="nodes" className="gap-1.5">
										<Package className="h-3.5 w-3.5" />
										Nodes ({nodes.length})
									</TabsTrigger>
									{manifest && (
										<TabsTrigger value="permissions" className="gap-1.5">
											<Shield className="h-3.5 w-3.5" />
											Permissions
										</TabsTrigger>
									)}
								</TabsList>

								<TabsContent value="nodes" className="space-y-3">
									<div className="rounded-xl border border-border/20 bg-card/50 p-4">
										<div className="flex items-center gap-2 mb-3">
											<span className="text-xs font-medium uppercase tracking-widest text-muted-foreground/60">
												Package Nodes
											</span>
											{isPackage && (
												<Badge variant="secondary" className="text-[10px]">
													Multi-node
												</Badge>
											)}
										</div>
										<div className="grid grid-cols-1 sm:grid-cols-2 lg:grid-cols-3 gap-3">
											{nodes.map((node, i) => (
												<NodeCard
													key={node.name}
													node={node}
													isSelected={i === selectedNodeIndex}
													onSelect={() => {
														selectNode(i);
														setActiveTab("debug");
													}}
												/>
											))}
										</div>
									</div>
								</TabsContent>

								{manifest && (
									<TabsContent value="permissions" className="space-y-3">
										<div className="rounded-xl border border-border/20 bg-card/50 p-4 space-y-4">
											<div className="flex items-center gap-2">
												<Shield className="h-3.5 w-3.5 text-muted-foreground/60" />
												<span className="text-xs font-medium uppercase tracking-widest text-muted-foreground/60">
													Package Permissions
												</span>
											</div>
											<PermissionsBadges manifest={manifest} />
											<div className="border-t border-border/10" />
											<PermissionsDetail manifest={manifest} />
										</div>
									</TabsContent>
								)}

								<TabsContent value="debug" className="space-y-3">
									{selectedNode && (
										<div className="rounded-xl bg-muted/5 p-4">
											<div className="flex items-center justify-between">
												<div className="flex items-center gap-3">
													{selectedNode.icon && (
														<span className="text-2xl">
															{selectedNode.icon}
														</span>
													)}
													<div>
														<h2 className="text-lg font-semibold tracking-tight">
															{selectedNode.friendly_name}
														</h2>
														<p className="text-sm text-muted-foreground/70">
															{selectedNode.description}
														</p>
													</div>
												</div>
												<div className="flex items-center gap-2">
													<Badge variant="secondary">
														{selectedNode.category}
													</Badge>
													{nodes.length > 1 && (
														<Badge variant="outline" className="text-xs">
															{selectedNodeIndex + 1}/{nodes.length}
														</Badge>
													)}
												</div>
											</div>
										</div>
									)}

									<div className="grid grid-cols-1 lg:grid-cols-2 gap-3">
										<div className="rounded-xl border border-border/20 bg-card/50 p-4">
											<div className="flex items-center justify-between mb-3">
												<div className="flex items-center gap-2">
													<span className="text-xs font-medium uppercase tracking-widest text-muted-foreground/60">
														Input Pins
													</span>
													<Badge variant="outline" className="text-[10px]">
														{inputPins.length}
													</Badge>
												</div>
												<Button
													size="sm"
													onClick={runNode}
													disabled={running || !wasmPath}
													className="h-8 rounded-full gap-1.5 px-4"
												>
													{running ? (
														<Loader2 className="h-3.5 w-3.5 animate-spin" />
													) : (
														<Play className="h-3.5 w-3.5" />
													)}
													Run
												</Button>
											</div>
											{inputPins.length === 0 ? (
												<p className="text-sm text-muted-foreground/60 text-center py-4">
													No input pins
												</p>
											) : (
												<ScrollArea className="max-h-125">
													<div className="space-y-4 pr-3">
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
																	onChange={(v) => setInputValue(pin.name, v)}
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
														Output
													</span>
													{result && (
														<>
															{result.error ? (
																<AlertCircle className="h-3.5 w-3.5 text-destructive" />
															) : (
																<CheckCircle2 className="h-3.5 w-3.5 text-green-500" />
															)}
														</>
													)}
												</div>
												{outputsExpanded ? (
													<ChevronDown className="h-4 w-4 text-muted-foreground/60" />
												) : (
													<ChevronRight className="h-4 w-4 text-muted-foreground/60" />
												)}
											</button>
											{outputsExpanded && (
												<>
													{!result ? (
														<div className="text-center py-8">
															<Play className="h-8 w-8 text-muted-foreground/20 mx-auto mb-2" />
															<p className="text-sm text-muted-foreground/60">
																Run the node to see output values
															</p>
														</div>
													) : result.error ? (
														<div className="bg-destructive/10 text-destructive rounded-lg p-3 text-sm">
															<p className="font-medium mb-1">Error</p>
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
																		value={
																			result.outputs[pin.name] ?? "(no value)"
																		}
																	/>
																))}
																{Object.keys(result.outputs).length === 0 && (
																	<p className="text-sm text-muted-foreground/60 text-center py-4">
																		No output values
																	</p>
																)}
																{result.activate_exec.length > 0 && (
																	<div className="pt-2">
																		<div className="border-t border-border/10 mb-3" />
																		<Label className="text-xs text-muted-foreground/60">
																			Activated Execution Pins
																		</Label>
																		<div className="flex gap-1 mt-1">
																			{result.activate_exec.map((e) => (
																				<Badge
																					key={e}
																					variant="outline"
																					className="text-xs"
																				>
																					{e}
																				</Badge>
																			))}
																		</div>
																	</div>
																)}
															</div>
														</ScrollArea>
													)}
												</>
											)}
										</div>
									</div>
								</TabsContent>
							</Tabs>
						</motion.div>
					)}
				</AnimatePresence>
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
