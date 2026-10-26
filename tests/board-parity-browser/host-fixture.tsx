"use client";
import {
	BoardRuntimeProvider,
	useReactFlow,
} from "../../packages/ui/components/flow/board-runtime";

import { DndContext } from "@dnd-kit/core";
import { I18nProvider } from "@flow-like/locales";
import { QueryClient, QueryClientProvider } from "@tanstack/react-query";

import { ThemeProvider } from "next-themes";
import { useEffect, useRef, useState } from "react";
import { Toaster } from "sonner";
import { FlowBoard } from "../../packages/ui/components/flow/flow-board";
import { historyRegistry } from "../../packages/ui/components/flow/flow-history";
import { FlowWrapper } from "../../packages/ui/components/flow/flow-wrapper";
import { MobileHeaderProvider } from "../../packages/ui/components/ui/mobile-header";
import { TooltipProvider } from "../../packages/ui/components/ui/tooltip";
import { NgramModel } from "../../packages/ui/lib/node-suggestions/ngram";
import { openSuggestionStore } from "../../packages/ui/lib/node-suggestions/store";
import type { BoardFacts } from "../../packages/ui/lib/node-suggestions/types";
import {
	type IBoard,
	ILayerType,
} from "../../packages/ui/lib/schema/flow/board";
import type { IGenericCommand } from "../../packages/ui/lib/schema/flow/board/commands/generic-command";
import type { INode } from "../../packages/ui/lib/schema/flow/node";
import {
	type IPin,
	IPinType,
	IValueType,
	IVariableType,
} from "../../packages/ui/lib/schema/flow/pin";
import type { IStorageItem } from "../../packages/ui/lib/schema/storage/storage-item";
import { useAssistantSurface } from "../../packages/ui/state/assistant-surface";
import {
	type IBackendState,
	useBackend,
	useBackendStore,
} from "../../packages/ui/state/backend-state";
import type { IBoardMutationOptions } from "../../packages/ui/state/backend-state/board-state";
import {
	FixtureRendererSettings,
	useFixtureRendererReady,
} from "../board-canvas-browser/renderer-settings";
import { applyHostCommand } from "./host-model";
import { makeParityBoard } from "./model";

const APP_ID = "host-parity-app";
const BOARD_ID = "host-parity";
const wait = (ms: number) =>
	new Promise<void>((resolve) => setTimeout(resolve, ms));

/** Shell drags need FlowWrapper's drop handling and something to drag. */
const dropFixture = () =>
	typeof window !== "undefined" &&
	new URLSearchParams(window.location.search).has("drop-ghosts");

function dropPin(
	id: string,
	name: string,
	direction: IPinType,
	index: number,
	dataType: IVariableType,
): IPin {
	return {
		id,
		name,
		friendly_name: name,
		description: "",
		pin_type: direction,
		data_type: dataType,
		value_type: IValueType.Normal,
		index,
		connected_to: [],
		depends_on: [],
	};
}

/** The production catalog entries a shell drop plans from. */
function dropTemplates(): INode[] {
	const template = (
		name: string,
		friendlyName: string,
		icon: string,
		pins: IPin[],
	): INode => ({
		id: `${name}-template`,
		name,
		friendly_name: friendlyName,
		description: friendlyName,
		category: "Fixture",
		icon,
		coordinates: [0, 0, 0],
		pins: Object.fromEntries(
			pins.map((pin) => [
				`${name}.${pin.id}`,
				{ ...pin, id: `${name}.${pin.id}` },
			]),
		),
	});
	const { Input, Output } = IPinType;
	const types = IVariableType;
	return [
		template("variable_get", "Get Variable", "/flow/icons/variable.svg", [
			dropPin("var_ref", "var_ref", Input, 1, types.String),
			dropPin("value", "value", Output, 1, types.Generic),
		]),
		template("variable_set", "Set Variable", "/flow/icons/variable.svg", [
			dropPin("exec_in", "exec_in", Input, 1, types.Execution),
			dropPin("var_ref", "var_ref", Input, 2, types.String),
			dropPin("value_in", "value_in", Input, 3, types.Generic),
			dropPin("exec_out", "exec_out", Output, 1, types.Execution),
			dropPin("value_out", "value_out", Output, 2, types.Generic),
		]),
		template(
			"control_call_function",
			"Call Function",
			"/flow/icons/workflow.svg",
			[
				dropPin(
					"function_layer_id",
					"function_layer_id",
					Input,
					1,
					types.String,
				),
			],
		),
		template("path_from_upload_dir", "Upload Dir", "/flow/icons/path.svg", [
			dropPin("path", "path", Output, 1, types.Struct),
		]),
		template("path_from_user_dir", "User Dir", "/flow/icons/path.svg", [
			dropPin("node_scope", "node_scope", Input, 1, types.Boolean),
			dropPin("path", "path", Output, 1, types.Struct),
		]),
		template("child", "Child", "/flow/icons/path.svg", [
			dropPin("parent_path", "parent_path", Input, 1, types.Struct),
			dropPin("child_name", "child_name", Input, 2, types.String),
			dropPin("path", "path", Output, 1, types.Struct),
		]),
	];
}

const DROP_STORAGE: IStorageItem[] = [
	{ location: "reports", is_dir: true, size: 0, last_modified: "" },
	{ location: "invoice.pdf", size: 2048, last_modified: "" },
];

function makeHostBoard(id = BOARD_ID): IBoard {
	const board = { ...makeParityBoard(), id };
	for (const comment of Object.values(board.comments))
		comment.is_locked ??= false;
	const connectionFixture =
		typeof window === "undefined"
			? null
			: new URLSearchParams(window.location.search).get("connection-targets");
	if (connectionFixture) {
		const reference = connectionFixture === "reference";
		const fixtureIcon = board.nodes["string-b"].icon;
		const template = structuredClone(
			board.nodes[connectionFixture === "execution" ? "exec-b" : "string-b"],
		);
		board.nodes = {};
		board.comments = {};
		board.layers = {};
		for (const [id, x, y, source] of [
			["source", 100, 100, true],
			["source-alt", 100, 300, true],
			["target", 500, 100, false],
			["target-alt", 850, 300, false],
		] as const) {
			const direction = source ? IPinType.Output : IPinType.Input;
			const original = Object.values(template.pins).find(
				(pin) => pin.pin_type === direction,
			);
			if (!original) throw new Error("Connection fixture template pin missing");
			const pin: IPin = {
				...structuredClone(original),
				id: `${id}.${source ? "out" : "first"}`,
				name: source ? "output" : "first",
				friendly_name: source ? "Output" : "First input",
				index: 1,
				connected_to: [],
				depends_on: [],
			};
			const second: IPin = {
				...structuredClone(pin),
				id: `${id}.second`,
				name: "second",
				friendly_name: "Second input",
				index: 2,
			};
			board.nodes[id] = {
				...structuredClone(template),
				id,
				name: `fixture_${id}`,
				friendly_name: `${reference ? "Reference" : "Connection"} ${id}`,
				coordinates: [x, reference && !source ? 500 : y, 0],
				long_running: false,
				// Keep original production handles mounted throughout each gesture.
				icon: fixtureIcon,
				comment: undefined,
				pins: reference
					? {}
					: Object.fromEntries(
							(source ? [pin] : [pin, second]).map((pin) => [pin.id, pin]),
						),
				fn_refs: reference
					? {
							can_reference_fns: source,
							can_be_referenced_by_fns: !source,
							fn_refs: id === "source" ? ["target"] : [],
						}
					: undefined,
			};
		}
		if (!reference && connectionFixture !== "adjacent") {
			board.nodes.source.pins["source.out"].connected_to = ["target.first"];
			board.nodes.target.pins["target.first"].depends_on = ["source.out"];
		}
		return board;
	}
	if (
		typeof window !== "undefined" &&
		new URLSearchParams(window.location.search).has("function-refs")
	) {
		const source = board.nodes["ref-a"];
		Object.assign(source, {
			name: "agent_register_function_tools",
			friendly_name: "Register Function Tools",
			coordinates: [100, 380, 0],
			fn_refs: {
				can_reference_fns: true,
				can_be_referenced_by_fns: false,
				fn_refs: [],
			},
			pins: Object.fromEntries(
				Object.values(board.nodes["integer-a"].pins).map((pin) => {
					const id = pin.id.replace("integer-a", "ref-a");
					return [
						id,
						{
							...pin,
							id,
							name: pin.pin_type === IPinType.Input ? "agent_in" : "agent_out",
							friendly_name: "Agent",
							data_type: IVariableType.Struct,
							connected_to: [],
							depends_on: [],
						},
					];
				}),
			),
		});
		for (const [id, x] of [
			["ref-b", 450],
			["ref-c", 800],
		] as const) {
			const pin = {
				...board.nodes["exec-a"].pins["exec-a.out"],
				id: `${id}.out`,
				name: "exec_out",
				connected_to: [],
				depends_on: [],
			};
			board.nodes[id] = {
				...structuredClone(board.nodes["ref-b"]),
				id,
				name: "events_simple",
				friendly_name: id === "ref-b" ? "Simple Event" : "Other Event",
				coordinates: [x, 380, 0],
				start: true,
				pins: { [pin.id]: pin },
				fn_refs: {
					can_reference_fns: false,
					can_be_referenced_by_fns: true,
					fn_refs: [],
				},
			};
		}
	}
	if (
		typeof window !== "undefined" &&
		new URLSearchParams(window.location.search).has("dynamic-pins")
	) {
		const template = board.nodes["exec-b"];
		const pin = (
			id: string,
			name: string,
			label: string,
			direction: IPinType,
			index: number,
		): IPin => ({
			...template.pins["exec-b.in"],
			id,
			name,
			friendly_name: label,
			pin_type: direction,
			index,
			depends_on: [],
			connected_to: [],
		});
		// Match the production catalog's Parallel Execution pin groups.
		const pins = [
			pin("parallel.in", "exec_in", "Input", IPinType.Input, 1),
			{
				...pin(
					"parallel.threads",
					"thread_model",
					"Threads",
					IPinType.Input,
					2,
				),
				data_type: IVariableType.String,
				default_value: Array.from(new TextEncoder().encode('"tasks"')),
				options: { valid_values: ["tasks", "threads"] },
			},
			pin("parallel.branch-1", "exec_out", "Parallel Task", IPinType.Output, 1),
			pin("parallel.branch-2", "exec_out", "Parallel Task", IPinType.Output, 2),
			pin("parallel.done", "exec_done", "Done", IPinType.Output, 3),
		];
		if (
			new URLSearchParams(window.location.search).get("dynamic-pins") ===
			"expanded"
		) {
			pins.splice(
				4,
				0,
				pin(
					"parallel.branch-3",
					"exec_out",
					"Parallel Task",
					IPinType.Output,
					3,
				),
			);
			const done = pins.find((value) => value.name === "exec_done");
			if (done) done.index = 4;
		}
		board.nodes.parallel = {
			...structuredClone(template),
			id: "parallel",
			name: "control_par_execution",
			friendly_name: "Parallel Execution",
			description: "Parallel Execution",
			category: "Control",
			icon: "/flow/icons/par_execution.svg",
			long_running: false,
			coordinates: [1500, 100, 0],
			pins: Object.fromEntries(pins.map((value) => [value.id, value])),
		};
		const targetPin = pin(
			"parallel-target.in",
			"exec_in",
			"Input",
			IPinType.Input,
			1,
		);
		board.nodes["parallel-target"] = {
			...structuredClone(template),
			id: "parallel-target",
			name: "fixture_parallel_target",
			friendly_name: "Branch target",
			long_running: false,
			coordinates: [1850, 180, 0],
			pins: { [targetPin.id]: targetPin },
		};
	}
	if (dropFixture()) {
		board.variables.counter = {
			id: "counter",
			name: "Counter",
			data_type: IVariableType.Integer,
			value_type: IValueType.Normal,
			editable: true,
			exposed: false,
			secret: false,
		};
		board.layers.fetch = {
			id: "fetch",
			name: "Fetch",
			type: ILayerType.Function,
			coordinates: [0, 0, 0],
			nodes: {},
			comments: {},
			variables: {},
			pins: Object.fromEntries(
				[
					dropPin(
						"fetch.exec_in",
						"exec_in",
						IPinType.Input,
						1,
						IVariableType.Execution,
					),
					dropPin("fetch.url", "url", IPinType.Input, 2, IVariableType.String),
					dropPin(
						"fetch.exec_out",
						"exec_out",
						IPinType.Output,
						1,
						IVariableType.Execution,
					),
					dropPin(
						"fetch.body",
						"body",
						IPinType.Output,
						2,
						IVariableType.String,
					),
				].map((pin) => [pin.id, pin]),
			),
		};
	}
	return board;
}

interface Transaction {
	kind: "execute" | "undo" | "redo";
	commands: IGenericCommand[];
	started: number;
	finished?: number;
}

function Host() {
	const originalBackend = useRef(useBackend()).current;
	const flow = useReactFlow();
	const [ready, setReady] = useState(false);
	const [boardId, setBoardId] = useState(BOARD_ID);
	const model = useRef<IBoard>(makeHostBoard());
	const transactions = useRef<Transaction[]>([]);
	const delay = useRef(0);
	const rejection = useRef(false);
	const reads = useRef<Record<string, number>>({});

	useEffect(() => {
		const params = new URLSearchParams(window.location.search);
		const boardReadDelay = Math.max(
			0,
			Math.min(Number(params.get("board-read-delay")) || 0, 15_000),
		);
		let boardReadFailures = Math.max(
			0,
			Number(params.get("board-read-failures")) || 0,
		);
		let invalidBoardReads = Math.max(
			0,
			Number(params.get("board-read-invalid")) || 0,
		);
		const read = (name: string) => {
			reads.current[name] = (reads.current[name] ?? 0) + 1;
		};
		const apply = (commands: IGenericCommand[], inverse = false) => {
			const next = structuredClone(model.current);
			const receipts = (inverse ? [...commands].reverse() : commands).map(
				(command) => applyHostCommand(next, command, inverse),
			);
			model.current = next;
			return receipts;
		};
		const commit = async (
			kind: Transaction["kind"],
			commands: IGenericCommand[],
			options?: IBoardMutationOptions,
		) => {
			const transaction: Transaction = {
				kind,
				commands: structuredClone(commands),
				started: performance.now(),
			};
			transactions.current.push(transaction);
			await wait(delay.current);
			if (rejection.current) {
				rejection.current = false;
				throw new Error("Fixture rejected the command batch");
			}
			const receipts = apply(commands, kind === "undo");
			transaction.finished = performance.now();
			options?.onBoard?.(structuredClone(model.current));
			return receipts;
		};
		const backend: IBackendState = {
			...originalBackend,
			boardState: Object.assign(Object.create(originalBackend.boardState), {
				async getCatalog() {
					read("getCatalog");
					const seen = new Set<string>();
					const catalog = Object.values(
						structuredClone(model.current.nodes),
					).filter((node) => {
						if (seen.has(node.name)) return false;
						seen.add(node.name);
						return true;
					});
					for (const node of catalog)
						for (const pin of Object.values(node.pins)) {
							pin.connected_to = [];
							pin.depends_on = [];
						}
					const template =
						catalog.find((node) => node.id === "integer-a") ??
						makeParityBoard().nodes["integer-a"];
					const reroute = structuredClone(template);
					reroute.id = "reroute-template";
					reroute.name = "reroute";
					reroute.friendly_name = "Reroute";
					for (const pin of Object.values(reroute.pins)) {
						pin.name = pin.pin_type === "Input" ? "route_in" : "route_out";
						pin.data_type = "Generic" as typeof pin.data_type;
					}
					return [
						...catalog,
						reroute,
						...(dropFixture() ? dropTemplates() : []),
					];
				},
				async getBoard() {
					read("getBoard");
					if (boardReadDelay) await wait(boardReadDelay);
					if (boardReadFailures > 0) {
						boardReadFailures -= 1;
						throw new Error("Fixture could not load the board");
					}
					if (invalidBoardReads > 0) {
						invalidBoardReads -= 1;
						return { message: "Invalid board response" } as unknown as IBoard;
					}
					return structuredClone(model.current);
				},
				async getBoardAuthoritative() {
					return structuredClone(model.current);
				},
				async getBoards() {
					return [structuredClone(model.current)];
				},
				async getBoardVersions() {
					return [[0, 0, 1]];
				},
				async listRuns() {
					return [];
				},
				async executeCommand(
					_app: string,
					_board: string,
					command: IGenericCommand,
					options?: IBoardMutationOptions,
				) {
					return (await commit("execute", [command], options))[0];
				},
				async executeCommands(
					_app: string,
					_board: string,
					commands: IGenericCommand[],
					options?: IBoardMutationOptions,
				) {
					return commit("execute", commands, options);
				},
				async undoBoard(
					_app: string,
					_board: string,
					commands: IGenericCommand[],
					options?: IBoardMutationOptions,
				) {
					await commit("undo", commands, options);
				},
				async redoBoard(
					_app: string,
					_board: string,
					commands: IGenericCommand[],
					options?: IBoardMutationOptions,
				) {
					await commit("redo", commands, options);
				},
			}),
			userState: Object.assign(Object.create(originalBackend.userState), {
				// An anonymous local profile keeps useHub from issuing an API request.
				async getProfile() {
					read("getProfile");
					return null;
				},
				async getSettingsProfile() {
					return { settings: {} };
				},
			}),
			appState: Object.assign(Object.create(originalBackend.appState), {
				async getApp() {
					read("getApp");
					return {
						id: APP_ID,
						name: "Actual FlowBoard host fixture",
						visibility: "Offline",
						execution_mode: "Local",
						boards: [BOARD_ID],
						events: [],
						page_ids: [],
						bits: [],
						authors: [],
						templates: [],
						widget_ids: [],
					};
				},
			}),
			eventState: Object.assign(Object.create(originalBackend.eventState), {
				async getEvents() {
					return [];
				},
			}),
			routeState: Object.assign(Object.create(originalBackend.routeState), {
				async getRoutes() {
					return [];
				},
			}),
			pageState: {
				async getPages() {
					return [];
				},
			} as unknown as IBackendState["pageState"],
			bitState: Object.assign(Object.create(originalBackend.bitState), {
				async getProfileBits() {
					return [];
				},
			}),
			...(dropFixture() && {
				storageState: Object.assign(
					Object.create(originalBackend.storageState),
					{
						async listStorageItems(_app: string, prefix: string) {
							return prefix ? [] : DROP_STORAGE;
						},
					},
				),
				// The explorer lists widgets and tables beside storage.
				widgetState: {
					async getWidgets() {
						return [];
					},
				} as unknown as IBackendState["widgetState"],
				dbState: Object.assign(Object.create(originalBackend.dbState), {
					async listTables() {
						return [];
					},
					async listTablesUser() {
						return [];
					},
				}),
			}),
			async isOffline() {
				return true;
			},
			async isLocalOnly() {
				return true;
			},
		};
		useBackendStore.getState().setBackend(backend);
		let disposed = false;
		void (async () => {
			await historyRegistry.clear(APP_ID, BOARD_ID);
			if (new URLSearchParams(window.location.search).has("ghosts")) {
				const fact: BoardFacts = {
					types: ["harness_integer-a", "harness_integer-b"],
					transitions: [
						{
							kind: "data",
							src: 0,
							srcPin: "output",
							dataType: "Integer",
							valueType: "Normal",
							schema: "",
							pred: [],
							ups: [],
							bag: 1,
							dst: 1,
							dstPin: "input",
						},
					],
					outcomes: [
						{ kind: "data", src: 0, srcPin: "output", connected: true },
					],
				};
				const payload = NgramModel.train(
					Array.from({ length: 100 }, () => structuredClone(fact)),
				).serialize();
				const store = await openSuggestionStore();
				await store.putModel("local", "ngram", {
					payload,
					fingerprint: "host-fixture",
					trainedAt: Date.now(),
					boards: 100,
					transitions: 100,
				});
				await store.setSyncDay("local", new Date().toISOString().slice(0, 10));
			}
			if (!disposed) setReady(true);
		})();
		return () => {
			disposed = true;
			useBackendStore.setState({ backend: null });
		};
	}, [originalBackend]);

	useEffect(() => {
		if (!ready) return;
		Object.assign(window, {
			__boardHost: {
				snapshot: () => ({
					board: structuredClone(model.current),
					surfaceBoardId: useAssistantSurface.getState().boardSurface?.boardId,
					transactions: structuredClone(transactions.current),
					reads: { ...reads.current },
					history: historyRegistry.get(APP_ID, boardId).getSummary(),
					nodes: flow
						.getNodes()
						.map(({ id, position, selected, type, className, data }) => ({
							id,
							position,
							selected,
							type,
							className,
							nativeDom: data.__flowNativeDom,
						})),
					edges: flow
						.getEdges()
						.map(
							({
								id,
								source,
								target,
								sourceHandle,
								targetHandle,
								selected,
							}) => ({
								id,
								source,
								target,
								sourceHandle,
								targetHandle,
								selected,
							}),
						),
					viewport: flow.getViewport(),
				}),
				rejectNext: () => {
					rejection.current = true;
				},
				setDelay: (ms: number) => {
					delay.current = ms;
				},
				highlightNode: (id?: string) => {
					const highlight =
						useAssistantSurface.getState().boardSurface?.highlightNode;
					if (!highlight)
						throw new Error("Board surface highlightNode is unavailable");
					highlight(id);
				},
				highlightNodes: (ids?: string[]) => {
					const highlight =
						useAssistantSurface.getState().boardSurface?.highlightNodes;
					if (!highlight)
						throw new Error("Board surface highlightNodes is unavailable");
					highlight(ids);
				},
				switchBoard: async (id: string) => {
					await historyRegistry.clear(APP_ID, id);
					model.current = makeHostBoard(id);
					transactions.current = [];
					setBoardId(id);
				},
				viewport: (x: number, y: number, zoom: number) =>
					flow.setViewport({ x, y, zoom }),
			},
		});
		return () => {
			Reflect.deleteProperty(window, "__boardHost");
		};
	}, [ready, flow, boardId]);

	return (
		<main className="parity" data-testid="actual-flow-board-host">
			<div className="parity-bar">
				Actual FlowBoard host, local in-memory backend
				<FixtureRendererSettings />
			</div>
			<div
				className="parity-board"
				style={{ display: "flex", flexDirection: "column" }}
			>
				{ready &&
					(dropFixture() ? (
						<FlowWrapper appId={APP_ID} boardId={boardId} externalAssistant />
					) : (
						<FlowBoard
							appId={APP_ID}
							boardId={boardId}
							initialVersion={
								new URLSearchParams(window.location.search).has("historical")
									? [0, 0, 1]
									: undefined
							}
							externalAssistant
						/>
					))}
			</div>
		</main>
	);
}

export default function HostFixture() {
	const rendererReady = useFixtureRendererReady();
	const [query] = useState(
		// The apps' freshness window: with staleTime 0 every board view mounting
		// after a read would refetch the board, which no real host does.
		() =>
			new QueryClient({
				defaultOptions: { queries: { retry: false, staleTime: 30 * 1000 } },
			}),
	);
	return (
		<QueryClientProvider client={query}>
			<I18nProvider language="en">
				<ThemeProvider attribute="class" defaultTheme="dark" forcedTheme="dark">
					<TooltipProvider>
						<MobileHeaderProvider>
							<DndContext>
								<BoardRuntimeProvider>
									{rendererReady && <Host />}
								</BoardRuntimeProvider>
							</DndContext>
						</MobileHeaderProvider>
						<Toaster />
					</TooltipProvider>
				</ThemeProvider>
			</I18nProvider>
		</QueryClientProvider>
	);
}
