import { I18nProvider } from "@flow-like/locales";
import { QueryClient, QueryClientProvider } from "@tanstack/react-query";
import { useEffect, useRef, useState } from "react";
import { createRoot } from "react-dom/client";
import {
	ActionProvider,
	useExecuteAction,
} from "../../packages/ui/components/a2ui/ActionHandler";
import { A2UIRenderer } from "../../packages/ui/components/a2ui/A2UIRenderer";
import { useSurfaceManager } from "../../packages/ui/components/a2ui/SurfaceManager";
import { WidgetInstanceProvider } from "../../packages/ui/components/a2ui/layout/A2UIWidgetInstance";
import type { A2UIServerMessage } from "../../packages/ui/components/a2ui/types";
import { TooltipProvider } from "../../packages/ui/components/ui/tooltip";
import { resetRunTiming } from "../../packages/ui/lib/run-timing";
import { installExecuteEvent } from "./backend";
import "./styles.css";

type Variant = "baseline" | "overlap" | "bundled" | "optimized";
interface TrialOptions {
	variant: Variant;
	cache: "warm" | "cold";
	personId: number;
	timeoutMs: number;
}
interface TrialResult extends TrialOptions {
	id: number;
	status: "ok" | "error";
	error?: string;
	clickToDispatchMs?: number;
	clickToPatchMs?: number;
	clickToHistoryDomMs?: number;
	clickToHistoryFrameOpportunityMs?: number;
	clickToSettledMs?: number;
	expectedRows?: number;
	serverSample?: unknown;
	actionTimings?: unknown;
	visibility: DocumentVisibilityState;
}
interface ActiveTrial extends TrialResult {
	clickAt: number;
	controller: AbortController;
	domReady: Promise<void>;
	resolveDom: () => void;
	observer?: MutationObserver;
}
interface Controls {
	ready: boolean;
	prepare: (options: TrialOptions) => Promise<void>;
	results: TrialResult[];
}
declare global {
	interface Window {
		__widgetBenchmark: Controls;
		__flowLikeRunTimings?: unknown[];
	}
}

const SURFACE_ID = "widget-action-bench";
const PAGE_ACTION = {
	actionId: "rental-history",
	manifestRevision: "benchmark-v1",
};
const ROOT_COMPONENT = {
	id: "rental-history-panel",
	component: {
		id: "rental-history-panel",
		type: "column" as const,
		children: { explicitList: [] as string[] },
	},
};
const nextFrame = () =>
	new Promise<void>((resolve) => requestAnimationFrame(() => resolve()));
const frameOpportunity = async () => {
	await nextFrame();
	await nextFrame();
};
const defaultOptions: TrialOptions = {
	variant: "optimized",
	cache: "warm",
	personId: 1,
	timeoutMs: 30_000,
};
const controls: Controls = {
	ready: false,
	prepare: async () => {},
	results: [],
};
window.__widgetBenchmark = controls;

function PersonCard({
	personId,
	busy,
	run,
}: {
	personId: number;
	busy: boolean;
	run: (personId: number, invoke: () => Promise<void>) => Promise<void>;
}) {
	const { executeAction } = useExecuteAction();
	return (
		<article className="person">
			<h2>Person {personId}</h2>
			<button
				type="button"
				data-benchmark-person={personId}
				disabled={busy}
				onClick={() =>
					void run(personId, () =>
						executeAction(
							{ name: "widget_event", context: { actionId: "history" } },
							`person-${personId}-history`,
							{ person_id: personId },
							{ requireSuccess: true },
						),
					)
				}
			>
				Mietverlauf ansehen
			</button>
		</article>
	);
}

function Benchmark() {
	const manager = useSurfaceManager();
	const [busy, setBusy] = useState(false);
	const [status, setStatus] = useState(
		"Ready. Select a person or run the browser benchmark.",
	);
	const options = useRef(defaultOptions);
	const active = useRef<ActiveTrial | null>(null);
	const sequence = useRef(0);

	const resetSurface = () =>
		manager.handleServerMessage({
			type: "beginRendering",
			surfaceId: SURFACE_ID,
			rootComponentId: ROOT_COMPONENT.id,
			components: [ROOT_COMPONENT],
			dataModel: [],
		});

	useEffect(() => {
		resetSurface();
		controls.prepare = async (next) => {
			if (active.current)
				throw new Error("A benchmark action is already running");
			if (![1, 2].includes(next.personId))
				throw new Error("Choose synthetic person 1 or 2");
			options.current = { ...defaultOptions, ...next };
			resetSurface();
			resetRunTiming();
			await frameOpportunity();
		};
		installExecuteEvent(
			async (_app, _event, _payload, _stream, onStarted, onEvents) => {
				const trial = active.current;
				if (!trial) throw new Error("No active benchmark action");
				trial.clickToDispatchMs = performance.now() - trial.clickAt;
				onStarted?.(`local-benchmark-${trial.id}`);
				const response = await fetch("/benchmark/action", {
					method: "POST",
					headers: { "Content-Type": "application/json" },
					body: JSON.stringify({
						variant: trial.variant,
						cache: trial.cache,
						person_id: trial.personId,
					}),
					signal: trial.controller.signal,
				});
				if (!response.ok || !response.body)
					throw new Error(`Benchmark server returned HTTP ${response.status}`);
				const reader = response.body.getReader();
				const decoder = new TextDecoder();
				let buffer = "";
				let completed = false;
				let patched = false;
				const consume = (line: string) => {
					if (!line.trim()) return;
					const frame = JSON.parse(line);
					if (frame.type === "error")
						throw new Error(frame.error || "Executor benchmark failed");
					if (frame.type === "patch") {
						if (
							frame.person_id !== trial.personId ||
							!Array.isArray(frame.messages) ||
							!Number.isInteger(frame.expected_rows) ||
							frame.expected_rows < 1
						)
							throw new Error(
								"The executor returned an unexpected person or row count",
							);
						trial.clickToPatchMs ??= performance.now() - trial.clickAt;
						trial.expectedRows = frame.expected_rows;
						patched = true;
						onEvents?.(
							frame.messages.map((payload: A2UIServerMessage) => ({
								event_type: "a2ui",
								payload,
							})) as never,
						);
					}
					if (frame.type === "complete") {
						if (!frame.sample || frame.sample.verified !== true)
							throw new Error(
								"The executor did not verify its benchmark result",
							);
						completed = true;
						trial.serverSample = frame.sample;
						onEvents?.([
							{ event_type: "completed", payload: { status: "completed" } },
						] as never);
					}
				};
				try {
					while (true) {
						const chunk = await reader.read();
						buffer += decoder.decode(chunk.value, { stream: !chunk.done });
						const lines = buffer.split("\n");
						buffer = lines.pop() ?? "";
						for (const line of lines) {
							consume(line);
							if (completed) break;
						}
						if (completed || chunk.done) break;
					}
					if (!completed) consume(buffer);
					if (!completed || !patched)
						throw new Error(
							"The action stream ended without its table patch and completion",
						);
				} finally {
					await reader.cancel().catch(() => {});
					reader.releaseLock();
				}
				return undefined;
			},
		);
		controls.ready = true;
		return () => {
			controls.ready = false;
			active.current?.controller.abort();
		};
	}, []);

	const run = async (personId: number, invoke: () => Promise<void>) => {
		if (active.current) return;
		let resolveDom = () => {};
		const domReady = new Promise<void>((resolve) => {
			resolveDom = resolve;
		});
		const trial: ActiveTrial = {
			...options.current,
			personId,
			id: ++sequence.current,
			status: "ok",
			visibility: document.visibilityState,
			clickAt: performance.now(),
			controller: new AbortController(),
			domReady,
			resolveDom,
		};
		active.current = trial;
		setBusy(true);
		setStatus(
			`Loading Person ${personId} with ${trial.variant} (${trial.cache})…`,
		);
		let rejectTimeout = (_error: Error) => {};
		const timeout = new Promise<never>((_resolve, reject) => {
			rejectTimeout = reject;
		});
		const timer = setTimeout(() => {
			trial.controller.abort();
			rejectTimeout(
				new Error(`Action did not finish within ${trial.timeoutMs} ms`),
			);
		}, trial.timeoutMs);
		const inspectTable = () => {
			if (
				trial.clickToHistoryDomMs !== undefined ||
				trial.expectedRows === undefined
			)
				return;
			const panel = document.getElementById("history-panel");
			const rows = [...(panel?.querySelectorAll("tbody tr") ?? [])].filter(
				(row) => row.querySelector("td"),
			);
			if (
				rows.length !== trial.expectedRows ||
				!rows.some((row) =>
					row.textContent?.includes(`Person ${personId} rental 1`),
				)
			)
				return;
			if (
				rows.some((row) =>
					row.textContent?.includes(`Person ${personId === 1 ? 2 : 1} rental `),
				)
			)
				return;
			trial.clickToHistoryDomMs = performance.now() - trial.clickAt;
			trial.observer?.disconnect();
			void frameOpportunity().then(() => {
				trial.clickToHistoryFrameOpportunityMs =
					performance.now() - trial.clickAt;
				trial.resolveDom();
			});
		};
		trial.observer = new MutationObserver(inspectTable);
		trial.observer.observe(document.getElementById("history-panel")!, {
			subtree: true,
			childList: true,
			characterData: true,
		});
		try {
			if (document.visibilityState !== "visible")
				throw new Error("Keep the benchmark tab visible");
			await Promise.race([
				Promise.all([
					invoke().then(() => {
						trial.clickToSettledMs = performance.now() - trial.clickAt;
					}),
					domReady,
				]),
				timeout,
			]);
			if (document.visibilityState !== "visible")
				throw new Error("Benchmark tab became hidden during the action");
			setStatus(
				`Person ${personId}: table DOM ${trial.clickToHistoryDomMs?.toFixed(1)} ms; frame opportunity ${trial.clickToHistoryFrameOpportunityMs?.toFixed(1)} ms; settled ${trial.clickToSettledMs?.toFixed(1)} ms`,
			);
		} catch (error) {
			trial.status = "error";
			trial.error = error instanceof Error ? error.message : String(error);
			trial.controller.abort();
			setStatus(trial.error);
		} finally {
			clearTimeout(timer);
			trial.observer.disconnect();
			const {
				controller,
				domReady: _ready,
				resolveDom: _resolve,
				observer,
				clickAt,
				...result
			} = trial;
			result.actionTimings = window.__flowLikeRunTimings?.at(-1);
			controls.results.push(result);
			active.current = null;
			setBusy(false);
		}
	};

	const surface = manager.surfaces.get(SURFACE_ID);
	return (
		<main>
			<h1>Widget action benchmark</h1>
			<p className="intro">
				Synthetic rental history through the local executor and production A2UI
				renderer.
			</p>
			<div className="workspace">
				<ActionProvider
					appId="widget-action-bench"
					surfaceId={SURFACE_ID}
					eventId="rental-history-event"
					governedPage
					elementDemand={{ selectors: [] }}
					isPreviewMode
					components={surface?.components ?? {}}
					onA2UIMessage={manager.handleServerMessage}
				>
					<section aria-label="People">
						{[1, 2].map((personId) => (
							<WidgetInstanceProvider
								key={personId}
								instanceId={`person-${personId}`}
								widgetId="person-card"
								componentId={`person-${personId}`}
								actionBindings={{
									history: {
										workflow: { flowId: "rental-history", inputMappings: {} },
										pageAction: PAGE_ACTION,
									},
								}}
							>
								<PersonCard personId={personId} busy={busy} run={run} />
							</WidgetInstanceProvider>
						))}
					</section>
				</ActionProvider>
				<section
					className="history"
					id="history-panel"
					aria-label="Rental history"
				>
					<h2>Mietverlauf</h2>
					{surface && (
						<A2UIRenderer
							surface={surface}
							appId="widget-action-bench"
							onA2UIMessage={manager.handleServerMessage}
							isPreviewMode
						/>
					)}
				</section>
			</div>
			<p id="status" role="status">
				{status}
			</p>
		</main>
	);
}

createRoot(document.getElementById("root")!).render(
	<I18nProvider>
		<QueryClientProvider
			client={
				new QueryClient({ defaultOptions: { queries: { retry: false } } })
			}
		>
			<TooltipProvider>
				<Benchmark />
			</TooltipProvider>
		</QueryClientProvider>
	</I18nProvider>,
);
