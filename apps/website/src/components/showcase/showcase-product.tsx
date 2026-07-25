import { FlowNodeShell } from "@flow-like/flow-like-ui/components/flow/flow-node-shell";
import { FlowPilotInput } from "@flow-like/flow-like-ui/components/flowpilot/FlowPilotInput";
import { FlowPilotBubbleOrb } from "@flow-like/flow-like-ui/components/global-chat/flowpilot-bubble-orb";
import { Badge } from "@flow-like/flow-like-ui/components/ui/badge";
import { Button } from "@flow-like/flow-like-ui/components/ui/button";
import {
	Card,
	CardContent,
	CardDescription,
	CardHeader,
	CardTitle,
} from "@flow-like/flow-like-ui/components/ui/card";
import { Input } from "@flow-like/flow-like-ui/components/ui/input";
import { Progress } from "@flow-like/flow-like-ui/components/ui/progress";
import { ScrollArea } from "@flow-like/flow-like-ui/components/ui/scroll-area";
import { Switch } from "@flow-like/flow-like-ui/components/ui/switch";
import {
	Table,
	TableBody,
	TableCell,
	TableHead,
	TableHeader,
	TableRow,
} from "@flow-like/flow-like-ui/components/ui/table";
import {
	Tabs,
	TabsContent,
	TabsList,
	TabsTrigger,
} from "@flow-like/flow-like-ui/components/ui/tabs";
import { cn } from "@flow-like/flow-like-ui/lib/utils";
import {
	type FormEvent,
	type ReactNode,
	useCallback,
	useEffect,
	useId,
	useMemo,
	useRef,
	useState,
} from "react";

export type ShowcaseProductVariant =
	| "workflow"
	| "runs"
	| "data"
	| "catalog"
	| "prototype";

export interface ShowcaseProductProps {
	variant: ShowcaseProductVariant;
	presentation?: "full" | "embedded" | "compact";
	className?: string;
}

type IconName =
	| "workflow"
	| "play"
	| "check"
	| "clock"
	| "database"
	| "search"
	| "package"
	| "sparkles"
	| "form"
	| "save"
	| "arrow";

const iconPaths: Record<IconName, ReactNode> = {
	workflow: (
		<>
			<rect x="3" y="5" width="6" height="5" rx="1.5" />
			<rect x="15" y="14" width="6" height="5" rx="1.5" />
			<path d="M9 7.5h3a3 3 0 0 1 3 3V14M12 10.5l3 3 3-3" />
		</>
	),
	play: <path d="m8 5 11 7-11 7V5Z" />,
	check: <path d="m5 12 4 4L19 6" />,
	clock: (
		<>
			<circle cx="12" cy="12" r="9" />
			<path d="M12 7v5l3 2" />
		</>
	),
	database: (
		<>
			<ellipse cx="12" cy="5" rx="8" ry="3" />
			<path d="M4 5v6c0 1.7 3.6 3 8 3s8-1.3 8-3V5M4 11v6c0 1.7 3.6 3 8 3s8-1.3 8-3v-6" />
		</>
	),
	search: (
		<>
			<circle cx="10.5" cy="10.5" r="6.5" />
			<path d="m16 16 4 4" />
		</>
	),
	package: (
		<>
			<path d="m12 3 8 4.5v9L12 21l-8-4.5v-9L12 3Z" />
			<path d="m4 7.5 8 4.5 8-4.5M12 12v9" />
		</>
	),
	sparkles: (
		<>
			<path d="M12 3c.6 3.2 2.3 4.9 5.5 5.5C14.3 9.1 12.6 10.8 12 14c-.6-3.2-2.3-4.9-5.5-5.5C9.7 7.9 11.4 6.2 12 3Z" />
			<path d="M18.5 14.5c.3 1.6 1.1 2.4 2.5 2.7-1.4.3-2.2 1.1-2.5 2.8-.3-1.7-1.1-2.5-2.5-2.8 1.4-.3 2.2-1.1 2.5-2.7Z" />
		</>
	),
	form: (
		<>
			<rect x="5" y="3" width="14" height="18" rx="2" />
			<path d="M8 8h8M8 12h8M8 16h5" />
		</>
	),
	save: (
		<>
			<path d="M5 3h12l3 3v15H4V3h1Z" />
			<path d="M8 3v6h8V3M8 21v-7h8v7" />
		</>
	),
	arrow: <path d="M5 12h14m-5-5 5 5-5 5" />,
};

function Icon({
	name,
	className,
}: Readonly<{ name: IconName; className?: string }>) {
	return (
		<svg
			aria-hidden="true"
			className={cn("size-4", className)}
			fill="none"
			stroke="currentColor"
			strokeLinecap="round"
			strokeLinejoin="round"
			strokeWidth="1.7"
			viewBox="0 0 24 24"
		>
			{iconPaths[name]}
		</svg>
	);
}

function useReducedMotion() {
	const [reduced, setReduced] = useState(false);

	useEffect(() => {
		const query = window.matchMedia("(prefers-reduced-motion: reduce)");
		const update = () => setReduced(query.matches);
		update();
		query.addEventListener?.("change", update);
		return () => query.removeEventListener?.("change", update);
	}, []);

	return reduced;
}

interface WorkflowNodeDefinition {
	id: string;
	kind: string;
	name: string;
	detail: string;
	icon: IconName;
	tone: string;
}

interface FlowPilotChange {
	node: WorkflowNodeDefinition;
	summary: string;
}

const workflowNodes: WorkflowNodeDefinition[] = [
	{
		id: "trigger",
		kind: "Trigger",
		name: "Incoming ticket",
		detail: "Webhook · typed event",
		icon: "play",
		tone: "bg-sky-500/15 text-sky-300 ring-sky-500/25",
	},
	{
		id: "classify",
		kind: "AI",
		name: "Classify intent",
		detail: "Local model · JSON",
		icon: "sparkles",
		tone: "bg-violet-500/15 text-violet-300 ring-violet-500/25",
	},
	{
		id: "route",
		kind: "Logic",
		name: "Route priority",
		detail: "3 governed branches",
		icon: "workflow",
		tone: "bg-amber-500/15 text-amber-300 ring-amber-500/25",
	},
	{
		id: "respond",
		kind: "Action",
		name: "Draft response",
		detail: "Knowledge base · email",
		icon: "form",
		tone: "bg-emerald-500/15 text-emerald-300 ring-emerald-500/25",
	},
];

function flowPilotChangeFor(prompt: string): FlowPilotChange {
	const normalized = prompt.toLowerCase();
	if (normalized.includes("slack") || normalized.includes("notify")) {
		return {
			node: {
				id: "pilot-slack",
				kind: "FlowPilot",
				name: "Notify in Slack",
				detail: "#support-escalations",
				icon: "sparkles",
				tone: "bg-violet-500/15 text-violet-300 ring-violet-500/25",
			},
			summary: "Added a Slack notification after the response is drafted.",
		};
	}
	if (
		normalized.includes("approve") ||
		normalized.includes("approval") ||
		normalized.includes("review")
	) {
		return {
			node: {
				id: "pilot-approval",
				kind: "FlowPilot",
				name: "Manager approval",
				detail: "Review gate · then send",
				icon: "check",
				tone: "bg-amber-500/15 text-amber-300 ring-amber-500/25",
			},
			summary: "Added a human review gate before the response is sent.",
		};
	}
	if (normalized.includes("email") || normalized.includes("reply")) {
		return {
			node: {
				id: "pilot-email",
				kind: "FlowPilot",
				name: "Send audited reply",
				detail: "Branded email · audited",
				icon: "form",
				tone: "bg-emerald-500/15 text-emerald-300 ring-emerald-500/25",
			},
			summary: "Added an audited email action to the workflow.",
		};
	}
	return {
		node: {
			id: "pilot-generated",
			kind: "FlowPilot",
			name: "Generated action",
			detail: "Created from your instruction",
			icon: "sparkles",
			tone: "bg-violet-500/15 text-violet-300 ring-violet-500/25",
		},
		summary: "Added a new governed action from your instruction.",
	};
}

interface DemoFlowPilotProps {
	coach: string;
	placeholder: string;
	result?: string | null;
	onApply: (instruction: string) => void;
	onUndo?: () => void;
	compact?: boolean;
}

function DemoFlowPilot({
	coach,
	placeholder,
	result,
	onApply,
	onUndo,
	compact = false,
}: Readonly<DemoFlowPilotProps>) {
	const [open, setOpen] = useState(false);
	const openerRef = useRef<HTMLButtonElement>(null);

	useEffect(() => {
		if (!open) return;
		const onKeyDown = (event: globalThis.KeyboardEvent) => {
			if (event.key !== "Escape") return;
			setOpen(false);
			openerRef.current?.focus();
		};
		document.addEventListener("keydown", onKeyDown);
		return () => document.removeEventListener("keydown", onKeyDown);
	}, [open]);

	const close = () => {
		setOpen(false);
		requestAnimationFrame(() => openerRef.current?.focus());
	};

	return (
		<div className="sp-pilot-anchor absolute bottom-3 right-3 z-30">
			{open ? (
				<section
					aria-label="FlowPilot assistant"
					className="sp-pilot-panel m-0 overflow-hidden rounded-xl border border-border bg-card p-0 text-card-foreground shadow-2xl"
				>
					<div className="flex items-center gap-2 border-b border-border/60 bg-muted/45 px-3 py-2">
						<FlowPilotBubbleOrb
							aria-label="Close FlowPilot"
							className="-my-1 size-9"
							onClick={close}
							title="Close FlowPilot"
						/>
						<div className="min-w-0 flex-1">
							<p className="text-xs font-semibold">FlowPilot</p>
							<p className="sp-shell-detail truncate text-[9px] text-muted-foreground">
								Edits stay inside this live preview
							</p>
						</div>
						<Badge className="text-[8px]" variant="outline">
							Local
						</Badge>
					</div>
					<div className="space-y-2.5 p-3">
						<FlowPilotInput
							className="[&_button]:size-9 [&_textarea]:min-h-9 [&_textarea]:bg-background/80 [&_textarea]:text-xs"
							mode="chat"
							onSubmit={onApply}
							placeholder={placeholder}
						/>
						{result && (
							<output
								aria-live="polite"
								className="block rounded-md border border-emerald-500/20 bg-emerald-500/10 px-2.5 py-2 text-[10px] leading-relaxed text-emerald-300"
							>
								<span className="font-semibold">Applied:</span> {result}
							</output>
						)}
						<div className="flex items-center justify-between gap-2">
							<span className="truncate text-[9px] text-muted-foreground">
								Try “{coach}”
							</span>
							{result && onUndo && (
								<Button
									onClick={onUndo}
									size="sm"
									type="button"
									variant="ghost"
								>
									Undo
								</Button>
							)}
						</div>
					</div>
				</section>
			) : (
				<div className="flex items-center gap-2">
					<span className="sp-pilot-label max-w-56 rounded-full border border-primary/20 bg-background/92 px-3 py-1.5 text-[10px] font-medium text-foreground shadow-lg backdrop-blur">
						<span className="text-primary">Try FlowPilot</span>
						{!compact && (
							<span className="ml-1 text-muted-foreground">· {coach}</span>
						)}
					</span>
					<FlowPilotBubbleOrb
						ref={openerRef}
						aria-label={`Ask FlowPilot to ${coach}`}
						className="size-12 sp-pilot-orb"
						onClick={() => setOpen(true)}
						title={`Ask FlowPilot to ${coach}`}
					/>
				</div>
			)}
		</div>
	);
}

function WorkflowDemo({ compact = false }: Readonly<{ compact?: boolean }>) {
	const reducedMotion = useReducedMotion();
	const [activeNode, setActiveNode] = useState(workflowNodes.length - 1);
	const [runState, setRunState] = useState<"idle" | "running" | "complete">(
		"complete",
	);
	const [pilotChange, setPilotChange] = useState<FlowPilotChange | null>(null);
	const timers = useRef<number[]>([]);

	const nodes = useMemo(() => {
		const next = [...workflowNodes];
		if (!pilotChange) return next;
		const insertAt = next.length - 1;
		next.splice(insertAt, 0, pilotChange.node);
		return next;
	}, [pilotChange]);

	const clearTimers = useCallback(() => {
		for (const timer of timers.current) window.clearTimeout(timer);
		timers.current = [];
	}, []);

	useEffect(() => clearTimers, [clearTimers]);

	const replay = useCallback(() => {
		clearTimers();
		setActiveNode(-1);
		setRunState("running");
		if (reducedMotion) {
			setActiveNode(nodes.length - 1);
			setRunState("complete");
			return;
		}
		nodes.forEach((_, index) => {
			timers.current.push(
				window.setTimeout(
					() => {
						setActiveNode(index);
						if (index === nodes.length - 1) setRunState("complete");
					},
					360 + index * 680,
				),
			);
		});
	}, [clearTimers, nodes, reducedMotion]);

	const inspectNode = (index: number) => {
		clearTimers();
		setActiveNode(index);
		setRunState("idle");
	};

	const applyFlowPilotChange = (instruction: string) => {
		if (!instruction) return;
		clearTimers();
		const change = flowPilotChangeFor(instruction);
		setPilotChange(change);
		setActiveNode(workflowNodes.length - 1);
		setRunState("idle");
	};

	const selectedNode = nodes[Math.max(activeNode, 0)];
	const progress = activeNode < 0 ? 3 : ((activeNode + 1) / nodes.length) * 100;

	return (
		<div className="flex h-full min-h-0 flex-col">
			<div className="flex flex-wrap items-center gap-2 border-b border-border/60 bg-background/55 px-3 py-2.5 sm:px-4">
				<div className="min-w-0 flex-1">
					<div className="flex items-center gap-2">
						<strong className="truncate text-sm font-semibold">
							Support triage workflow
						</strong>
						<Badge
							className={cn(
								"h-5 border-0 px-1.5 text-[10px]",
								runState === "running"
									? "bg-sky-500/15 text-sky-400"
									: runState === "complete"
										? "bg-emerald-500/15 text-emerald-400"
										: "bg-muted text-muted-foreground",
							)}
						>
							<span
								className={cn(
									"size-1.5 rounded-full bg-current",
									runState === "running" && "sp-status-pulse",
								)}
							/>
							{runState === "running"
								? "Running"
								: runState === "complete"
									? "Ready"
									: "Inspecting"}
						</Badge>
					</div>
					<p className="sp-shell-detail mt-0.5 truncate text-[11px] text-muted-foreground">
						Four typed nodes · runs fully offline
					</p>
				</div>
				<Button
					aria-label="Replay workflow"
					onClick={replay}
					size="sm"
					variant="outline"
				>
					<Icon name="play" />
					Replay
				</Button>
			</div>

			<Progress
				aria-label={`Workflow progress: ${Math.round(progress)} percent`}
				className="h-1 rounded-none bg-muted/60"
				value={progress}
			/>

			<div className="sp-workflow-layout min-h-0 flex-1">
				<section className="sp-workflow-canvas relative min-h-0 overflow-hidden border-border/50">
					<ol
						aria-label="Interactive support triage workflow"
						className="sp-workflow-track m-0 list-none"
					>
						{nodes.map((node, index) => {
							const active = index === activeNode;
							const complete =
								index < activeNode ||
								(runState === "complete" && index < nodes.length - 1);
							const changed = node.id === pilotChange?.node.id;
							return (
								<li
									className={cn(
										"sp-workflow-node group relative min-w-0",
										active && "is-active",
										complete && "is-complete",
										changed && "is-pilot-changed",
									)}
									key={node.id}
								>
									<button
										aria-current={active ? "step" : undefined}
										className="block h-full w-full text-left"
										onClick={() => inspectNode(index)}
										type="button"
									>
										<FlowNodeShell
											className="sp-workflow-node-card h-full min-h-24"
											description={node.detail}
											icon={
												complete ? (
													<Icon className="size-2" name="check" />
												) : (
													<Icon className="size-2" name={node.icon} />
												)
											}
											kind={changed ? `${node.kind} · AI edit` : node.kind}
											label={node.name}
											selected={active}
											state={
												active && runState === "running"
													? "running"
													: complete
														? "complete"
														: "idle"
											}
											tone={
												node.id === "trigger"
													? "primary"
													: node.id === "route"
														? "neutral"
														: "tertiary"
											}
										/>
									</button>
								</li>
							);
						})}
					</ol>

					<DemoFlowPilot
						coach="add approval"
						compact={compact}
						onApply={applyFlowPilotChange}
						onUndo={() => setPilotChange(null)}
						placeholder="Add a manager approval before sending…"
						result={pilotChange?.summary}
					/>
				</section>

				<aside className="sp-workflow-inspector min-h-0 bg-muted/15 p-3 sm:p-4">
					<Card className="h-full gap-0 overflow-hidden py-0 shadow-none hover:border-border/50 hover:shadow-none">
						<CardHeader className="border-b border-border/50 px-3 py-3 sm:px-4">
							<CardDescription className="text-[9px] font-semibold uppercase tracking-[0.14em]">
								Selected node {Math.max(activeNode, 0) + 1} / {nodes.length}
							</CardDescription>
							<CardTitle className="mt-1 text-sm">
								{selectedNode.name}
							</CardTitle>
						</CardHeader>
						<CardContent className="space-y-3 px-3 py-3 text-[11px] sm:px-4">
							<div>
								<p className="text-muted-foreground">Configuration</p>
								<p className="mt-1 font-medium">{selectedNode.detail}</p>
							</div>
							<div className="grid grid-cols-2 gap-2">
								<div className="rounded-md bg-muted/55 p-2">
									<p className="text-[9px] text-muted-foreground">Input</p>
									<p className="mt-0.5 truncate font-mono">Ticket</p>
								</div>
								<div className="rounded-md bg-muted/55 p-2">
									<p className="text-[9px] text-muted-foreground">Output</p>
									<p className="mt-0.5 truncate font-mono">Result</p>
								</div>
							</div>
							<div className="sp-wide-only rounded-md border border-border/50 bg-background/65 p-2 font-mono text-[9px] leading-relaxed text-muted-foreground">
								<span className="text-emerald-400">✓</span> types connected
								<br />
								<span className="text-emerald-400">✓</span> policy checks passed
							</div>
						</CardContent>
					</Card>
				</aside>
			</div>
		</div>
	);
}

const runSteps = [
	{
		name: "Webhook received",
		detail: "support.ticket.created",
		duration: "12 ms",
		output: "24 new tickets",
	},
	{
		name: "Classify intent",
		detail: "Local Llama 3.2",
		duration: "482 ms",
		output: "18 billing · 4 bugs · 2 requests",
	},
	{
		name: "Route by priority",
		detail: "Typed condition",
		duration: "3 ms",
		output: "4 escalated to engineering",
	},
	{
		name: "Draft responses",
		detail: "Knowledge base + model",
		duration: "721 ms",
		output: "18 drafts ready for review",
	},
	{
		name: "Approval gate",
		detail: "Human in the loop",
		duration: "Waiting",
		output: "Review requested from Maya",
	},
] as const;

function RunsDemo({ compact = false }: Readonly<{ compact?: boolean }>) {
	const reducedMotion = useReducedMotion();
	const [activeStep, setActiveStep] = useState(runSteps.length - 1);
	const [pilotInsight, setPilotInsight] = useState<string | null>(null);
	const [runState, setRunState] = useState<"idle" | "running" | "complete">(
		"complete",
	);
	const timers = useRef<number[]>([]);

	const clearTimers = useCallback(() => {
		for (const timer of timers.current) window.clearTimeout(timer);
		timers.current = [];
	}, []);

	useEffect(() => clearTimers, [clearTimers]);

	const replay = useCallback(() => {
		clearTimers();
		setActiveStep(-1);
		setRunState("running");

		if (reducedMotion) {
			setActiveStep(runSteps.length - 1);
			setRunState("complete");
			return;
		}

		runSteps.forEach((_, index) => {
			const timer = window.setTimeout(
				() => {
					setActiveStep(index);
					if (index === runSteps.length - 1) setRunState("complete");
				},
				420 + index * 620,
			);
			timers.current.push(timer);
		});
	}, [clearTimers, reducedMotion]);

	const inspectStep = (index: number) => {
		clearTimers();
		setRunState("idle");
		setActiveStep(index);
	};

	const shownStep = runSteps[Math.max(activeStep, 0)];
	const progress =
		activeStep < 0 ? 4 : ((activeStep + 1) / runSteps.length) * 100;

	return (
		<div className="relative flex h-full min-h-0 flex-col">
			<div className="flex flex-wrap items-center gap-2 border-b border-border/60 bg-background/55 px-3 py-2.5 sm:px-4">
				<div className="min-w-0 flex-1">
					<div className="flex items-center gap-2">
						<strong className="truncate text-sm font-semibold">
							Support triage
						</strong>
						<Badge
							className={cn(
								"h-5 border-0 px-1.5 text-[10px]",
								runState === "running"
									? "bg-sky-500/15 text-sky-400"
									: runState === "complete"
										? "bg-emerald-500/15 text-emerald-400"
										: "bg-muted text-muted-foreground",
							)}
						>
							<span
								className={cn(
									"size-1.5 rounded-full bg-current",
									runState === "running" && "sp-status-pulse",
								)}
							/>
							{runState === "running"
								? "Running"
								: runState === "complete"
									? "Completed"
									: "Inspecting"}
						</Badge>
					</div>
					<p className="sp-shell-detail mt-0.5 truncate text-[11px] text-muted-foreground">
						Run #FL-2841 · local runtime
					</p>
				</div>
				<Button
					aria-label="Replay workflow execution"
					onClick={replay}
					size="sm"
					variant="outline"
				>
					<Icon name="play" />
					<span>Replay</span>
				</Button>
			</div>

			<Progress
				aria-label={`Run progress: ${Math.round(progress)} percent`}
				className="h-1 rounded-none bg-muted/60"
				value={progress}
			/>

			<div className="sp-run-grid min-h-0 flex-1">
				<ScrollArea className="min-h-0 border-border/50 sp-run-list">
					<div className="space-y-1.5 p-2.5 sm:p-3">
						{runSteps.map((step, index) => {
							const complete = index < activeStep || runState === "complete";
							const active = index === activeStep;
							return (
								<button
									aria-current={active ? "step" : undefined}
									className={cn(
										"sp-run-step group relative flex w-full items-center gap-2.5 rounded-lg border px-2.5 py-2 text-left transition-colors",
										active
											? "sp-step-active border-primary/40 bg-primary/10"
											: "border-transparent hover:border-border hover:bg-muted/45",
									)}
									key={step.name}
									onClick={() => inspectStep(index)}
									type="button"
								>
									<span
										className={cn(
											"grid size-6 shrink-0 place-items-center rounded-full border text-[10px] font-semibold",
											complete
												? "border-emerald-500/30 bg-emerald-500/15 text-emerald-400"
												: active
													? "border-primary/40 bg-primary/15 text-primary"
													: "border-border bg-background text-muted-foreground",
										)}
									>
										{complete ? (
											<Icon className="size-3" name="check" />
										) : (
											index + 1
										)}
									</span>
									<span className="min-w-0 flex-1">
										<span className="block truncate text-xs font-medium sm:text-sm">
											{step.name}
										</span>
										<span className="block truncate text-[10px] text-muted-foreground sm:text-[11px]">
											{step.detail}
										</span>
									</span>
									<span className="font-mono text-[9px] text-muted-foreground sm:text-[10px]">
										{active && runState === "running" ? "now" : step.duration}
									</span>
								</button>
							);
						})}
					</div>
				</ScrollArea>

				<div className="sp-run-detail min-h-0 bg-muted/15 p-3 sm:p-4">
					<Card className="h-full gap-0 overflow-hidden py-0 shadow-none hover:border-border/50 hover:shadow-none">
						<CardHeader className="border-b border-border/50 px-3 py-3 sm:px-4">
							<div className="flex items-start justify-between gap-3">
								<div className="min-w-0">
									<CardDescription className="text-[10px] font-semibold uppercase tracking-[0.14em]">
										Selected step
									</CardDescription>
									<CardTitle className="mt-1 truncate text-sm">
										{shownStep.name}
									</CardTitle>
								</div>
								<Badge variant="outline">{shownStep.duration}</Badge>
							</div>
						</CardHeader>
						<CardContent className="space-y-3 px-3 py-3 text-xs sm:px-4">
							{pilotInsight && (
								<output className="block rounded-md border border-primary/20 bg-primary/10 p-2 text-[10px] leading-relaxed text-foreground">
									{pilotInsight}
								</output>
							)}
							<div>
								<p className="text-muted-foreground">Output</p>
								<p className="mt-1 font-medium text-foreground">
									{shownStep.output}
								</p>
							</div>
							<div className="grid grid-cols-2 gap-2">
								<div className="rounded-md bg-muted/55 p-2">
									<p className="text-[10px] text-muted-foreground">Rows</p>
									<p className="mt-0.5 font-mono font-semibold">24</p>
								</div>
								<div className="rounded-md bg-muted/55 p-2">
									<p className="text-[10px] text-muted-foreground">Cost</p>
									<p className="mt-0.5 font-mono font-semibold">€0.004</p>
								</div>
							</div>
							<div className="sp-wide-only rounded-md border border-border/50 bg-background/65 p-2 font-mono text-[10px] leading-relaxed text-muted-foreground">
								<span className="text-emerald-400">✓</span> schema validated
								<br />
								<span className="text-emerald-400">✓</span> audit event recorded
							</div>
						</CardContent>
					</Card>
				</div>
			</div>
			<DemoFlowPilot
				coach="explain this run"
				compact={compact}
				onApply={() => {
					setActiveStep(1);
					setRunState("idle");
					setPilotInsight(
						"The local model is the longest step (482 ms); the approval gate is the only remaining blocker.",
					);
				}}
				onUndo={() => setPilotInsight(null)}
				placeholder="Explain the slowest step…"
				result={
					pilotInsight
						? "Highlighted the run’s main latency and blocker."
						: null
				}
			/>
		</div>
	);
}

interface Dataset {
	id: string;
	name: string;
	description: string;
	accent: string;
	columns: { key: string; label: string; type: string }[];
	rows: Record<string, string>[];
}

const datasets: Dataset[] = [
	{
		id: "customers",
		name: "Customers",
		description: "CRM · synced 2m ago",
		accent: "bg-violet-400",
		columns: [
			{ key: "company", label: "Company", type: "string" },
			{ key: "plan", label: "Plan", type: "enum" },
			{ key: "health", label: "Health", type: "score" },
			{ key: "mrr", label: "MRR", type: "currency" },
		],
		rows: [
			{
				company: "Northstar Labs",
				plan: "Scale",
				health: "92",
				mrr: "€18,400",
			},
			{ company: "Acme Robotics", plan: "Team", health: "78", mrr: "€8,200" },
			{ company: "Vela Health", plan: "Scale", health: "86", mrr: "€14,750" },
			{ company: "Kiteworks", plan: "Starter", health: "64", mrr: "€2,100" },
		],
	},
	{
		id: "revenue",
		name: "Revenue",
		description: "Finance · live view",
		accent: "bg-emerald-400",
		columns: [
			{ key: "month", label: "Month", type: "date" },
			{ key: "region", label: "Region", type: "string" },
			{ key: "actual", label: "Actual", type: "currency" },
			{ key: "target", label: "Target", type: "currency" },
		],
		rows: [
			{ month: "Apr 2026", region: "DACH", actual: "€284k", target: "€270k" },
			{
				month: "Apr 2026",
				region: "Nordics",
				actual: "€146k",
				target: "€155k",
			},
			{
				month: "Apr 2026",
				region: "Benelux",
				actual: "€118k",
				target: "€112k",
			},
			{ month: "Mar 2026", region: "DACH", actual: "€261k", target: "€255k" },
		],
	},
	{
		id: "tickets",
		name: "Support tickets",
		description: "Helpdesk · 24 open",
		accent: "bg-sky-400",
		columns: [
			{ key: "ticket", label: "Ticket", type: "id" },
			{ key: "topic", label: "Topic", type: "string" },
			{ key: "priority", label: "Priority", type: "enum" },
			{ key: "owner", label: "Owner", type: "user" },
		],
		rows: [
			{
				ticket: "#4821",
				topic: "Invoice export",
				priority: "Medium",
				owner: "Maya",
			},
			{
				ticket: "#4820",
				topic: "OAuth callback",
				priority: "High",
				owner: "Jonas",
			},
			{
				ticket: "#4819",
				topic: "Add workspace",
				priority: "Low",
				owner: "Ari",
			},
			{
				ticket: "#4818",
				topic: "Model timeout",
				priority: "High",
				owner: "Jonas",
			},
		],
	},
] as const;

function DataDemo({ compact = false }: Readonly<{ compact?: boolean }>) {
	const searchId = useId();
	const [selectedId, setSelectedId] = useState(datasets[0].id);
	const [query, setQuery] = useState("");
	const [pilotResult, setPilotResult] = useState<string | null>(null);
	const selected =
		datasets.find((dataset) => dataset.id === selectedId) ?? datasets[0];
	const normalizedQuery = query.trim().toLowerCase();
	const visibleDatasets = datasets.filter(
		(dataset) =>
			`${dataset.name} ${dataset.description}`
				.toLowerCase()
				.includes(normalizedQuery) ||
			dataset.rows.some((row) =>
				Object.values(row).some((value) =>
					value.toLowerCase().includes(normalizedQuery),
				),
			),
	);
	const visibleRows = selected.rows.filter((row) =>
		Object.values(row).some((value) =>
			value.toLowerCase().includes(normalizedQuery),
		),
	);
	const rows = normalizedQuery ? visibleRows : selected.rows;

	return (
		<div className="sp-data-grid relative h-full min-h-0">
			<aside className="sp-data-sidebar flex min-h-0 flex-col border-border/60 bg-muted/15">
				<div className="border-b border-border/50 p-2.5 sm:p-3">
					<label className="relative block" htmlFor={searchId}>
						<span className="sr-only">Search datasets and rows</span>
						<Icon
							className="pointer-events-none absolute left-2.5 top-1/2 z-10 size-3.5 -translate-y-1/2 text-muted-foreground"
							name="search"
						/>
						<Input
							className="h-8 bg-background/75 pl-8 text-xs"
							id={searchId}
							onChange={(event) => setQuery(event.target.value)}
							placeholder="Search data…"
							value={query}
						/>
					</label>
				</div>
				<ScrollArea
					className="sp-dataset-scroll min-h-0 flex-1"
					orientation="both"
				>
					<div className="sp-dataset-list space-y-1 p-2">
						{visibleDatasets.map((dataset) => (
							<button
								aria-pressed={dataset.id === selected.id}
								className={cn(
									"sp-dataset-button flex w-full items-center gap-2.5 rounded-lg border px-2.5 py-2 text-left transition-colors",
									dataset.id === selected.id
										? "border-primary/35 bg-primary/10"
										: "border-transparent hover:border-border hover:bg-muted/50",
								)}
								key={dataset.id}
								onClick={() => setSelectedId(dataset.id)}
								type="button"
							>
								<span
									className={cn("size-2 shrink-0 rounded-full", dataset.accent)}
								/>
								<span className="min-w-0">
									<span className="block truncate text-xs font-medium">
										{dataset.name}
									</span>
									<span className="sp-shell-detail block truncate text-[10px] text-muted-foreground">
										{dataset.description}
									</span>
								</span>
							</button>
						))}
						{visibleDatasets.length === 0 && (
							<p className="px-2 py-3 text-center text-[11px] text-muted-foreground">
								No dataset matches “{query}”. The selected table remains open.
							</p>
						)}
					</div>
				</ScrollArea>
			</aside>

			<section className="flex min-h-0 min-w-0 flex-col bg-background/50">
				<div className="flex items-center gap-3 border-b border-border/60 px-3 py-2.5 sm:px-4">
					<span className={cn("size-2.5 rounded-full", selected.accent)} />
					<div className="min-w-0 flex-1">
						<h3 className="truncate text-sm font-semibold">{selected.name}</h3>
						<p className="sp-shell-detail truncate text-[10px] text-muted-foreground">
							{selected.rows.length.toLocaleString()} preview rows · governed
							view
						</p>
					</div>
					<Badge
						className="bg-emerald-500/15 text-emerald-400"
						variant="secondary"
					>
						Live
					</Badge>
				</div>

				<Tabs className="min-h-0 flex-1 gap-0" defaultValue="rows">
					<div className="flex items-center justify-between border-b border-border/50 px-3 py-1.5 sm:px-4">
						<TabsList className="h-7 bg-muted/60">
							<TabsTrigger className="h-6 px-2 text-[11px]" value="rows">
								Rows
							</TabsTrigger>
							<TabsTrigger className="h-6 px-2 text-[11px]" value="schema">
								Schema
							</TabsTrigger>
						</TabsList>
						<span className="sp-wide-only font-mono text-[10px] text-muted-foreground">
							SELECT * · LIMIT 100
						</span>
					</div>

					<TabsContent className="min-h-0 overflow-auto" value="rows">
						<Table className="text-[11px] sm:text-xs">
							<TableHeader className="sticky top-0 z-10 bg-background/95 backdrop-blur">
								<TableRow>
									{selected.columns.map((column) => (
										<TableHead
											className="h-8 px-3 text-[10px] uppercase tracking-wider text-muted-foreground"
											key={column.key}
										>
											{column.label}
										</TableHead>
									))}
								</TableRow>
							</TableHeader>
							<TableBody>
								{rows.map((row, index) => (
									<TableRow key={`${selected.id}-${index}`}>
										{selected.columns.map((column) => (
											<TableCell className="px-3 py-2" key={column.key}>
												{row[column.key]}
											</TableCell>
										))}
									</TableRow>
								))}
							</TableBody>
						</Table>
						{rows.length === 0 && (
							<div className="grid min-h-28 place-items-center px-4 text-center text-[11px] text-muted-foreground">
								No rows match “{query}”.
							</div>
						)}
					</TabsContent>
					<TabsContent
						className="min-h-0 overflow-auto p-2.5 sm:p-4"
						value="schema"
					>
						<div className="grid gap-1.5">
							{selected.columns.map((column) => (
								<div
									className="flex items-center gap-3 rounded-md border border-border/50 bg-muted/25 px-3 py-2"
									key={column.key}
								>
									<Icon
										className="size-3.5 text-muted-foreground"
										name="database"
									/>
									<code className="min-w-0 flex-1 truncate text-[11px]">
										{column.key}
									</code>
									<Badge className="font-mono text-[9px]" variant="outline">
										{column.type}
									</Badge>
								</div>
							))}
						</div>
					</TabsContent>
				</Tabs>
			</section>
			<DemoFlowPilot
				coach="show high-priority tickets"
				compact={compact}
				onApply={() => {
					setSelectedId("tickets");
					setQuery("High");
					setPilotResult(
						"Created a governed view with two high-priority tickets.",
					);
				}}
				onUndo={() => {
					setQuery("");
					setPilotResult(null);
				}}
				placeholder="Create a view for high-priority tickets…"
				result={pilotResult}
			/>
		</div>
	);
}

type CatalogCategory = "all" | "ai" | "data" | "ops";

const catalogNodes = [
	{
		id: "openai",
		name: "Chat completion",
		publisher: "Flow-Like",
		category: "ai" as const,
		description: "Run a model with tools and structured output.",
		color: "from-violet-500/25 to-fuchsia-500/10 text-violet-300",
	},
	{
		id: "postgres",
		name: "PostgreSQL query",
		publisher: "Flow-Like",
		category: "data" as const,
		description: "Read or write typed rows in any Postgres database.",
		color: "from-sky-500/25 to-cyan-500/10 text-sky-300",
	},
	{
		id: "http",
		name: "HTTP request",
		publisher: "Core",
		category: "ops" as const,
		description: "Call APIs with auth, retries and validation.",
		color: "from-amber-500/25 to-orange-500/10 text-amber-300",
	},
	{
		id: "document",
		name: "Document extractor",
		publisher: "Rheosoph Labs",
		category: "ai" as const,
		description: "Extract governed fields from PDFs and images.",
		color: "from-emerald-500/25 to-teal-500/10 text-emerald-300",
	},
	{
		id: "s3",
		name: "Object storage",
		publisher: "Core",
		category: "data" as const,
		description: "Work with S3-compatible files and buckets.",
		color: "from-rose-500/25 to-red-500/10 text-rose-300",
	},
] as const;

function CatalogDemo({ compact = false }: Readonly<{ compact?: boolean }>) {
	const searchId = useId();
	const [query, setQuery] = useState("");
	const [category, setCategory] = useState<CatalogCategory>("all");
	const [installed, setInstalled] = useState(() => new Set(["http"]));
	const [pilotResult, setPilotResult] = useState<string | null>(null);
	const normalizedQuery = query.trim().toLowerCase();
	const visibleNodes = catalogNodes.filter(
		(node) =>
			(category === "all" || node.category === category) &&
			`${node.name} ${node.publisher} ${node.description}`
				.toLowerCase()
				.includes(normalizedQuery),
	);

	const toggleInstalled = (id: string) => {
		setInstalled((current) => {
			const next = new Set(current);
			if (next.has(id)) next.delete(id);
			else next.add(id);
			return next;
		});
	};

	return (
		<div className="relative flex h-full min-h-0 flex-col">
			<div className="border-b border-border/60 bg-background/55 px-3 py-2.5 sm:px-4">
				<div className="flex items-center gap-2.5">
					<label className="relative min-w-0 flex-1" htmlFor={searchId}>
						<span className="sr-only">Search node catalog</span>
						<Icon
							className="pointer-events-none absolute left-2.5 top-1/2 z-10 size-3.5 -translate-y-1/2 text-muted-foreground"
							name="search"
						/>
						<Input
							className="h-8 bg-background/75 pl-8 text-xs"
							id={searchId}
							onChange={(event) => setQuery(event.target.value)}
							placeholder="Search 400+ nodes…"
							value={query}
						/>
					</label>
					<Badge className="sp-shell-detail shrink-0" variant="outline">
						{installed.size} installed
					</Badge>
					<span aria-live="polite" className="sr-only">
						{installed.size} catalog nodes installed
					</span>
				</div>
			</div>

			<Tabs
				className="min-h-0 flex-1 gap-0"
				onValueChange={(value) => setCategory(value as CatalogCategory)}
				value={category}
			>
				<div className="border-b border-border/50 px-3 py-1.5 sm:px-4">
					<TabsList className="h-7 w-full justify-start bg-muted/60 sm:w-fit">
						{(["all", "ai", "data", "ops"] as const).map((value) => (
							<TabsTrigger
								className="h-6 px-2.5 text-[11px] capitalize"
								key={value}
								value={value}
							>
								{value}
							</TabsTrigger>
						))}
					</TabsList>
				</div>

				<TabsContent className="min-h-0" value={category}>
					<ScrollArea className="h-full">
						<div className="sp-catalog-grid grid gap-2 p-2.5 sm:grid-cols-2 sm:p-3">
							{visibleNodes.map((node) => {
								const isInstalled = installed.has(node.id);
								return (
									<Card
										className="gap-0 overflow-hidden py-0 shadow-none hover:-translate-y-0.5 hover:shadow-md"
										key={node.id}
									>
										<CardContent className="flex h-full items-start gap-2.5 p-2.5 sm:p-3">
											<div
												className={cn(
													"grid size-9 shrink-0 place-items-center rounded-lg bg-gradient-to-br",
													node.color,
												)}
											>
												<Icon
													className="size-4"
													name={
														node.category === "ai"
															? "sparkles"
															: node.category === "data"
																? "database"
																: "package"
													}
												/>
											</div>
											<div className="min-w-0 flex-1">
												<div className="flex items-start gap-2">
													<div className="min-w-0 flex-1">
														<h3 className="truncate text-xs font-semibold sm:text-sm">
															{node.name}
														</h3>
														<p className="text-[9px] text-muted-foreground sm:text-[10px]">
															by {node.publisher}
														</p>
													</div>
													<Button
														aria-label={`${isInstalled ? "Remove" : "Install"} ${node.name}`}
														className={cn(
															"h-7 px-2 text-[10px]",
															isInstalled && "text-emerald-400",
														)}
														onClick={() => toggleInstalled(node.id)}
														size="sm"
														variant={isInstalled ? "ghost" : "outline"}
													>
														{isInstalled ? (
															<>
																<Icon name="check" />
																<span className="sp-installed-label">
																	Installed
																</span>
															</>
														) : (
															"Install"
														)}
													</Button>
												</div>
												<p className="sp-node-description mt-2 line-clamp-2 text-[10px] leading-relaxed text-muted-foreground sm:text-[11px]">
													{node.description}
												</p>
											</div>
										</CardContent>
									</Card>
								);
							})}
							{visibleNodes.length === 0 && (
								<div className="col-span-full grid min-h-32 place-items-center rounded-xl border border-dashed border-border p-6 text-center">
									<div>
										<Icon
											className="mx-auto size-5 text-muted-foreground"
											name="search"
										/>
										<p className="mt-2 text-xs font-medium">No matching node</p>
										<p className="mt-1 text-[10px] text-muted-foreground">
											Try another search or category.
										</p>
									</div>
								</div>
							)}
						</div>
					</ScrollArea>
				</TabsContent>
			</Tabs>
			<DemoFlowPilot
				coach="install PostgreSQL"
				compact={compact}
				onApply={() => {
					setCategory("data");
					setQuery("PostgreSQL");
					setInstalled((current) => new Set([...current, "postgres"]));
					setPilotResult(
						"Installed PostgreSQL Query and filtered the catalog to related data nodes.",
					);
				}}
				onUndo={() => {
					setQuery("");
					setCategory("all");
					setInstalled((current) => {
						const next = new Set(current);
						next.delete("postgres");
						return next;
					});
					setPilotResult(null);
				}}
				placeholder="Install the PostgreSQL node…"
				result={pilotResult}
			/>
		</div>
	);
}

interface PrototypeForm {
	title: string;
	requestor: string;
	purpose: string;
	expires: string;
	requiresApproval: boolean;
}

const initialForm: PrototypeForm = {
	title: "Visitor access",
	requestor: "Alex Morgan",
	purpose: "Customer workshop",
	expires: "2026-07-18",
	requiresApproval: true,
};

function PrototypeDemo({ compact = false }: Readonly<{ compact?: boolean }>) {
	const fieldId = useId();
	const [draft, setDraft] = useState(initialForm);
	const [saved, setSaved] = useState(initialForm);
	const [submitted, setSubmitted] = useState(false);
	const [pilotResult, setPilotResult] = useState<string | null>(null);
	const dirty = JSON.stringify(draft) !== JSON.stringify(saved);

	const update = <Key extends keyof PrototypeForm>(
		key: Key,
		value: PrototypeForm[Key],
	) => setDraft((current) => ({ ...current, [key]: value }));

	const save = (event: FormEvent) => {
		event.preventDefault();
		setSaved(draft);
		setSubmitted(false);
	};

	return (
		<div className="sp-prototype-grid relative h-full min-h-0">
			<form
				className="flex min-h-0 flex-col border-border/60 bg-muted/15"
				onSubmit={save}
			>
				<div className="flex items-center justify-between gap-3 border-b border-border/50 px-3 py-2.5 sm:px-4">
					<div className="min-w-0">
						<h3 className="truncate text-sm font-semibold">
							Edit internal app
						</h3>
						<p className="sp-shell-detail truncate text-[10px] text-muted-foreground">
							Changes stay in this preview
						</p>
					</div>
					<Badge
						className={dirty ? "text-amber-400" : "text-emerald-400"}
						variant="outline"
					>
						{dirty ? "Unsaved" : "Saved"}
					</Badge>
				</div>

				<ScrollArea className="min-h-0 flex-1">
					<div className="grid gap-3 p-3 sm:p-4">
						<label className="grid gap-1.5" htmlFor={`${fieldId}-title`}>
							<span className="text-[10px] font-medium uppercase tracking-wider text-muted-foreground">
								App title
							</span>
							<Input
								className="h-8 bg-background/75 text-xs"
								id={`${fieldId}-title`}
								onChange={(event) => update("title", event.target.value)}
								value={draft.title}
							/>
						</label>
						<div className="sp-form-fields grid grid-cols-2 gap-3">
							<label className="grid gap-1.5" htmlFor={`${fieldId}-requestor`}>
								<span className="text-[10px] font-medium uppercase tracking-wider text-muted-foreground">
									Requestor
								</span>
								<Input
									className="h-8 bg-background/75 text-xs"
									id={`${fieldId}-requestor`}
									onChange={(event) => update("requestor", event.target.value)}
									value={draft.requestor}
								/>
							</label>
							<label className="grid gap-1.5" htmlFor={`${fieldId}-expires`}>
								<span className="text-[10px] font-medium uppercase tracking-wider text-muted-foreground">
									Expires
								</span>
								<Input
									className="h-8 bg-background/75 text-xs"
									id={`${fieldId}-expires`}
									onChange={(event) => update("expires", event.target.value)}
									type="date"
									value={draft.expires}
								/>
							</label>
						</div>
						<label className="grid gap-1.5" htmlFor={`${fieldId}-purpose`}>
							<span className="text-[10px] font-medium uppercase tracking-wider text-muted-foreground">
								Purpose
							</span>
							<Input
								className="h-8 bg-background/75 text-xs"
								id={`${fieldId}-purpose`}
								onChange={(event) => update("purpose", event.target.value)}
								value={draft.purpose}
							/>
						</label>
						<label
							className="flex items-center justify-between gap-3 rounded-lg border border-border/50 bg-background/55 px-3 py-2"
							htmlFor={`${fieldId}-approval`}
						>
							<span>
								<span className="block text-xs font-medium">
									Require manager approval
								</span>
								<span className="sp-shell-detail block text-[10px] text-muted-foreground">
									Adds a governed review step
								</span>
							</span>
							<Switch
								checked={draft.requiresApproval}
								id={`${fieldId}-approval`}
								onCheckedChange={(checked) =>
									update("requiresApproval", checked)
								}
							/>
						</label>
					</div>
				</ScrollArea>

				<div className="flex items-center justify-between gap-3 border-t border-border/50 bg-background/45 px-3 py-2 sm:px-4">
					<span className="sp-shell-detail text-[10px] text-muted-foreground">
						Draft · local only
					</span>
					<Button disabled={!dirty} size="sm" type="submit">
						<Icon name="save" />
						Save app
					</Button>
				</div>
			</form>

			<div className="sp-prototype-preview min-h-0 bg-background/50 p-3 sm:p-4">
				<Card className="mx-auto h-full max-w-lg gap-0 overflow-hidden py-0 shadow-lg hover:border-border/50 hover:shadow-lg">
					<div className="flex items-center gap-2 border-b border-border/50 bg-muted/30 px-3 py-2">
						<span className="grid size-6 place-items-center rounded-md bg-primary/15 text-primary">
							<Icon className="size-3.5" name="form" />
						</span>
						<span className="min-w-0 flex-1 truncate text-xs font-medium">
							Workplace tools
						</span>
						<Badge className="text-[9px]" variant="outline">
							Internal
						</Badge>
					</div>
					<CardHeader className="px-4 pb-2 pt-4">
						<CardTitle className="text-base sm:text-lg">
							{saved.title}
						</CardTitle>
						<CardDescription className="text-xs">
							Submit a temporary building-access request.
						</CardDescription>
					</CardHeader>
					<CardContent className="space-y-3 px-4 pb-4 text-xs">
						<div className="grid grid-cols-2 gap-2">
							<div className="rounded-lg bg-muted/45 p-2.5">
								<p className="text-[9px] uppercase tracking-wider text-muted-foreground">
									Requestor
								</p>
								<p className="mt-1 truncate font-medium">
									{saved.requestor || "Not set"}
								</p>
							</div>
							<div className="rounded-lg bg-muted/45 p-2.5">
								<p className="text-[9px] uppercase tracking-wider text-muted-foreground">
									Valid until
								</p>
								<p className="mt-1 truncate font-medium">
									{saved.expires || "Not set"}
								</p>
							</div>
						</div>
						<div className="rounded-lg border border-border/50 px-3 py-2.5">
							<p className="text-[9px] uppercase tracking-wider text-muted-foreground">
								Purpose
							</p>
							<p className="mt-1 font-medium">{saved.purpose || "Not set"}</p>
						</div>
						<div className="flex items-center justify-between gap-3">
							<span className="text-[10px] text-muted-foreground">
								{saved.requiresApproval
									? "Manager approval required"
									: "Auto-approved by policy"}
							</span>
							<Button
								onClick={() => setSubmitted(true)}
								size="sm"
								type="button"
							>
								{submitted ? <Icon name="check" /> : <Icon name="arrow" />}
								{submitted ? "Sent" : "Request"}
							</Button>
						</div>
					</CardContent>
				</Card>
			</div>
			<DemoFlowPilot
				coach="adapt this app"
				compact={compact}
				onApply={(instruction) => {
					const contractor = instruction.toLowerCase().includes("contractor");
					const next = {
						...saved,
						title: contractor ? "Contractor access" : "Team access request",
						purpose: contractor
							? "Temporary contractor onboarding"
							: saved.purpose,
					};
					setDraft(next);
					setSaved(next);
					setSubmitted(false);
					setPilotResult(`Updated and saved “${next.title}”.`);
				}}
				onUndo={() => {
					setDraft(initialForm);
					setSaved(initialForm);
					setPilotResult(null);
				}}
				placeholder="Adapt this for contractor access…"
				result={pilotResult}
			/>
		</div>
	);
}

const variantMeta: Record<
	ShowcaseProductVariant,
	{ icon: IconName; label: string; eyebrow: string }
> = {
	workflow: { icon: "workflow", label: "Workflow canvas", eyebrow: "Build" },
	runs: { icon: "play", label: "Run inspector", eyebrow: "Observe" },
	data: { icon: "database", label: "Data explorer", eyebrow: "Govern" },
	catalog: { icon: "package", label: "Node catalog", eyebrow: "Extend" },
	prototype: { icon: "form", label: "App builder", eyebrow: "Prototype" },
};

export default function ShowcaseProduct({
	variant,
	presentation = "full",
	className,
}: Readonly<ShowcaseProductProps>) {
	const meta = variantMeta[variant];
	const compact = presentation === "compact";
	const demo = useMemo(() => {
		switch (variant) {
			case "workflow":
				return <WorkflowDemo compact={compact} />;
			case "runs":
				return <RunsDemo compact={compact} />;
			case "data":
				return <DataDemo compact={compact} />;
			case "catalog":
				return <CatalogDemo compact={compact} />;
			case "prototype":
				return <PrototypeDemo compact={compact} />;
		}
	}, [compact, variant]);

	return (
		<div
			className={cn(
				"showcase-product flex h-full min-h-0 w-full flex-col overflow-hidden bg-background text-foreground",
				className,
			)}
			data-presentation={presentation}
			data-showcase-product={variant}
		>
			<style>{showcaseProductStyles}</style>
			{presentation === "full" && (
				<header className="flex h-10 shrink-0 items-center gap-2 border-b border-border/60 bg-background/90 px-3 backdrop-blur sm:h-11 sm:px-4">
					<span className="grid size-6 shrink-0 place-items-center rounded-md bg-primary/12 text-primary">
						<Icon className="size-3.5" name={meta.icon} />
					</span>
					<div className="min-w-0 flex-1">
						<p className="truncate text-[11px] font-semibold sm:text-xs">
							{meta.label}
						</p>
						<p className="sp-shell-detail truncate font-mono text-[8px] uppercase tracking-[0.16em] text-muted-foreground sm:text-[9px]">
							Flow-Like Studio · {meta.eyebrow}
						</p>
					</div>
				</header>
			)}
			<section className="min-h-0 flex-1" aria-label={meta.label}>
				{demo}
			</section>
		</div>
	);
}

const showcaseProductStyles = `
	.showcase-product {
		container-type: inline-size;
		background:
			radial-gradient(circle at 78% 0%, color-mix(in oklch, var(--primary) 9%, transparent), transparent 34%),
			var(--background);
	}

	.showcase-product .sp-workflow-layout {
		display: grid;
		grid-template-columns: minmax(0, 1fr) minmax(190px, 0.28fr);
	}

	.showcase-product .sp-workflow-canvas {
		border-right: 1px solid color-mix(in oklch, var(--border) 60%, transparent);
		background-image: radial-gradient(color-mix(in oklch, var(--foreground) 12%, transparent) 0.75px, transparent 0.75px);
		background-size: 13px 13px;
	}

	.showcase-product .sp-workflow-track {
		position: relative;
		display: grid;
		grid-template-columns: repeat(auto-fit, minmax(112px, 1fr));
		align-content: center;
		gap: 1.75rem;
		height: 100%;
		padding: clamp(1rem, 4cqw, 3rem);
	}

	.showcase-product .sp-workflow-node::after {
		position: absolute;
		z-index: 0;
		top: 50%;
		left: 100%;
		width: 1.75rem;
		border-top: 1px solid color-mix(in oklch, var(--foreground) 24%, transparent);
		content: "";
		pointer-events: none;
	}

	.showcase-product .sp-workflow-node::before {
		position: absolute;
		z-index: 1;
		top: calc(50% - 3px);
		right: -1.75rem;
		width: 6px;
		height: 6px;
		border-top: 1px solid color-mix(in oklch, var(--foreground) 34%, transparent);
		border-right: 1px solid color-mix(in oklch, var(--foreground) 34%, transparent);
		content: "";
		transform: rotate(45deg);
		pointer-events: none;
	}

	.showcase-product .sp-workflow-node:last-child::before,
	.showcase-product .sp-workflow-node:last-child::after {
		display: none;
	}

	.showcase-product .sp-workflow-node-card {
		position: relative;
		z-index: 2;
		background: color-mix(in oklch, var(--card) 92%, transparent);
	}

	.showcase-product .sp-workflow-node.is-active .sp-workflow-node-card {
		border-color: color-mix(in oklch, var(--primary) 58%, var(--border));
		box-shadow:
			0 0 0 2px color-mix(in oklch, var(--primary) 13%, transparent),
			0 14px 34px color-mix(in srgb, black 22%, transparent);
		transform: translateY(-2px);
	}

	.showcase-product .sp-workflow-node.is-complete::after {
		border-color: color-mix(in oklch, #34d399 42%, transparent);
	}

	.showcase-product .sp-workflow-node.is-pilot-changed .sp-workflow-node-card {
		animation: sp-pilot-change 0.7s cubic-bezier(0.16, 1, 0.3, 1);
	}

	.showcase-product .sp-pilot-panel {
		width: 22rem;
		max-width: calc(100% - 0.5rem);
		background: color-mix(in oklch, var(--background) 96%, transparent);
	}

	.showcase-product[data-presentation="compact"] .sp-workflow-inspector,
	.showcase-product[data-presentation="compact"] .sp-run-detail {
		display: none;
	}

	.showcase-product[data-presentation="compact"] .sp-workflow-layout,
	.showcase-product[data-presentation="compact"] .sp-run-grid {
		grid-template-columns: minmax(0, 1fr);
	}

	.showcase-product[data-presentation="compact"] .sp-workflow-canvas,
	.showcase-product[data-presentation="compact"] .sp-run-list {
		border-right: 0;
	}

	.showcase-product[data-presentation="compact"] .sp-workflow-track {
		grid-template-columns: repeat(2, minmax(0, 1fr));
		grid-template-rows: repeat(2, minmax(0, 1fr));
		gap: 0.65rem 1.15rem;
		padding: 0.65rem 0.85rem;
	}

	.showcase-product[data-presentation="compact"] .sp-workflow-node:nth-child(3) {
		grid-column: 2;
		grid-row: 2;
	}

	.showcase-product[data-presentation="compact"] .sp-workflow-node:nth-child(4) {
		grid-column: 1;
		grid-row: 2;
	}

	.showcase-product[data-presentation="compact"] .sp-workflow-node:nth-child(n + 5),
	.showcase-product[data-presentation="compact"] .sp-workflow-node::before,
	.showcase-product[data-presentation="compact"] .sp-workflow-node::after {
		display: none;
	}

	.showcase-product[data-presentation="compact"] .sp-workflow-track::before,
	.showcase-product[data-presentation="compact"] .sp-workflow-track::after {
		position: absolute;
		z-index: 0;
		content: "";
		pointer-events: none;
	}

	.showcase-product[data-presentation="compact"] .sp-workflow-track::before {
		top: 25%;
		right: 25%;
		bottom: 25%;
		left: 25%;
		border-top: 1px solid color-mix(in oklch, var(--foreground) 24%, transparent);
		border-bottom: 1px solid color-mix(in oklch, var(--foreground) 24%, transparent);
	}

	.showcase-product[data-presentation="compact"] .sp-workflow-track::after {
		top: 25%;
		right: 25%;
		bottom: 25%;
		border-right: 1px solid color-mix(in oklch, var(--foreground) 24%, transparent);
	}

	@keyframes sp-pilot-change {
		0% { opacity: 0.45; transform: scale(0.96); }
		55% { box-shadow: 0 0 0 5px color-mix(in oklch, var(--primary) 13%, transparent); }
		100% { opacity: 1; transform: scale(1); }
	}

	.showcase-product .sp-run-grid {
		display: grid;
		grid-template-columns: minmax(0, 1.15fr) minmax(190px, 0.85fr);
	}

	.showcase-product .sp-run-list,
	.showcase-product .sp-data-sidebar {
		border-right: 1px solid color-mix(in oklch, var(--border) 60%, transparent);
	}

	.showcase-product .sp-data-grid {
		display: grid;
		grid-template-columns: minmax(150px, 0.33fr) minmax(0, 1fr);
	}

	.showcase-product .sp-prototype-grid {
		display: grid;
		grid-template-columns: minmax(230px, 0.8fr) minmax(280px, 1.2fr);
	}

	.showcase-product .sp-catalog-grid {
		grid-template-columns: repeat(auto-fit, minmax(230px, 1fr));
	}

	.showcase-product .sp-step-active::after {
		position: absolute;
		inset: -1px;
		border: 1px solid color-mix(in oklch, var(--primary) 28%, transparent);
		border-radius: inherit;
		content: "";
		pointer-events: none;
		animation: sp-soft-ring 1.8s ease-out infinite;
	}

	.showcase-product .sp-status-pulse {
		animation: sp-status-pulse 1.2s ease-in-out infinite;
	}

	@keyframes sp-soft-ring {
		0% { opacity: 0.8; transform: scale(0.995); }
		70%, 100% { opacity: 0; transform: scale(1.025); }
	}

	@keyframes sp-status-pulse {
		0%, 100% { opacity: 0.45; }
		50% { opacity: 1; }
	}

	@container (max-width: 760px) {
		.showcase-product .sp-workflow-layout {
			grid-template-columns: minmax(0, 1fr);
			grid-template-rows: minmax(0, 1fr) auto;
		}

		.showcase-product .sp-workflow-canvas {
			border-right: 0;
			border-bottom: 1px solid color-mix(in oklch, var(--border) 60%, transparent);
		}

		.showcase-product .sp-workflow-track {
			padding: 1rem;
		}

		.showcase-product .sp-workflow-inspector {
			max-height: 8.5rem;
			padding: 0.5rem;
		}
	}

	@container (max-width: 560px) {
		.showcase-product .sp-workflow-layout {
			grid-template-rows: minmax(0, 1fr);
		}

		.showcase-product .sp-workflow-inspector {
			display: none;
		}

		.showcase-product .sp-workflow-track {
			grid-template-columns: repeat(2, minmax(0, 1fr));
			grid-template-rows: repeat(2, minmax(0, 1fr));
			gap: 0.75rem 1.25rem;
			padding: 0.65rem;
		}

		.showcase-product .sp-workflow-node:nth-child(3) {
			grid-column: 2;
			grid-row: 2;
		}

		.showcase-product .sp-workflow-node:nth-child(4) {
			grid-column: 1;
			grid-row: 2;
		}

		.showcase-product .sp-workflow-node::before,
		.showcase-product .sp-workflow-node::after {
			display: none;
		}

		.showcase-product .sp-workflow-track::before {
			position: absolute;
			z-index: 0;
			top: 25%;
			right: 25%;
			bottom: 25%;
			left: 25%;
			border-top: 1px solid color-mix(in oklch, var(--foreground) 24%, transparent);
			border-bottom: 1px solid color-mix(in oklch, var(--foreground) 24%, transparent);
			content: "";
			pointer-events: none;
		}

		.showcase-product .sp-workflow-track::after {
			position: absolute;
			z-index: 0;
			top: 25%;
			right: 25%;
			bottom: 25%;
			border-right: 1px solid color-mix(in oklch, var(--foreground) 24%, transparent);
			content: "";
			pointer-events: none;
		}

		.showcase-product .sp-pilot-anchor {
			inset: 0.25rem;
			display: flex;
			align-items: flex-end;
			justify-content: flex-end;
			pointer-events: none;
		}

		.showcase-product .sp-pilot-anchor > * {
			pointer-events: auto;
		}

		.showcase-product .sp-pilot-panel {
			max-width: 100%;
			max-height: 100%;
			overflow: auto;
		}

		.showcase-product .sp-run-grid,
		.showcase-product .sp-data-grid,
		.showcase-product .sp-prototype-grid {
			grid-template-columns: minmax(0, 1fr);
		}

		.showcase-product .sp-run-grid {
			grid-template-rows: minmax(118px, 1fr) auto;
		}

		.showcase-product .sp-run-list,
		.showcase-product .sp-data-sidebar {
			border-right: 0;
			border-bottom: 1px solid color-mix(in oklch, var(--border) 60%, transparent);
		}

		.showcase-product .sp-run-detail {
			padding: 0.5rem;
		}

		.showcase-product .sp-run-detail [data-slot="card-content"] {
			display: none;
		}

		.showcase-product .sp-data-grid {
			grid-template-rows: auto minmax(0, 1fr);
		}

		.showcase-product .sp-data-sidebar {
			display: block;
		}

		.showcase-product .sp-dataset-scroll {
			height: 3.25rem;
		}

		.showcase-product .sp-dataset-list {
			display: flex;
			gap: 0.25rem;
			padding: 0.375rem 0.5rem;
		}

		.showcase-product .sp-dataset-button {
			min-width: max-content;
			width: auto;
			padding-block: 0.4rem;
		}

		.showcase-product .sp-catalog-grid {
			grid-template-columns: minmax(0, 1fr);
		}

		.showcase-product .sp-prototype-grid {
			grid-template-rows: minmax(0, 1fr) auto;
		}

		.showcase-product .sp-prototype-preview {
			max-height: 13rem;
			overflow: auto;
			border-top: 1px solid color-mix(in oklch, var(--border) 60%, transparent);
		}
	}

	@container (max-width: 360px) {
		.showcase-product > header {
			height: 2rem;
			padding-inline: 0.5rem;
		}

		.showcase-product > header > [data-slot="badge"] {
			display: none;
		}

		.showcase-product .sp-shell-detail,
		.showcase-product .sp-wide-only,
		.showcase-product .sp-node-description {
			display: none;
		}

		.showcase-product .sp-workflow-track {
			gap: 0.4rem 0.75rem;
			padding: 0.4rem;
		}

		.showcase-product .sp-workflow-node-card [data-slot="card-content"] {
			gap: 0.4rem;
			padding: 0.4rem;
		}

		.showcase-product .sp-workflow-node-card [data-slot="card-content"] > span:first-child {
			width: 1.75rem;
			height: 1.75rem;
		}

		.showcase-product .sp-pilot-label {
			display: none;
		}

		.showcase-product .sp-pilot-orb {
			width: 2.5rem;
			height: 2.5rem;
		}

		.showcase-product .sp-run-grid {
			grid-template-rows: minmax(0, 1fr);
		}

		.showcase-product .sp-run-detail {
			display: none;
		}

		.showcase-product .sp-run-step {
			padding-block: 0.35rem;
		}

		.showcase-product .sp-data-sidebar {
			display: grid;
			grid-template-columns: minmax(112px, 0.85fr) minmax(0, 1.15fr);
		}

		.showcase-product .sp-data-sidebar > div:first-child {
			border-right: 1px solid color-mix(in oklch, var(--border) 60%, transparent);
			border-bottom: 0;
			padding: 0.375rem;
		}

		.showcase-product .sp-dataset-scroll {
			height: 2.75rem;
		}

		.showcase-product .sp-dataset-button {
			padding: 0.3rem 0.45rem;
		}

		.showcase-product .sp-data-grid > section > div:first-child {
			display: none;
		}

		.showcase-product .sp-form-fields {
			grid-template-columns: minmax(0, 1fr);
		}

		.showcase-product .sp-prototype-preview {
			display: none;
		}
	}

	@media (prefers-reduced-motion: reduce) {
		.showcase-product *,
		.showcase-product *::before,
		.showcase-product *::after {
			scroll-behavior: auto !important;
			animation-duration: 0.001ms !important;
			animation-iteration-count: 1 !important;
			transition-duration: 0.001ms !important;
		}
	}
`;
