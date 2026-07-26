"use client";

import {
	Badge,
	Button,
	Input,
	Label,
	ScrollArea,
	cn,
	useBackend,
	useCopilotSDK,
} from "@flow-like/flow-like-ui";
import type { CopilotModel } from "@flow-like/flow-like-ui/components/flowpilot/types";
import { GlobalChatView } from "@flow-like/flow-like-ui/components/global-chat/global-chat-view";
import { FLOWPILOT_DEBUG_ENABLED } from "@flow-like/flow-like-ui/lib/flowpilot-debug";
import {
	type FlowScriptGenerationRunReceipt,
	flowScriptGenerationRunsForConversation,
} from "@flow-like/flow-like-ui/lib/flowpilot/flowscript-generation-receipt";
import { useGlobalChatStore } from "@flow-like/flow-like-ui/state/global-chat/global-chat-store";
import {
	LAST_CONVERSATION_KEY,
	readActiveRun,
} from "@flow-like/flow-like-ui/state/global-chat/global-chat-stream";
import {
	AlertTriangle,
	CheckCircle2,
	Clipboard,
	Download,
	FlaskConical,
	Loader2,
	Play,
	XCircle,
} from "lucide-react";
import {
	useCallback,
	useEffect,
	useLayoutEffect,
	useMemo,
	useRef,
	useState,
} from "react";
import { toast } from "sonner";
import {
	FLOWPILOT_APP_CREATION_CASES,
	FLOWPILOT_APP_CREATION_SMOKE_CASES,
	FLOWPILOT_E2E_DEFAULT_MODEL,
	type FlowPilotAppCreationSnapshot,
	type FlowPilotE2EArtifact,
	type FlowPilotE2EAssistantTrace,
	type FlowPilotE2ECaseDefinition,
	type FlowPilotE2ECaseId,
	type FlowPilotE2ECheck,
	type FlowPilotE2ECliEnvelope,
	type FlowPilotE2EModelConfig,
	type FlowPilotE2ERunOptions,
	type FlowPilotE2ERunReport,
	type FlowPilotE2ERunnerIssue,
	appCreationFailureFingerprint,
	authoredFlowScriptEvidence,
	buildCasePrompt,
	evaluateAppCreationCase,
	flowPilotE2EArtifactPassed,
	resolveFlowPilotE2ERunCases,
} from "../../../lib/flowpilot-e2e";

const START_TIMEOUT_MS = 60_000;
// Generous per-case ceiling: slow runs must COMPLETE so their receipts show where the time went
// (plan-step timestamps + generation-run windows); a timeout destroys exactly that evidence.
const RUN_TIMEOUT_MS = 35 * 60_000;
// After a case timeout the shared chat is still streaming; give cancellation this long to land
// before abandoning the remaining cases.
const CANCEL_TIMEOUT_MS = 30_000;
const CREATED_APP_TIMEOUT_MS = 20_000;
const COMPILER_RECEIPT_SETTLE_TIMEOUT_MS = 30_000;
const CLI_CALLBACK_ATTEMPTS = 3;
const CLI_CALLBACK_TIMEOUT_MS = 15_000;
const MAX_CLI_REPEAT = 20;

type RunPhase =
	| "idle"
	| "preparing"
	| "running"
	| "collecting"
	| "passed"
	| "failed";

type RunnerIssue = FlowPilotE2ERunnerIssue;
type AssistantTrace = FlowPilotE2EAssistantTrace;

class FlowPilotE2EPartialRunError extends Error {
	readonly artifacts: FlowPilotE2EArtifact[];

	constructor(message: string, artifacts: readonly FlowPilotE2EArtifact[]) {
		super(message);
		this.name = "FlowPilotE2EPartialRunError";
		this.artifacts = [...artifacts];
	}
}

interface CaseRunState {
	phase: RunPhase;
	startedAt?: number;
	artifact?: FlowPilotE2EArtifact;
	error?: string;
}

declare global {
	interface Window {
		flowPilotE2E?: {
			cases: readonly FlowPilotE2ECaseId[];
			run: (
				options?: FlowPilotE2ERunOptions,
			) => Promise<FlowPilotE2EArtifact[]>;
		};
		/**
		 * Survives React hydration recovery/remounts for the lifetime of the webview. A ref is only
		 * component-instance local, so a remount could otherwise launch the same CLI run twice and
		 * let the two fresh-conversation setup paths overwrite each other.
		 */
		flowPilotE2EClaimedCliRunIds?: Set<string>;
	}
}

function errorMessage(error: unknown): string {
	return error instanceof Error ? error.message : String(error);
}

function parseMinimumOverride(value: string | null): number | undefined {
	if (!value?.trim()) return undefined;
	const parsed = Number(value);
	if (!Number.isSafeInteger(parsed) || parsed < 1) {
		throw new Error("FlowScript minimum must be a positive integer.");
	}
	return parsed;
}

function validatedRepeat(value: number | undefined): number {
	const parsed = value ?? 1;
	if (!Number.isSafeInteger(parsed) || parsed < 1 || parsed > MAX_CLI_REPEAT) {
		throw new Error(
			`CLI repeat must be an integer from 1 to ${MAX_CLI_REPEAT}.`,
		);
	}
	return parsed;
}

function parseCliRepeat(value: string | null): number {
	return validatedRepeat(value?.trim() ? Number(value) : undefined);
}

function parseCliCaseIds(
	value: string | null,
): FlowPilotE2ECaseId[] | undefined {
	if (!value?.trim()) return undefined;
	const known = new Set(
		FLOWPILOT_APP_CREATION_CASES.map((caseDefinition) => caseDefinition.id),
	);
	const ids = value
		.split(",")
		.map((id) => id.trim())
		.filter(Boolean);
	for (const id of ids) {
		if (!known.has(id as FlowPilotE2ECaseId)) {
			throw new Error(`Unknown FlowPilot app-creation E2E case: ${id}`);
		}
	}
	return [...new Set(ids)] as FlowPilotE2ECaseId[];
}

function parseCliCallback(value: string | null): URL {
	if (!value) throw new Error("CLI callback URL is missing.");
	const callback = new URL(value);
	if (
		callback.protocol !== "http:" ||
		callback.hostname !== "localhost" ||
		!callback.port
	) {
		throw new Error(
			"CLI callback must be an explicit http://localhost:<port> URL.",
		);
	}
	return callback;
}

function delay(ms: number): Promise<void> {
	return new Promise((resolve) => setTimeout(resolve, ms));
}

async function postCliEnvelope(
	callback: URL,
	envelope: FlowPilotE2ECliEnvelope,
): Promise<void> {
	let lastError: unknown;
	for (let attempt = 1; attempt <= CLI_CALLBACK_ATTEMPTS; attempt += 1) {
		const controller = new AbortController();
		const timeout = window.setTimeout(
			() => controller.abort(),
			CLI_CALLBACK_TIMEOUT_MS,
		);
		try {
			const response = await fetch(callback, {
				method: "POST",
				headers: { "content-type": "application/json" },
				body: JSON.stringify(envelope),
				signal: controller.signal,
			});
			if (!response.ok) {
				throw new Error(`CLI callback returned HTTP ${response.status}.`);
			}
			return;
		} catch (error) {
			lastError = error;
			if (attempt < CLI_CALLBACK_ATTEMPTS) await delay(500 * attempt);
		} finally {
			window.clearTimeout(timeout);
		}
	}
	throw lastError instanceof Error
		? lastError
		: new Error(String(lastError ?? "CLI callback failed."));
}

function waitForChatState(
	predicate: (state: ReturnType<typeof useGlobalChatStore.getState>) => boolean,
	timeoutMs: number,
	label: string,
): Promise<void> {
	return new Promise((resolve, reject) => {
		let settled = false;
		let unsubscribe = () => {};
		const finish = (error?: Error) => {
			if (settled) return;
			settled = true;
			clearTimeout(timeout);
			unsubscribe();
			if (error) reject(error);
			else resolve();
		};
		const check = (state: ReturnType<typeof useGlobalChatStore.getState>) => {
			if (predicate(state)) finish();
		};
		const timeout = setTimeout(
			() => finish(new Error(`${label} timed out after ${timeoutMs / 1000}s`)),
			timeoutMs,
		);
		unsubscribe = useGlobalChatStore.subscribe(check);
		check(useGlobalChatStore.getState());
	});
}

function runSuffix(caseId: FlowPilotE2ECaseId): string {
	const stamp = new Date()
		.toISOString()
		.replace(/[-:]/g, "")
		.replace(/\.\d{3}Z$/, "Z");
	const nonce = Math.random().toString(36).slice(2, 6);
	return `[E2E ${stamp} ${caseId} ${nonce}]`;
}

function answerForUnexpectedAsk(
	prompt: NonNullable<
		ReturnType<typeof useGlobalChatStore.getState>["toolPrompt"]
	>,
): unknown {
	const ask = prompt.ask;
	if (ask?.defaultValue !== undefined) return ask.defaultValue;
	const first = ask?.choices[0];
	const firstValue = first?.value ?? first?.label;
	if (ask?.mode === "multiple_choice") {
		return firstValue === undefined ? [] : [firstValue];
	}
	return firstValue ?? "Use a safe placeholder suitable for this benchmark.";
}

function appendRunnerFailures(
	report: FlowPilotE2ERunReport,
	issues: readonly RunnerIssue[],
): FlowPilotE2ERunReport {
	if (issues.length === 0) return report;
	const extra: FlowPilotE2ECheck[] = issues.map((issue, index) => ({
		code: `${issue.code}.${index + 1}`,
		status: "fail",
		message: issue.message,
		path: "runner",
		expected: false,
		actual: true,
	}));
	const checks = [...report.checks, ...extra];
	const failures = [...report.failures, ...extra];
	return {
		...report,
		passed: false,
		checks,
		failures,
		summary: {
			checks: checks.length,
			passed: checks.length - failures.length,
			failed: failures.length,
		},
	};
}

function nativeDiagnostics(
	diagnostics: readonly {
		message: string;
		severity: string;
		line?: number;
		col?: number;
	}[],
) {
	return diagnostics.map((diagnostic) => ({
		message: diagnostic.message,
		severity: diagnostic.severity,
		line: diagnostic.line,
		column: diagnostic.col,
	}));
}

function boardNodeInventory(board: {
	nodes?: Record<string, { name?: string }>;
	layers?: Record<string, { nodes?: Record<string, { name?: string }> }>;
}): { ids: string[]; types: string[] } {
	const nodes = [
		...Object.entries(board.nodes ?? {}),
		...Object.values(board.layers ?? {}).flatMap((layer) =>
			Object.entries(layer.nodes ?? {}),
		),
	];
	const ids = new Set(nodes.map(([nodeId]) => nodeId));
	const types = new Set(
		nodes
			.map(([, node]) => node.name?.trim())
			.filter((nodeType): nodeType is string => Boolean(nodeType)),
	);
	return { ids: [...ids], types: [...types] };
}

async function waitForGenerationReceipts(
	conversationId: string,
	appId: string,
): Promise<readonly FlowScriptGenerationRunReceipt[]> {
	const deadline = Date.now() + COMPILER_RECEIPT_SETTLE_TIMEOUT_MS;
	let runs = flowScriptGenerationRunsForConversation(conversationId).filter(
		(run) => run.appId === appId,
	);
	while (
		runs.some((run) => run.outcome === "awaiting_approval") &&
		Date.now() < deadline
	) {
		await delay(250);
		runs = flowScriptGenerationRunsForConversation(conversationId).filter(
			(run) => run.appId === appId,
		);
	}
	return runs;
}

async function collectOr<T>(
	issues: RunnerIssue[],
	code: string,
	fallback: T,
	read: () => Promise<T>,
): Promise<T> {
	try {
		return await read();
	} catch (error) {
		issues.push({
			code: `collector.${code}`,
			message: `${code} collection failed: ${errorMessage(error)}`,
		});
		return fallback;
	}
}

async function collectSnapshot(
	backend: ReturnType<typeof useBackend>,
	appId: string,
	appName: string,
	authoredFlowScript: string | undefined,
	authoredFlowScriptStatus: string | undefined,
	authoredFlowScriptCompletion: string | undefined,
	model: FlowPilotE2EModelConfig | undefined,
	issues: RunnerIssue[],
	flowScriptGenerationRuns: readonly FlowScriptGenerationRunReceipt[],
): Promise<FlowPilotAppCreationSnapshot> {
	const lintFlowScript = backend.boardState.lintFlowScript;
	const checkFlowScriptReconcile = backend.boardState.checkFlowScriptReconcile;
	const [boards, pageEntries, widgetEntries, systemTables, userTables, events] =
		await Promise.all([
			collectOr(issues, "boards", [], () =>
				backend.boardState.getBoards(appId),
			),
			collectOr(issues, "pages", [], () => backend.pageState.getPages(appId)),
			collectOr(issues, "widgets", [], () =>
				backend.widgetState.getWidgets(appId),
			),
			collectOr(issues, "tables.project", [], () =>
				backend.dbState.listTables(appId),
			),
			collectOr(issues, "tables.user", [], () =>
				backend.dbState.listTablesUser(appId),
			),
			collectOr(issues, "events", [], () =>
				backend.eventState.getEvents(appId, true),
			),
		]);

	const [boardSnapshots, pages, widgets, authoredLintDiagnostics] =
		await Promise.all([
			Promise.all(
				boards.map(async (board) => {
					const boardAuthored = authoredFlowScriptEvidence(
						flowScriptGenerationRuns.filter(
							(run) => run.appId === appId && run.boardId === board.id,
						),
					);
					// Anchors preserve stable identity, so reconciling the persisted canonical
					// source against the same board must be a zero-command round trip.
					const flowScript = await collectOr<string | undefined>(
						issues,
						`board.${board.id}.flowscript`,
						undefined,
						() =>
							backend.boardState.getFlowScript(
								appId,
								board.id,
								undefined,
								true,
							),
					);
					const lint =
						flowScript && lintFlowScript
							? await collectOr(
									issues,
									`board.${board.id}.lint`,
									undefined,
									() => lintFlowScript(flowScript),
								)
							: undefined;
					const reconcile =
						flowScript && checkFlowScriptReconcile
							? await collectOr(
									issues,
									`board.${board.id}.reconcile`,
									undefined,
									() => checkFlowScriptReconcile(appId, board.id, flowScript),
								)
							: undefined;
					const nodeInventory = boardNodeInventory(board);
					return {
						id: board.id,
						name: board.name,
						nodeCount: nodeInventory.ids.length,
						nodeIds: nodeInventory.ids,
						nodeTypes: nodeInventory.types,
						flowScript,
						authoredFlowScript: boardAuthored.source,
						lintDiagnostics: lint ? nativeDiagnostics(lint) : undefined,
						reconcile: reconcile
							? {
									parseValid: reconcile.parse_valid,
									reconcileValid: reconcile.reconcile_valid,
									idempotent: reconcile.idempotent,
									commandCount: reconcile.command_count,
									corrections: reconcile.corrections,
									diagnostics: reconcile.diagnostics,
								}
							: undefined,
					};
				}),
			),
			Promise.all(
				pageEntries.map(async (entry) => {
					const page = await collectOr(
						issues,
						`page.${entry.pageId}`,
						undefined,
						() => backend.pageState.getPage(appId, entry.pageId, entry.boardId),
					);
					if (!page) {
						return {
							id: entry.pageId,
							name: entry.name,
							boardId: entry.boardId,
						};
					}
					return {
						id: page.id,
						name: page.name,
						route: page.route,
						boardId: page.boardId ?? entry.boardId,
						onLoadEventId: page.onLoadEventId,
						onUnloadEventId: page.onUnloadEventId,
						onIntervalEventId: page.onIntervalEventId,
						content: page.content,
						widgetRefs: page.widgetRefs,
					};
				}),
			),
			Promise.all(
				widgetEntries.map(async ([, widgetId, metadata]) => {
					const widget = await collectOr(
						issues,
						`widget.${widgetId}`,
						undefined,
						() => backend.widgetState.getWidget(appId, widgetId),
					);
					if (!widget) {
						return { id: widgetId, name: metadata?.name ?? widgetId };
					}
					return {
						id: widget.id,
						name: widget.name,
						actions: widget.actions?.map((action) => ({
							id: action.id,
							name: action.label,
							label: action.label,
						})),
					};
				}),
			),
			authoredFlowScript && lintFlowScript
				? collectOr(issues, "authored_flowscript.lint", undefined, () =>
						lintFlowScript(authoredFlowScript).then(nativeDiagnostics),
					)
				: Promise.resolve(undefined),
		]);

	return {
		appId,
		appName,
		model,
		authoredFlowScript,
		authoredFlowScriptStatus,
		authoredFlowScriptCompletion,
		authoredLintDiagnostics,
		flowScriptGenerationRuns,
		boards: boardSnapshots,
		pages,
		widgets,
		tables: [...new Set([...systemTables, ...userTables])],
		events: events.map((event) => ({
			id: event.id,
			name: event.name,
			boardId: event.board_id,
			nodeId: event.node_id,
			eventType: event.event_type,
			pageId: event.default_page_id ?? undefined,
		})),
	};
}

async function findCreatedApp(
	backend: ReturnType<typeof useBackend>,
	beforeIds: ReadonlySet<string>,
	expectedName: string,
	appRefs: readonly string[],
): Promise<{ appId: string; appName: string }> {
	const deadline = Date.now() + CREATED_APP_TIMEOUT_MS;
	let lastCandidates: string[] = [];
	while (Date.now() < deadline) {
		const apps = await backend.appState.getApps();
		const entries = apps.map(([app, meta]) => ({
			appId: app.id,
			appName: meta?.name ?? app.id,
			isNew: !beforeIds.has(app.id),
		}));
		const exact = entries.find(
			(entry) => entry.isNew && entry.appName === expectedName,
		);
		if (exact) return exact;
		const referenced = entries.find(
			(entry) => entry.isNew && appRefs.includes(entry.appId),
		);
		if (referenced) return referenced;
		const created = entries.filter((entry) => entry.isNew);
		if (created.length === 1) return created[0];
		lastCandidates = created.map(
			(entry) => `${entry.appName} (${entry.appId})`,
		);
		await delay(500);
	}
	throw new Error(
		`Could not identify the created app ${JSON.stringify(expectedName)}${
			lastCandidates.length > 0
				? `; new candidates: ${lastCandidates.join(", ")}`
				: "; no new app appeared"
		}.`,
	);
}

function assistantTrace(): AssistantTrace | undefined {
	const message = useGlobalChatStore.getState().messages.at(-1);
	if (!message) return undefined;
	return {
		id: message.id,
		content: message.inner?.content,
		appRefs: message.app_refs,
		planSteps: message.plan_steps,
		usageStats: message.usage_stats,
		debugReport: message.debug_report,
	};
}

function downloadJson(fileName: string, value: unknown) {
	const blob = new Blob([JSON.stringify(value, null, 2)], {
		type: "application/json",
	});
	const url = URL.createObjectURL(blob);
	const anchor = document.createElement("a");
	anchor.href = url;
	anchor.download = fileName;
	anchor.click();
	URL.revokeObjectURL(url);
}

function StatusIcon({ phase }: { phase: RunPhase }) {
	if (phase === "preparing" || phase === "running" || phase === "collecting") {
		return <Loader2 className="h-4 w-4 animate-spin text-primary" />;
	}
	if (phase === "passed") {
		return <CheckCircle2 className="h-4 w-4 text-emerald-500" />;
	}
	if (phase === "failed") {
		return <XCircle className="h-4 w-4 text-destructive" />;
	}
	return <span className="h-2 w-2 rounded-full bg-muted-foreground/30" />;
}

export default function FlowPilotE2EPage() {
	const backend = useBackend();
	const codex = useCopilotSDK("codex");
	const [selected, setSelected] = useState<Set<FlowPilotE2ECaseId>>(
		() => new Set(FLOWPILOT_APP_CREATION_SMOKE_CASES.map((item) => item.id)),
	);
	const [minimumOverride, setMinimumOverride] = useState("");
	const [runs, setRuns] = useState<
		Partial<Record<FlowPilotE2ECaseId, CaseRunState>>
	>({});
	const [isSuiteRunning, setIsSuiteRunning] = useState(false);
	const autoRunStarted = useRef(false);
	const cliRunStarted = useRef(false);
	const runningRef = useRef(false);

	// Prevent the chat surface's best-effort history restoration from racing the benchmark's fresh
	// conversation. The first benchmark message immediately persists its own replacement key.
	useLayoutEffect(() => {
		if (!useGlobalChatStore.getState().isStreaming) {
			sessionStorage.removeItem(LAST_CONVERSATION_KEY);
		}
	}, []);

	const setRun = useCallback(
		(caseId: FlowPilotE2ECaseId, patch: Partial<CaseRunState>) => {
			setRuns((current) => ({
				...current,
				[caseId]: { phase: "idle", ...current[caseId], ...patch },
			}));
		},
		[],
	);

	const ensureModel = useCallback(async () => {
		await codex.start({ backend: "codex", useStdio: true });
		const { invoke } = await import("@tauri-apps/api/core");
		const models = await invoke<CopilotModel[]>(
			"flowpilot_agent_backend_list_models",
			{ backend: "codex" },
		);
		const terra = models.find(
			(model) => model.id === FLOWPILOT_E2E_DEFAULT_MODEL.model,
		);
		if (!terra) {
			throw new Error(
				`Codex model ${FLOWPILOT_E2E_DEFAULT_MODEL.model} is unavailable. Available: ${
					models.map((model) => model.id).join(", ") || "none"
				}.`,
			);
		}
		if (
			terra.supportedReasoningEfforts &&
			!terra.supportedReasoningEfforts.some(
				(option) => option.id === FLOWPILOT_E2E_DEFAULT_MODEL.reasoningEffort,
			)
		) {
			throw new Error(
				`${terra.id} does not advertise ${FLOWPILOT_E2E_DEFAULT_MODEL.reasoningEffort} reasoning.`,
			);
		}
	}, [codex]);

	const runCases = useCallback(
		async (
			caseDefinitions: readonly FlowPilotE2ECaseDefinition[],
			minimum?: number,
		): Promise<FlowPilotE2EArtifact[]> => {
			if (!FLOWPILOT_DEBUG_ENABLED) {
				throw new Error(
					"FlowPilot app-creation E2E requires a development build so compiler receipts and traces can be captured; no model request was started.",
				);
			}
			const initialChat = useGlobalChatStore.getState();
			if (runningRef.current || initialChat.isStreaming) {
				throw new Error("FlowPilot is already running.");
			}
			if (initialChat.toolPrompt) {
				throw new Error(
					"Resolve the existing FlowPilot prompt before starting E2E.",
				);
			}
			runningRef.current = true;
			setIsSuiteRunning(true);
			const original = useGlobalChatStore.getState();
			const originalSelection = {
				provider: original.provider,
				model: original.selectedModelId,
				reasoning: original.reasoningEffort,
				autoMode: original.autoMode,
			};
			const artifacts: FlowPilotE2EArtifact[] = [];

			try {
				await ensureModel();
				for (const caseDefinition of caseDefinitions) {
					const startedAt = Date.now();
					const built = buildCasePrompt(
						caseDefinition,
						runSuffix(caseDefinition.id),
						minimum === undefined
							? undefined
							: { minFlowScriptNonWhitespaceChars: minimum },
					);
					const issues: RunnerIssue[] = [];
					const suppressedNavigations: string[] = [];
					const handledPrompts = new Set<string>();
					let snapshot: FlowPilotAppCreationSnapshot | undefined;
					let report: FlowPilotE2ERunReport | undefined;
					let trace: AssistantTrace | undefined;
					let generationRuns: readonly FlowScriptGenerationRunReceipt[] = [];
					let conversationId: string | undefined;
					let failure: string | undefined;
					let observedModel:
						| {
								provider: string;
								model: string;
								reasoningEffort: string;
						  }
						| undefined;
					let snapshotModel: FlowPilotE2EModelConfig | undefined;

					setRun(caseDefinition.id, {
						phase: "preparing",
						startedAt,
						error: undefined,
						artifact: undefined,
					});

					const guard = useGlobalChatStore.subscribe((state) => {
						if (state.pendingNavigation) {
							suppressedNavigations.push(state.pendingNavigation);
							state.setPendingNavigation(null);
						}
						const prompt = state.toolPrompt;
						if (!prompt || handledPrompts.has(prompt.id)) return;
						if (prompt.kind === "ask") {
							handledPrompts.add(prompt.id);
							issues.push({
								code: "runner.unexpected_ask",
								message: `The benchmark required no questions, but ${prompt.toolName} asked: ${prompt.description ?? prompt.title}`,
							});
							prompt.respond({ answer: answerForUnexpectedAsk(prompt) });
						} else if (prompt.destructive) {
							handledPrompts.add(prompt.id);
							issues.push({
								code: "runner.destructive_approval",
								message: `The run requested destructive approval for ${prompt.toolName}; the runner denied it.`,
							});
							prompt.respond({ approved: false, remember: false });
						}
					});

					try {
						const before = await backend.appState.getApps();
						const beforeIds = new Set(before.map(([app]) => app.id));
						const chat = useGlobalChatStore.getState();
						chat.newConversation();
						conversationId = useGlobalChatStore.getState().activeConversationId;
						useGlobalChatStore.setState({
							draft: null,
							pendingNavigation: null,
							toolPrompt: null,
						});
						chat.selectProvider(FLOWPILOT_E2E_DEFAULT_MODEL.provider);
						chat.selectModel(FLOWPILOT_E2E_DEFAULT_MODEL.model);
						chat.selectReasoningEffort(
							FLOWPILOT_E2E_DEFAULT_MODEL.reasoningEffort,
						);
						chat.setAutoMode(true);
						const configured = useGlobalChatStore.getState();
						if (
							configured.provider !== FLOWPILOT_E2E_DEFAULT_MODEL.provider ||
							configured.selectedModelId !==
								FLOWPILOT_E2E_DEFAULT_MODEL.model ||
							configured.reasoningEffort !==
								FLOWPILOT_E2E_DEFAULT_MODEL.reasoningEffort
						) {
							throw new Error(
								"Could not configure Codex Terra with high reasoning.",
							);
						}
						chat.setDraft({
							prompt: built.prompt,
							modelId: FLOWPILOT_E2E_DEFAULT_MODEL.model,
						});

						setRun(caseDefinition.id, { phase: "running" });
						await waitForChatState(
							(state) => state.isStreaming,
							START_TIMEOUT_MS,
							`Starting ${caseDefinition.id}`,
						);
						const activeModel = useGlobalChatStore.getState();
						const activeSelection =
							activeModel.activeTurnSelection ?? activeModel;
						observedModel = {
							provider: activeSelection.provider,
							model: activeSelection.selectedModelId,
							reasoningEffort: activeSelection.reasoningEffort,
						};
						if (
							observedModel.provider === FLOWPILOT_E2E_DEFAULT_MODEL.provider &&
							(observedModel.reasoningEffort === "low" ||
								observedModel.reasoningEffort === "medium" ||
								observedModel.reasoningEffort === "high")
						) {
							snapshotModel = {
								provider: "codex",
								model: observedModel.model,
								reasoningEffort: observedModel.reasoningEffort,
							};
						}
						if (
							observedModel.provider !== FLOWPILOT_E2E_DEFAULT_MODEL.provider ||
							observedModel.model !== FLOWPILOT_E2E_DEFAULT_MODEL.model ||
							observedModel.reasoningEffort !==
								FLOWPILOT_E2E_DEFAULT_MODEL.reasoningEffort
						) {
							issues.push({
								code: "runner.model_mismatch",
								message: `The live turn started as ${observedModel.provider}/${observedModel.model}/${observedModel.reasoningEffort || "auto"}, not codex/${FLOWPILOT_E2E_DEFAULT_MODEL.model}/${FLOWPILOT_E2E_DEFAULT_MODEL.reasoningEffort}.`,
							});
						}
						await waitForChatState(
							(state) => !state.isStreaming,
							RUN_TIMEOUT_MS,
							`Running ${caseDefinition.id}`,
						);
						useGlobalChatStore.getState().setPendingNavigation(null);
						setRun(caseDefinition.id, { phase: "collecting" });

						trace = assistantTrace();
						if (!trace) {
							issues.push({
								code: "runner.missing_assistant_trace",
								message: "The completed turn has no persisted assistant trace.",
							});
						}
						const debugReport = trace?.debugReport as
							| {
									outcome?: string;
									provider?: string;
									model?: string;
									reasoning_effort?: string;
							  }
							| undefined;
						const debugOutcome = debugReport?.outcome;
						if (!trace?.debugReport) {
							issues.push({
								code: "runner.missing_debug_report",
								message: "The completed turn has no FlowPilot debug report.",
							});
						} else if (debugOutcome !== "ok") {
							issues.push({
								code: "runner.agent_outcome",
								message: `The agent debug report ended with outcome ${debugOutcome ?? "missing"}.`,
							});
						}
						if (
							debugReport &&
							(debugReport.provider !== FLOWPILOT_E2E_DEFAULT_MODEL.provider ||
								debugReport.model !== FLOWPILOT_E2E_DEFAULT_MODEL.model ||
								debugReport.reasoning_effort !==
									FLOWPILOT_E2E_DEFAULT_MODEL.reasoningEffort)
						) {
							issues.push({
								code: "runner.debug_model_mismatch",
								message: `The persisted trace records ${debugReport.provider ?? "missing"}/${debugReport.model ?? "missing"}/${debugReport.reasoning_effort ?? "missing"}, not codex/${FLOWPILOT_E2E_DEFAULT_MODEL.model}/${FLOWPILOT_E2E_DEFAULT_MODEL.reasoningEffort}.`,
							});
						}
						const appRefs = trace?.appRefs ?? [];
						const created = await findCreatedApp(
							backend,
							beforeIds,
							built.expectedAppName,
							appRefs,
						);
						let createdAppName = created.appName;
						try {
							const metadata = await backend.appState.getAppMeta(created.appId);
							if (metadata.name?.trim()) createdAppName = metadata.name;
						} catch {
							// The tuple metadata is still enough to report a deterministic mismatch.
						}
						generationRuns = conversationId
							? await waitForGenerationReceipts(conversationId, created.appId)
							: [];
						const compiledAuthored = authoredFlowScriptEvidence(generationRuns);
						const workspace = useGlobalChatStore.getState().flowscriptWorkspace;
						snapshot = await collectSnapshot(
							backend,
							created.appId,
							createdAppName,
							compiledAuthored.source ?? workspace?.source,
							compiledAuthored.status ?? workspace?.status,
							compiledAuthored.completion ?? workspace?.completion,
							snapshotModel,
							issues,
							generationRuns,
						);
						report = appendRunnerFailures(
							evaluateAppCreationCase(built.caseDefinition, snapshot),
							issues,
						);
					} catch (error) {
						failure = errorMessage(error);
					} finally {
						guard();
						useGlobalChatStore.getState().setPendingNavigation(null);
					}
					if (generationRuns.length === 0 && conversationId) {
						generationRuns =
							flowScriptGenerationRunsForConversation(conversationId);
					}

					const artifact: FlowPilotE2EArtifact = {
						schema: "flowpilot.app-creation-e2e-artifact/v1",
						generatedAt: new Date().toISOString(),
						durationMs: Date.now() - startedAt,
						requestedModel: FLOWPILOT_E2E_DEFAULT_MODEL,
						observedModel,
						caseId: caseDefinition.id,
						expectedAppName: built.expectedAppName,
						prompt: built.prompt,
						snapshot,
						flowScriptGenerationRuns: generationRuns,
						assistantTrace: trace,
						runner: { suppressedNavigations, issues },
						report,
						failureFingerprint:
							report && !report.passed
								? appCreationFailureFingerprint(report)
								: undefined,
						error: failure,
					};
					artifacts.push(artifact);
					const passed = Boolean(report?.passed) && !failure;
					setRun(caseDefinition.id, {
						phase: passed ? "passed" : "failed",
						artifact,
						error: failure,
					});

					// A timed-out run is still using the shared chat/backend. Cancel it so the
					// remaining cases can run; abandon the suite only when the stream refuses to
					// stop — one slow case must not silently skip every later case.
					if (useGlobalChatStore.getState().isStreaming) {
						const active = readActiveRun();
						try {
							if (active?.runId) {
								await backend.boardState.cancelCopilotChat?.(active.runId);
							}
						} catch {
							// Best-effort; the bounded wait below decides whether to continue.
						}
						try {
							await waitForChatState(
								(state) => !state.isStreaming,
								CANCEL_TIMEOUT_MS,
								`Cancelling ${caseDefinition.id}`,
							);
						} catch {
							break;
						}
					}
				}
			} finally {
				const chat = useGlobalChatStore.getState();
				if (!chat.isStreaming) {
					chat.selectProvider(originalSelection.provider);
					chat.selectModel(originalSelection.model);
					chat.selectReasoningEffort(originalSelection.reasoning);
					chat.setAutoMode(originalSelection.autoMode);
				}
				runningRef.current = false;
				setIsSuiteRunning(false);
			}
			return artifacts;
		},
		[backend, ensureModel, setRun],
	);

	const runRequestedCases = useCallback(
		async (options: FlowPilotE2ERunOptions = {}) => {
			const definitions = resolveFlowPilotE2ERunCases(options);
			const repeat = validatedRepeat(options.repeat);
			const artifacts: FlowPilotE2EArtifact[] = [];
			try {
				for (let round = 0; round < repeat; round += 1) {
					for (const caseDefinition of definitions) {
						const completed = await runCases(
							[caseDefinition],
							options.minFlowScriptNonWhitespaceChars,
						);
						artifacts.push(...completed);
						if (
							options.failFast &&
							(completed.length !== 1 ||
								completed.some((item) => !flowPilotE2EArtifactPassed(item)))
						) {
							return artifacts;
						}
					}
				}
			} catch (error) {
				throw new FlowPilotE2EPartialRunError(errorMessage(error), artifacts);
			}
			return artifacts;
		},
		[runCases],
	);

	const selectedCases = useMemo(
		() =>
			FLOWPILOT_APP_CREATION_CASES.filter((caseDefinition) =>
				selected.has(caseDefinition.id),
			),
		[selected],
	);

	const startSelected = useCallback(async () => {
		try {
			const parsed = parseMinimumOverride(minimumOverride);
			return await runCases(selectedCases, parsed);
		} catch (error) {
			toast.error(errorMessage(error));
			return [];
		}
	}, [minimumOverride, runCases, selectedCases]);

	useEffect(() => {
		window.flowPilotE2E = {
			cases: FLOWPILOT_APP_CREATION_CASES.map((item) => item.id),
			run: runRequestedCases,
		};
		return () => {
			window.flowPilotE2E = undefined;
		};
	}, [runRequestedCases]);

	useEffect(() => {
		const params = new URLSearchParams(window.location.search);
		if (params.get("cli") !== "1" || cliRunStarted.current) return;
		const claimedRunId = params.get("cliRunId")?.trim() || "missing-run-id";
		let claimedRuns = window.flowPilotE2EClaimedCliRunIds;
		if (!claimedRuns) {
			claimedRuns = new Set<string>();
			window.flowPilotE2EClaimedCliRunIds = claimedRuns;
		}
		if (claimedRuns.has(claimedRunId)) return;
		claimedRuns.add(claimedRunId);
		cliRunStarted.current = true;

		void (async () => {
			const startedAtMs = Date.now();
			const startedAt = new Date(startedAtMs).toISOString();
			const runId = claimedRunId;
			let callback: URL | undefined;
			let caseIds: FlowPilotE2ECaseId[] = [];
			let repeat = 1;
			let minimum: number | undefined;
			let failFast = false;
			let artifacts: FlowPilotE2EArtifact[] = [];
			let failure: string | undefined;

			try {
				callback = parseCliCallback(params.get("callback"));
				if (!/^[A-Za-z0-9_-]{8,128}$/.test(runId)) {
					throw new Error("CLI run id is invalid.");
				}
				const explicitIds = parseCliCaseIds(params.get("cases"));
				const singleId = params.get("case")?.trim() as
					| FlowPilotE2ECaseId
					| undefined;
				const suiteParam = params.get("suite");
				const options: FlowPilotE2ERunOptions = explicitIds?.length
					? { caseIds: explicitIds }
					: singleId
						? { caseId: singleId }
						: {
								suite:
									suiteParam === "full" || suiteParam === "all"
										? "full"
										: "smoke",
							};
				minimum = parseMinimumOverride(params.get("minChars"));
				repeat = parseCliRepeat(params.get("repeat"));
				failFast = params.get("failFast") === "1";
				const definitions = resolveFlowPilotE2ERunCases(options);
				caseIds = definitions.map((caseDefinition) => caseDefinition.id);
				setSelected(new Set(caseIds));
				if (minimum !== undefined) setMinimumOverride(String(minimum));
				artifacts = await runRequestedCases({
					caseIds,
					minFlowScriptNonWhitespaceChars: minimum,
					repeat,
					failFast,
				});
			} catch (error) {
				if (error instanceof FlowPilotE2EPartialRunError) {
					artifacts = error.artifacts;
				}
				failure = errorMessage(error);
				console.error("FlowPilot E2E CLI run failed", error);
			}

			const passedRuns = artifacts.filter(flowPilotE2EArtifactPassed).length;
			const requestedRuns = caseIds.length * repeat;
			const completedAtMs = Date.now();
			const envelope: FlowPilotE2ECliEnvelope = {
				schema: "flowpilot.app-creation-e2e-cli-result/v1",
				runId,
				startedAt,
				completedAt: new Date(completedAtMs).toISOString(),
				durationMs: completedAtMs - startedAtMs,
				selection: {
					caseIds,
					repeat,
					minFlowScriptNonWhitespaceChars: minimum,
					failFast,
				},
				artifacts,
				passed:
					!failure &&
					requestedRuns > 0 &&
					artifacts.length === requestedRuns &&
					passedRuns === requestedRuns,
				summary: {
					requestedRuns,
					completedRuns: artifacts.length,
					passed: passedRuns,
					failed: artifacts.length - passedRuns,
					skipped: Math.max(0, requestedRuns - artifacts.length),
				},
				error: failure,
			};

			if (callback) {
				try {
					await postCliEnvelope(callback, envelope);
				} catch (error) {
					console.error("FlowPilot E2E CLI callback failed", error);
					toast.error(`CLI callback failed: ${errorMessage(error)}`);
				}
			} else {
				toast.error(failure ?? "CLI callback URL is unavailable.");
			}
		})();
	}, [runRequestedCases]);

	useEffect(() => {
		const params = new URLSearchParams(window.location.search);
		if (params.get("cli") === "1" || autoRunStarted.current) return;
		autoRunStarted.current = true;
		try {
			const explicitIds = parseCliCaseIds(params.get("cases"));
			const singleId = params.get("case")?.trim() as
				| FlowPilotE2ECaseId
				| undefined;
			const suiteParam = params.get("suite");
			const options: FlowPilotE2ERunOptions = explicitIds?.length
				? { caseIds: explicitIds }
				: singleId
					? { caseId: singleId }
					: {
							suite:
								suiteParam === "full" || suiteParam === "all"
									? "full"
									: "smoke",
						};
			const definitions = resolveFlowPilotE2ERunCases(options);
			setSelected(new Set(definitions.map((item) => item.id)));
			const minimum = parseMinimumOverride(params.get("minChars"));
			if (minimum !== undefined) setMinimumOverride(String(minimum));
			if (params.get("run") === "1" && definitions.length > 0) {
				void runRequestedCases({
					caseIds: definitions.map((item) => item.id),
					minFlowScriptNonWhitespaceChars: minimum,
					repeat: parseCliRepeat(params.get("repeat")),
					failFast: params.get("failFast") === "1",
				}).catch((error) => toast.error(errorMessage(error)));
			}
		} catch (error) {
			toast.error(errorMessage(error));
		}
	}, [runRequestedCases]);

	const choose = (ids: readonly FlowPilotE2ECaseId[]) =>
		setSelected(new Set(ids));
	const completedArtifacts = Object.values(runs)
		.map((run) => run?.artifact)
		.filter((artifact): artifact is FlowPilotE2EArtifact => Boolean(artifact));

	return (
		<div className="flex min-h-0 flex-1 flex-col gap-3">
			<div className="flex flex-wrap items-center justify-between gap-3 rounded-lg border bg-card/60 px-4 py-3">
				<div className="flex items-center gap-3">
					<div className="rounded-md bg-primary/10 p-2 text-primary">
						<FlaskConical className="h-5 w-5" />
					</div>
					<div>
						<h1 className="text-base font-semibold">
							FlowPilot app-creation E2E
						</h1>
						<p className="text-xs text-muted-foreground">
							Real global chat, tool bridge, persisted artifacts, and
							authoritative FlowScript checks.
						</p>
					</div>
				</div>
				<div className="flex items-center gap-2 text-xs">
					<Badge variant="secondary">Codex</Badge>
					<Badge variant="outline">{FLOWPILOT_E2E_DEFAULT_MODEL.model}</Badge>
					<Badge variant="outline">high reasoning</Badge>
				</div>
			</div>

			<div className="grid min-h-0 flex-1 gap-3 xl:grid-cols-[390px_minmax(0,1fr)]">
				<div className="flex min-h-0 flex-col rounded-lg border bg-card/50">
					<div className="space-y-3 border-b p-3">
						<div className="flex flex-wrap gap-2">
							<Button
								size="sm"
								variant="outline"
								onClick={() =>
									choose(
										FLOWPILOT_APP_CREATION_SMOKE_CASES.map((item) => item.id),
									)
								}
								disabled={isSuiteRunning}
							>
								Smoke
							</Button>
							<Button
								size="sm"
								variant="outline"
								onClick={() =>
									choose(FLOWPILOT_APP_CREATION_CASES.map((item) => item.id))
								}
								disabled={isSuiteRunning}
							>
								All six
							</Button>
							<Button
								size="sm"
								className="ml-auto gap-2"
								onClick={() => void startSelected()}
								disabled={isSuiteRunning || selectedCases.length === 0}
							>
								{isSuiteRunning ? (
									<Loader2 className="h-4 w-4 animate-spin" />
								) : (
									<Play className="h-4 w-4" />
								)}
								Run {selectedCases.length}
							</Button>
						</div>
						<div className="space-y-1.5">
							<Label htmlFor="flowscript-min" className="text-xs">
								FlowScript non-whitespace floor override
							</Label>
							<Input
								id="flowscript-min"
								type="number"
								min={1}
								placeholder="Use each case default"
								value={minimumOverride}
								onChange={(event) => setMinimumOverride(event.target.value)}
								disabled={isSuiteRunning}
								className="h-8"
							/>
						</div>
						{completedArtifacts.length > 0 && (
							<Button
								size="sm"
								variant="ghost"
								className="w-full gap-2"
								onClick={() =>
									downloadJson("flowpilot-app-creation-e2e.json", {
										schema: "flowpilot.app-creation-e2e-suite/v1",
										artifacts: completedArtifacts,
									})
								}
							>
								<Download className="h-4 w-4" /> Export completed runs
							</Button>
						)}
					</div>

					<ScrollArea className="min-h-0 flex-1">
						<div className="space-y-2 p-3">
							{FLOWPILOT_APP_CREATION_CASES.map((caseDefinition) => {
								const run = runs[caseDefinition.id];
								const report = run?.artifact?.report;
								return (
									<div
										key={caseDefinition.id}
										className={cn(
											"rounded-md border p-3 transition-colors",
											selected.has(caseDefinition.id) &&
												"border-primary/40 bg-primary/5",
										)}
									>
										<div className="flex items-start gap-2">
											<input
												type="checkbox"
												className="mt-1 accent-primary"
												checked={selected.has(caseDefinition.id)}
												disabled={isSuiteRunning}
												onChange={(event) => {
													setSelected((current) => {
														const next = new Set(current);
														if (event.target.checked)
															next.add(caseDefinition.id);
														else next.delete(caseDefinition.id);
														return next;
													});
												}}
											/>
											<div className="min-w-0 flex-1">
												<div className="flex items-center gap-2">
													<StatusIcon phase={run?.phase ?? "idle"} />
													<span className="truncate text-sm font-medium">
														{caseDefinition.title}
													</span>
													{caseDefinition.smoke && (
														<Badge variant="secondary" className="text-[10px]">
															smoke
														</Badge>
													)}
												</div>
												<p className="mt-1 text-xs text-muted-foreground">
													{caseDefinition.description}
												</p>
											</div>
										</div>

										{report && (
											<div className="mt-2 flex items-center gap-2 text-xs">
												<span>
													{report.summary.passed}/{report.summary.checks} checks
												</span>
												<span className="text-muted-foreground">
													{report.inventory.totalNodes} nodes ·{" "}
													{report.inventory.pages} pages ·{" "}
													{report.inventory.tables} tables
												</span>
											</div>
										)}
										{run?.error && (
											<p className="mt-2 flex gap-1.5 text-xs text-destructive">
												<AlertTriangle className="mt-0.5 h-3.5 w-3.5 shrink-0" />
												{run.error}
											</p>
										)}
										{report && report.failures.length > 0 && (
											<details className="mt-2 text-xs text-destructive">
												<summary className="cursor-pointer">
													{report.failures.length} failed checks
												</summary>
												<ul className="mt-1 list-disc space-y-1 pl-4">
													{report.failures.map((failure, index) => (
														<li key={`${failure.code}-${index}`}>
															{failure.message}
														</li>
													))}
												</ul>
											</details>
										)}
										{run?.artifact && (
											<div className="mt-2 flex gap-1">
												<Button
													size="sm"
													variant="ghost"
													className="h-7 gap-1 px-2 text-xs"
													onClick={() =>
														downloadJson(
															`flowpilot-e2e-${caseDefinition.id}.json`,
															run.artifact,
														)
													}
												>
													<Download className="h-3.5 w-3.5" /> JSON
												</Button>
												<Button
													size="sm"
													variant="ghost"
													className="h-7 gap-1 px-2 text-xs"
													onClick={() => {
														void navigator.clipboard.writeText(
															JSON.stringify(run.artifact, null, 2),
														);
														toast.success("Benchmark artifact copied");
													}}
												>
													<Clipboard className="h-3.5 w-3.5" /> Copy
												</Button>
											</div>
										)}
									</div>
								);
							})}
						</div>
					</ScrollArea>
				</div>

				<div className="min-h-[520px] overflow-hidden rounded-lg border bg-background xl:min-h-0">
					<GlobalChatView />
				</div>
			</div>
		</div>
	);
}
