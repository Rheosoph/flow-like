"use client";

import { useBackend } from "@flow-like/flow-like-ui";
import { FlowPilot } from "@flow-like/flow-like-ui/components/flowpilot/FlowPilot";
import {
	type CopilotStreamEvent,
	createCopilotStreamParser,
} from "@flow-like/flow-like-ui/components/flowpilot/copilot-stream-parser";
import { GlobalChatView } from "@flow-like/flow-like-ui/components/global-chat/global-chat-view";
import type { IBoard } from "@flow-like/flow-like-ui/lib";
import {
	BackendContext,
	type IBoardState,
} from "@flow-like/flow-like-ui/state/backend-state";
import { useGlobalChatStore } from "@flow-like/flow-like-ui/state/global-chat/global-chat-store";
import {
	cancelGlobalChatRun,
	switchGlobalChatBot,
} from "@flow-like/flow-like-ui/state/global-chat/global-chat-stream";
import { useQueryClient } from "@tanstack/react-query";
import { invoke } from "@tauri-apps/api/core";
import { useCallback, useEffect, useMemo, useRef, useState } from "react";

const PROVIDERS = ["claude-code", "codex"] as const;
const PATHS = [
	"flux",
	"flowpilot",
	"bot",
	"use-case",
	"invoke-model",
	"agent-from-model",
] as const;
type Provider = (typeof PROVIDERS)[number];
type CasePath = (typeof PATHS)[number];
type JsonRecord = Record<string, unknown>;

interface Fixture {
	provider: Provider;
	bitId: string;
	bitName: string;
	modelId: string;
	modelName: string;
	appId: string;
	botId: string;
	boardId: string;
	board: IBoard;
}

interface Seed {
	runId: string;
	profileId: string;
	nonce: string;
	fixtures: Fixture[];
	errors: { provider: Provider; message: string }[];
}

interface CaseResult {
	id: string;
	provider: Provider;
	bitId: string;
	modelId: string;
	passed: boolean;
	evidence: JsonRecord;
	error?: string;
}

interface Report {
	schema: "flowpilot.custom-bit-e2e/v1";
	runId: string;
	passed: boolean;
	cases: CaseResult[];
	error?: string;
	progress?: {
		stage: string;
		at: string;
		caseId?: string;
		details?: JsonRecord;
	};
	checkpoints?: NonNullable<Report["progress"]>[];
}

type CopilotResponse = Awaited<ReturnType<IBoardState["copilot_chat"]>>;
interface BoardObservation {
	request?: JsonRecord;
	requestId?: string;
	stream: CopilotStreamEvent[];
	response?: CopilotResponse;
	error?: string;
	reviewJob?: unknown;
	reviewError?: string;
	reviewReady?: boolean;
}

declare global {
	interface Window {
		customBitE2E?: {
			run: () => Promise<Report>;
			report: () => Report | undefined;
		};
	}
}

function errorMessage(error: unknown): string {
	if (error instanceof Error) return error.message;
	if (typeof error === "string") return error;
	return JSON.stringify(error);
}

function assert(condition: unknown, message: string): asserts condition {
	if (!condition) throw new Error(message);
}

function delay(ms: number) {
	return new Promise<void>((resolve) => setTimeout(resolve, ms));
}

async function waitFor<T>(
	read: () => T | undefined,
	signal: AbortSignal,
	label: string,
	timeoutMs = 60_000,
): Promise<T> {
	const deadline = Date.now() + timeoutMs;
	while (Date.now() < deadline) {
		signal.throwIfAborted();
		const value = read();
		if (value !== undefined) return value;
		await delay(50);
	}
	throw new Error(`${label} did not become ready within ${timeoutMs / 1000}s.`);
}

async function bounded<T>(
	operation: (signal: AbortSignal) => Promise<T>,
	timeoutMs: number,
): Promise<T> {
	const controller = new AbortController();
	let timer: ReturnType<typeof setTimeout> | undefined;
	try {
		return await Promise.race([
			operation(controller.signal),
			new Promise<never>((_, reject) => {
				timer = setTimeout(() => {
					const error = new Error(
						`Live case timed out after ${timeoutMs / 1000}s.`,
					);
					controller.abort(error);
					reject(error);
				}, timeoutMs);
			}),
		]);
	} finally {
		clearTimeout(timer);
		controller.abort();
	}
}

async function postReport(report: Report, progress: boolean) {
	const params = new URLSearchParams(window.location.search);
	if (params.get("cli") !== "1") return;
	const callback = new URL(params.get("callback") ?? "");
	assert(
		callback.protocol === "http:" &&
			callback.hostname === "localhost" &&
			callback.port,
		"The callback must be an explicit http://localhost:<port> URL.",
	);
	if (progress) callback.searchParams.set("progress", "1");
	const response = await fetch(callback, {
		method: "POST",
		headers: { "content-type": "application/json" },
		body: JSON.stringify(report),
		signal: AbortSignal.timeout(10_000),
	});
	assert(response.ok, `Callback returned HTTP ${response.status}.`);
}

/** A real desktop acceptance runner. Observers delegate to the installed backend unchanged. */
export function CustomBitE2ERunner() {
	const backend = useBackend();
	const queryClient = useQueryClient();
	const [panel, setPanel] = useState<{
		path: "flux" | "flowpilot" | "bot";
		fixture: Fixture;
	}>();
	const [report, setReport] = useState<Report>();
	const [status, setStatus] = useState(
		"Waiting for the isolated desktop runtime.",
	);
	const hostRef = useRef<HTMLDivElement>(null);
	const boardObservation = useRef<BoardObservation | undefined>(undefined);
	const running = useRef<Promise<Report> | undefined>(undefined);
	const latestReport = useRef<Report | undefined>(undefined);
	const autorun = useRef(false);

	const observedBackend = useMemo(() => {
		const boardState = Object.create(backend.boardState) as IBoardState;
		boardState.copilot_chat = async (...args) => {
			const observed = boardObservation.current;
			const parser = createCopilotStreamParser();
			if (observed) {
				observed.requestId = args[19];
				observed.request = {
					scope: args[0],
					boardId: args[1]?.id,
					modelId: args[11],
					appId: args[21],
					requestId: args[19],
					userPrompt: args[20],
				};
			}
			const onToken = args[10];
			args[10] = (token) => {
				if (observed && observed.stream.length < 256) {
					for (const event of parser.push(token)) {
						if (event.type !== "text" && JSON.stringify(event).length <= 32_768)
							observed.stream.push(event);
					}
				}
				onToken?.(token);
			};
			try {
				const response = await backend.boardState.copilot_chat.apply(
					backend.boardState,
					args,
				);
				if (observed) observed.response = response;
				return response;
			} catch (error) {
				if (observed) observed.error = errorMessage(error);
				throw error;
			}
		};
		const createReview = backend.boardState.createBoardEditJob;
		if (createReview) {
			boardState.createBoardEditJob = async (...args) => {
				const observed = boardObservation.current;
				try {
					const job = await createReview.apply(backend.boardState, args);
					if (observed) observed.reviewJob = job;
					return job;
				} catch (error) {
					if (observed) observed.reviewError = errorMessage(error);
					throw error;
				} finally {
					if (observed) observed.reviewReady = true;
				}
			};
		}
		return Object.assign(Object.create(backend), {
			boardState,
		}) as typeof backend;
	}, [backend]);

	const run = useCallback((): Promise<Report> => {
		if (running.current) return running.current;
		const task = async (): Promise<Report> => {
			const params = new URLSearchParams(window.location.search);
			const result: Report = {
				schema: "flowpilot.custom-bit-e2e/v1",
				runId: params.get("cliRunId") ?? "interactive",
				passed: false,
				cases: [],
			};
			latestReport.current = result;
			const publish = async (progress = true) => {
				const snapshot = { ...result, cases: [...result.cases] };
				latestReport.current = snapshot;
				setReport(snapshot);
				await postReport(snapshot, progress).catch((error) =>
					setStatus(`Callback: ${errorMessage(error)}`),
				);
			};
			const checkpoint = async (
				stage: string,
				details?: JsonRecord,
				caseId?: string,
			) => {
				result.progress = {
					stage,
					at: new Date().toISOString(),
					caseId,
					details,
				};
				result.checkpoints = [...(result.checkpoints ?? []), result.progress];
				await publish();
			};
			const addFailure = async (
				provider: Provider,
				path: CasePath,
				error: unknown,
				fixture?: Fixture,
			) => {
				result.cases.push({
					id: `${provider}:${path}`,
					provider,
					bitId: fixture?.bitId ?? "unavailable",
					modelId: fixture?.modelId ?? "unavailable",
					passed: false,
					evidence: {},
					error: errorMessage(error),
				});
				await publish();
			};
			try {
				setStatus(
					"Verifying isolated storage and discovering live provider models.",
				);
				await checkpoint("isolation-check-start");
				const gate = await invoke<{ available: boolean; policy: string }>(
					"intake_e2e_runtime_status",
				);
				assert(
					gate.available && gate.policy === "flowpilot.intake-local-runtime/v1",
					"This runner requires the isolated native E2E runtime.",
				);
				await checkpoint("isolation-check-complete", gate);
				await checkpoint("seed-start");
				const seed = await invoke<Seed>("custom_bit_e2e_seed");
				assert(
					!params.get("cliRunId") || seed.runId === params.get("cliRunId"),
					"The native fixture belongs to another controller run.",
				);
				result.runId = seed.runId;
				await checkpoint("seed-complete", {
					profileId: seed.profileId,
					fixtures: seed.fixtures.map(
						({ provider, bitId, modelId, appId, botId, boardId }) => ({
							provider,
							bitId,
							modelId,
							appId,
							botId,
							boardId,
						}),
					),
					errors: seed.errors,
				});
				await checkpoint("catalog-refresh-start");
				await queryClient.invalidateQueries();
				await checkpoint("catalog-refresh-complete");
				for (const provider of PROVIDERS) {
					const fixture = seed.fixtures.find(
						(item) => item.provider === provider,
					);
					if (!fixture) {
						const error =
							seed.errors.find((item) => item.provider === provider)?.message ??
							"The runtime did not create a provider fixture.";
						for (const path of PATHS) await addFailure(provider, path, error);
						continue;
					}
					const capture = async (
						path: CasePath,
						operation: (
							evidence: JsonRecord,
							signal: AbortSignal,
						) => Promise<void>,
						timeoutMs = 180_000,
					) => {
						const evidence: JsonRecord = {
							startedAt: new Date().toISOString(),
							identity: {
								provider,
								bitId: fixture.bitId,
								configuredRuntimeModelId: fixture.modelId,
							},
						};
						const item: CaseResult = {
							id: `${provider}:${path}`,
							provider,
							bitId: fixture.bitId,
							modelId: fixture.modelId,
							passed: false,
							evidence,
						};
						setStatus(`Running ${item.id}`);
						await checkpoint(
							"case-start",
							{ provider, bitId: fixture.bitId, modelId: fixture.modelId },
							item.id,
						);
						try {
							await bounded((signal) => operation(evidence, signal), timeoutMs);
							item.passed = true;
						} catch (error) {
							item.error = errorMessage(error);
						} finally {
							evidence.completedAt = new Date().toISOString();
							result.cases.push(item);
							await checkpoint(
								"case-complete",
								{ passed: item.passed, error: item.error },
								item.id,
							);
						}
					};
					const runChat = async (
						path: "flux" | "bot",
						evidence: JsonRecord,
						signal: AbortSignal,
					) => {
						setPanel({ path, fixture });
						await waitFor(
							() =>
								hostRef.current?.dataset.customBitE2ePanel === path &&
								hostRef.current?.dataset.bitId === fixture.bitId
									? true
									: undefined,
							signal,
							"The requested chat panel",
						);
						await waitFor(
							() => hostRef.current?.querySelector("textarea") ?? undefined,
							signal,
							"The real chat composer",
						);
						await switchGlobalChatBot(path === "bot" ? fixture.botId : null);
						if (path === "flux")
							useGlobalChatStore.getState().newConversation();
						const chat = useGlobalChatStore.getState();
						chat.selectProvider("bits");
						chat.selectModel(fixture.bitId);
						chat.selectReasoningEffort("");
						chat.setAutoMode(false);
						const conversationId =
							useGlobalChatStore.getState().activeConversationId;
						const startedAt = Date.now();
						const expectedTool =
							path === "flux" ? "list_apps" : "get_app_detail";
						const prompt =
							path === "flux"
								? `Use list_apps with query exactly ${fixture.appId} to read that app. Return only the exact verification nonce from its description. Do not modify anything.`
								: `Use get_app_detail to read app ${fixture.appId}. Return only the exact verification nonce from its description. Do not modify anything.`;
						evidence.request = {
							conversationId,
							botId: path === "bot" ? fixture.botId : "flux",
							provider: "bits",
							modelId: fixture.bitId,
							prompt,
						};
						await checkpoint(
							"chat-send",
							evidence.request as JsonRecord,
							`${provider}:${path}`,
						);
						signal.throwIfAborted();
						chat.setDraft({
							prompt,
							modelId: fixture.bitId,
							botId: path === "bot" ? fixture.botId : null,
						});
						try {
							const message = await waitFor(
								() =>
									useGlobalChatStore
										.getState()
										.messages.findLast(
											(item) =>
												item.sessionId === conversationId &&
												(item.run_context?.started_at_ms ?? 0) >= startedAt &&
												Boolean(item.run_context?.ended_at_ms),
										),
								signal,
								"The completed real chat response",
								170_000,
							);
							await checkpoint(
								"chat-response",
								{ messageId: message.id, runContext: message.run_context },
								`${provider}:${path}`,
							);
							evidence.response = {
								id: message.id,
								content: message.inner.content,
								runContext: message.run_context,
								debugReport: message.debug_report,
								planSteps: message.plan_steps,
								usage: message.usage_stats,
							};
							evidence.renderedText = hostRef.current?.innerText.slice(-16_384);
							const context = message.run_context;
							const completionModels = (message.usage_stats ?? []).flatMap(
								(stat) => (stat.stats.calls ?? []).map((call) => call.model),
							);
							evidence.identityEvidence = {
								requestedBitId: context?.model_id,
								nativeSelectedBitId: context?.backend_model_id,
								adapterReportedRuntimeModels: completionModels,
							};
							assert(
								context?.model_id === fixture.bitId,
								"The rendered chat sent a different selected Bit.",
							);
							assert(
								context.effective_model_id === fixture.bitId,
								"The chat resolved a different Bit.",
							);
							assert(
								context.backend_model_id === fixture.bitId,
								"The native backend did not announce the selected Bit.",
							);
							assert(
								completionModels.length > 0 &&
									completionModels.every((model) => model === fixture.modelId),
								"Completion usage did not identify the configured runtime model.",
							);
							assert(
								context.outcome === "ok",
								`The chat ended with ${context.outcome}: ${context.error ?? "no error detail"}`,
							);
							assert(
								JSON.stringify(message.inner.content).includes(seed.nonce),
								"The real chat response did not contain the nonce read from the fixture app.",
							);
							const tool = message.debug_report?.events.find(
								(event) =>
									event.kind === "tool" &&
									event.name === expectedTool &&
									event.stage === "tool_end" &&
									event.status === "done",
							);
							assert(
								tool,
								`The chat has no successful ${expectedTool} tool receipt.`,
							);
							assert(
								tool.result_preview?.includes(seed.nonce),
								"The tool result did not contain the fixture nonce.",
							);
							if (path === "flux") {
								const result = JSON.parse(tool.result_preview ?? "{}");
								assert(
									result.query === fixture.appId &&
										result.apps?.some(
											(app: JsonRecord) => app.app_id === fixture.appId,
										),
									"Flux did not query the exact fixture app.",
								);
							}
							evidence.tool = tool;
						} finally {
							const current = useGlobalChatStore.getState();
							evidence.renderedText = hostRef.current?.innerText.slice(-16_384);
							evidence.uiState = {
								provider: current.provider,
								selectedModelId: current.selectedModelId,
								conversationId: current.activeConversationId,
								conversationLoading: current.conversationLoading,
								draftPending: Boolean(current.draft),
							};
							evidence.pendingRuns = Object.values(current.runs)
								.filter((active) => active.conversationId === conversationId)
								.map((active) => ({
									runId: active.runId,
									status: active.status,
									debugReport: active.debugReport,
								}));
							for (const active of Object.values(
								useGlobalChatStore.getState().runs,
							)) {
								if (active.conversationId === conversationId)
									await cancelGlobalChatRun(active.runId);
							}
						}
					};
					await capture("flux", (evidence, signal) =>
						runChat("flux", evidence, signal),
					);
					await capture(
						"flowpilot",
						async (evidence, signal) => {
							const observed: BoardObservation = { stream: [] };
							boardObservation.current = observed;
							setPanel({ path: "flowpilot", fixture });
							try {
								await waitFor(
									() =>
										hostRef.current?.dataset.customBitE2ePanel ===
											"flowpilot" &&
										hostRef.current?.dataset.bitId === fixture.bitId
											? true
											: undefined,
									signal,
									"The requested Board panel",
								);
								const trigger = await waitFor(
									() =>
										hostRef.current?.querySelector<HTMLButtonElement>(
											"button[aria-label^='Provider and model:']",
										) ?? undefined,
									signal,
									"The real Board model picker",
								);
								trigger.click();
								const choice = await waitFor(
									() =>
										Array.from(
											document.querySelectorAll<HTMLButtonElement>(
												"[data-radix-popper-content-wrapper] button",
											),
										).find((button) =>
											button.textContent?.includes(fixture.bitName),
										),
									signal,
									`The persisted ${fixture.bitName} Bit in the picker`,
								);
								choice.click();
								evidence.picker = await waitFor(
									() =>
										trigger
											.getAttribute("aria-label")
											?.includes(fixture.bitName)
											? trigger.getAttribute("aria-label")
											: undefined,
									signal,
									"The explicitly selected Board Bit",
								);
								const textarea = await waitFor(
									() =>
										hostRef.current?.querySelector<HTMLTextAreaElement>(
											"textarea",
										) ?? undefined,
									signal,
									"The Board composer",
								);
								const variable = `e2e_${provider.replaceAll("-", "_")}`;
								const prompt = `Add one string variable named ${variable} with value ${seed.nonce} to this board. Use FlowScript and submit the validated compiler commit for review. Do not add nodes or change anything else.`;
								Object.getOwnPropertyDescriptor(
									HTMLTextAreaElement.prototype,
									"value",
								)?.set?.call(textarea, prompt);
								textarea.dispatchEvent(new Event("input", { bubbles: true }));
								const send = await waitFor(
									() => {
										const button = hostRef.current
											?.querySelector("svg.lucide-send")
											?.closest("button");
										return button && !button.disabled ? button : undefined;
									},
									signal,
									"The Board Send button",
								);
								await checkpoint(
									"board-send",
									{ bitId: fixture.bitId, boardId: fixture.boardId },
									`${provider}:flowpilot`,
								);
								signal.throwIfAborted();
								send.click();
								await waitFor(
									() =>
										observed.response || observed.error ? observed : undefined,
									signal,
									"The real Board compiler response",
									270_000,
								);
								await checkpoint(
									"board-response",
									{
										requestId: observed.requestId,
										error: observed.error,
										hasCommit: Boolean(observed.response?.flow_ir_commit),
									},
									`${provider}:flowpilot`,
								);
								evidence.request = observed.request;
								evidence.response = observed.response;
								evidence.stream = observed.stream;
								evidence.renderedText =
									hostRef.current?.innerText.slice(-16_384);
								assert(
									!observed.error,
									observed.error ?? "Board generation failed.",
								);
								assert(
									observed.request?.modelId === fixture.bitId,
									"The Board sent a different Bit.",
								);
								const resolutions = observed.stream
									.filter((event) => event.type === "model_resolved")
									.map((event) => event.data as JsonRecord);
								evidence.nativeModelResolution = resolutions;
								assert(
									resolutions.length > 0 &&
										resolutions.every(
											(identity) =>
												identity.bit_id === fixture.bitId &&
												identity.provider_name === `custom:${provider}` &&
												identity.configured_model_id === fixture.modelId &&
												identity.model_id === fixture.modelId,
										),
									"The native Board model factory did not resolve the selected Bit and runtime model.",
								);
								const token = observed.response?.flow_ir_commit;
								assert(
									token?.board_id === fixture.boardId,
									"The Board did not stage a compiled commit for its own fixture.",
								);
								await waitFor(
									() => (observed.reviewReady ? true : undefined),
									signal,
									"The real Board review job creation",
								);
								evidence.uiReview = {
									job: observed.reviewJob,
									error: observed.reviewError,
								};
								await checkpoint(
									"board-review-apply-start",
									{
										boardId: fixture.boardId,
										uiReviewError: observed.reviewError,
									},
									`${provider}:flowpilot`,
								);
								const review = await invoke<JsonRecord>(
									"custom_bit_e2e_apply_review",
									{ provider, token },
								);
								evidence.review = review;
								await checkpoint(
									"board-review-apply-complete",
									{ passed: review.passed },
									`${provider}:flowpilot`,
								);
								assert(
									review.passed === true,
									"The native lazy-open review/apply/persistence checks failed.",
								);
								assert(
									!observed.reviewError,
									`The rendered Board failed to create its review: ${observed.reviewError}`,
								);
							} finally {
								evidence.request = observed.request;
								evidence.stream = observed.stream;
								evidence.renderedText =
									hostRef.current?.innerText.slice(-16_384);
								if (!observed.response && observed.requestId)
									await backend.boardState.cancelCopilotChat?.(
										observed.requestId,
									);
								boardObservation.current = undefined;
							}
						},
						300_000,
					);
					await capture(
						"bot",
						async (evidence, signal) => {
							let renderedError: string | undefined;
							try {
								await runChat("bot", evidence, signal);
							} catch (error) {
								renderedError = errorMessage(error);
								evidence.renderedChatError = renderedError;
							}
							signal.throwIfAborted();
							await checkpoint(
								"native-bot-start",
								{ bitId: fixture.bitId, botId: fixture.botId },
								`${provider}:bot`,
							);
							const native = await invoke<JsonRecord>(
								"custom_bit_e2e_run_bot",
								{ provider },
							);
							evidence.nativeBot = native;
							await checkpoint(
								"native-bot-complete",
								{ passed: native.passed },
								`${provider}:bot`,
							);
							assert(
								native.passed === true,
								"The bot capability route did not prove its configured Bit, tool result and nonce.",
							);
							assert(
								!renderedError,
								renderedError ?? "The rendered bot chat failed.",
							);
						},
						390_000,
					);
					setStatus(`Running ${provider} workflow nodes`);
					let workflows: Record<string, JsonRecord>;
					try {
						await checkpoint("native-workflows-start", {
							provider,
							bitId: fixture.bitId,
							cases: ["use-case", "invoke-model", "agent-from-model"].map(
								(path) => `${provider}:${path}`,
							),
						});
						workflows = await bounded(
							() =>
								invoke<Record<string, JsonRecord>>(
									"custom_bit_e2e_run_workflows",
									{ provider },
								),
							390_000,
						);
						await checkpoint("native-workflows-complete", {
							provider,
							results: Object.fromEntries(
								Object.entries(workflows).map(([key, value]) => [
									key,
									{ passed: value.passed, error: value.error },
								]),
							),
						});
					} catch (error) {
						for (const path of [
							"use-case",
							"invoke-model",
							"agent-from-model",
						] as const)
							await addFailure(provider, path, error, fixture);
						continue;
					}
					for (const [path, key] of [
						["use-case", "useCase"],
						["invoke-model", "invokeModel"],
						["agent-from-model", "agentFromModel"],
					] as const) {
						await capture(path, async (evidence) => {
							const native = workflows[key];
							evidence.nativeWorkflow = native;
							assert(
								native?.passed === true,
								errorMessage(
									native?.error ?? `The ${key} workflow checks failed.`,
								),
							);
						});
					}
				}
			} catch (error) {
				result.error = errorMessage(error);
				for (const provider of PROVIDERS)
					for (const path of PATHS) {
						if (!result.cases.some((item) => item.id === `${provider}:${path}`))
							await addFailure(provider, path, error);
					}
			}
			result.passed =
				result.cases.length === PROVIDERS.length * PATHS.length &&
				result.cases.every((item) => item.passed);
			setStatus(
				result.passed
					? "All real custom Bit paths passed."
					: "The live run completed with failures. Inspect each case receipt.",
			);
			await publish(false);
			return result;
		};
		running.current = task();
		return running.current;
	}, [backend, queryClient]);

	useEffect(() => {
		window.customBitE2E = { run, report: () => latestReport.current };
		if (
			new URLSearchParams(window.location.search).get("cli") === "1" &&
			!autorun.current
		) {
			autorun.current = true;
			void run();
		}
		return () => {
			window.customBitE2E = undefined;
		};
	}, [run]);

	return (
		<div className="flex h-full min-h-0 flex-col gap-3 p-4">
			<div className="flex items-center justify-between gap-4">
				<div>
					<h1 className="font-semibold">Live custom Bit acceptance</h1>
					<output className="text-sm text-muted-foreground">{status}</output>
				</div>
				<button
					type="button"
					className="rounded border px-3 py-2 text-sm"
					disabled={Boolean(running.current)}
					onClick={() => void run()}
				>
					Run isolated checks
				</button>
			</div>
			<div className="flex min-h-0 flex-1 gap-4">
				<div className="w-72 shrink-0 overflow-auto text-xs">
					{report?.cases.map((item) => (
						<details key={item.id} className="mb-2 rounded border p-2">
							<summary>
								{item.passed ? "PASS" : "FAIL"} {item.id}
							</summary>
							<pre className="mt-2 whitespace-pre-wrap break-all">
								{JSON.stringify(item, null, 2)}
							</pre>
						</details>
					))}
				</div>
				<div
					ref={hostRef}
					className="relative min-h-0 min-w-0 flex-1 overflow-hidden rounded border"
					data-custom-bit-e2e-panel={panel?.path}
					data-bit-id={panel?.fixture.bitId}
				>
					{panel?.path === "flowpilot" ? (
						<BackendContext.Provider value={observedBackend}>
							<FlowPilot
								key={panel.fixture.boardId}
								agentMode="board"
								forceProvider="bits"
								board={panel.fixture.board}
								appId={panel.fixture.appId}
							/>
						</BackendContext.Provider>
					) : panel ? (
						<GlobalChatView key={`${panel.path}:${panel.fixture.provider}`} />
					) : (
						<p className="p-4 text-sm text-muted-foreground">
							A real chat panel opens after the native isolation check.
						</p>
					)}
				</div>
			</div>
		</div>
	);
}
