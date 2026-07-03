"use client";

import {
	Button,
	Checkbox,
	Dialog,
	DialogContent,
	DialogDescription,
	DialogFooter,
	DialogHeader,
	DialogTitle,
	type IMetadata,
	IRole,
	Response,
	Textarea,
	nowSystemTime,
	useBackend,
} from "@flow-like/flow-like-ui";
import {
	flowPilotModelIdForProvider,
	normalizeAIProvider,
} from "@flow-like/flow-like-ui/components/flowpilot/types";
import type {
	IAttachment,
	IMessage,
} from "@flow-like/flow-like-ui/components/interfaces/chat-default/chat-db";
import { processChatEvents } from "@flow-like/flow-like-ui/components/interfaces/chat-default/event-processor";
import { parseUint8ArrayToJson } from "@flow-like/flow-like-ui/lib/uint8";
import { createId } from "@paralleldrive/cuid2";
import { usePathname, useRouter } from "next/navigation";
import { useCallback, useEffect, useRef, useState } from "react";
import { useGlobalChatStore } from "../../lib/global-chat-store";

const GLOBAL_FRONTEND_TOOL_EVENT = "flowpilot://global-tool-request";

type ApprovalKind = "none" | "mutating" | "execute";

interface FrontendToolApproval {
	kind: ApprovalKind;
	title?: string;
	description?: string;
	sessionKey?: string;
}

interface FrontendToolRequest {
	requestId: string;
	toolName: string;
	arguments: Record<string, unknown>;
	approval?: FrontendToolApproval;
}

interface FrontendToolResponse {
	requestId: string;
	approved: boolean;
	result?: unknown;
	error?: string;
}

type DialogState =
	| { type: "approval"; request: FrontendToolRequest; remember: boolean }
	| { type: "ask"; request: FrontendToolRequest; value: string };

function argString(args: Record<string, unknown>, key: string): string {
	const value = args[key];
	return typeof value === "string" ? value : "";
}

/** The board copilot's flowscript_workspace may be the raw source or a {source,status} JSON blob. */
function extractFlowScriptSource(
	workspace: string | undefined,
): string | undefined {
	const trimmed = workspace?.trim();
	if (!trimmed) return undefined;
	try {
		const parsed = JSON.parse(trimmed);
		if (
			parsed &&
			typeof parsed === "object" &&
			typeof parsed.source === "string"
		) {
			return parsed.source;
		}
	} catch {
		// not JSON — treat the whole string as the flowscript source
	}
	return trimmed;
}

function routeForView(args: Record<string, unknown>): string {
	const explicit = argString(args, "route");
	if (explicit) return explicit;
	const view = argString(args, "view").toLowerCase();
	const appId = argString(args, "app_id") || argString(args, "appId");
	switch (view) {
		case "home":
			return "/";
		case "apps":
		case "library":
			return "/library";
		case "store":
		case "packages":
			return "/store/explore/apps";
		case "settings":
			return "/settings";
		case "profile":
		case "profiles":
			return "/settings/profiles";
		case "learn":
		case "university":
		case "courses":
			return "/learn";
		case "board":
		case "flow":
			return appId ? `/library?id=${appId}` : "/library";
		default:
			return "/";
	}
}

/**
 * Listens for the global FlowPilot assistant's tool requests (a dedicated Tauri event, separate from
 * the board copilot's) and executes them in the app: navigation, app creation, and delegating board
 * work. Mutating/execute tools open an approval dialog; ask_user opens an input dialog. The response
 * is returned through the shared `flowpilot_frontend_tool_result` command.
 */
export function GlobalToolBridge() {
	const router = useRouter();
	const pathname = usePathname();
	const backend = useBackend();
	const openOverlay = useGlobalChatStore((s) => s.openOverlay);
	const addInlineAppChat = useGlobalChatStore((s) => s.addInlineAppChat);

	// The full /chat page already renders the conversation — only dock the overlay elsewhere.
	const pathnameRef = useRef(pathname);
	useEffect(() => {
		pathnameRef.current = pathname;
	}, [pathname]);
	const showConversation = useCallback(() => {
		if (pathnameRef.current !== "/chat") openOverlay();
	}, [openOverlay]);
	const [dialog, setDialog] = useState<DialogState | null>(null);
	type DialogResolution =
		| { approved: boolean; remember: boolean }
		| string
		| null;
	const resolverRef = useRef<((value: DialogResolution) => void) | null>(null);
	// The agent loop executes tool calls in parallel (join_all in Rust), so multiple dialog
	// requests can arrive concurrently — queue them and show one at a time, or the orphaned
	// request would block the agent until its bridge timeout.
	const dialogQueueRef = useRef<
		Array<{ dialog: DialogState; resolve: (value: DialogResolution) => void }>
	>([]);
	const approvedKeysRef = useRef<Set<string>>(new Set());
	const executeRef = useRef<
		(request: FrontendToolRequest) => Promise<FrontendToolResponse>
	>(async (request) => ({ requestId: request.requestId, approved: false }));

	const openDialog = useCallback(
		(next: DialogState) =>
			new Promise<DialogResolution>((resolve) => {
				if (resolverRef.current) {
					dialogQueueRef.current.push({ dialog: next, resolve });
					return;
				}
				resolverRef.current = resolve;
				setDialog(next);
			}),
		[],
	);

	const resolveDialog = useCallback((value: DialogResolution) => {
		const resolver = resolverRef.current;
		resolverRef.current = null;
		resolver?.(value);
		const next = dialogQueueRef.current.shift();
		if (next) {
			resolverRef.current = next.resolve;
			setDialog(next.dialog);
		} else {
			setDialog(null);
		}
	}, []);

	const runTool = useCallback(
		async (request: FrontendToolRequest): Promise<unknown> => {
			const args = request.arguments ?? {};
			// Only apps visible in the CURRENT profile are eligible for app-interface tools.
			const getProfileAppIds = async (): Promise<Set<string>> => {
				try {
					const profile = await backend.userState.getSettingsProfile();
					return new Set(
						(profile?.hub_profile?.apps ?? []).map((entry) => entry.app_id),
					);
				} catch {
					return new Set<string>();
				}
			};
			switch (request.toolName) {
				case "list_apps": {
					// Selection is driven by app + EVENT metadata only (no board loading): each app's
					// active events and their event_type tell the agent which interfaces it can call.
					const profileAppIds = await getProfileAppIds();
					const apps = await backend.appState.getApps();
					const visible = apps.filter(([app]) => profileAppIds.has(app.id));
					const detailed = await Promise.all(
						visible.slice(0, 25).map(async ([app, meta]) => {
							let events: Array<{
								id: string;
								name: string;
								description: string;
								event_type: string;
							}> = [];
							try {
								const appEvents = await backend.eventState.getEvents(app.id);
								events = appEvents
									.filter((event) => event.active)
									.map((event) => ({
										id: event.id,
										name: event.name,
										description: event.description,
										event_type: event.event_type,
									}));
							} catch {
								// ignore apps whose events cannot be listed
							}
							return {
								app_id: app.id,
								name: meta?.name ?? app.id,
								description: meta?.description ?? "",
								events,
							};
						}),
					);
					return { status: "ok", apps: detailed };
				}
				case "navigate_view": {
					const route = routeForView(args);
					router.push(route);
					// Morph the conversation into the docked overlay so the user keeps chatting
					// while the destination view is shown — decided by the TARGET route (the
					// pre-navigation pathname may still be /chat when this runs).
					if (!route.startsWith("/chat")) openOverlay();
					return { status: "ok", route };
				}
				case "describe_app_interface": {
					const appId = argString(args, "app_id") || argString(args, "appId");
					const eventId =
						argString(args, "event_id") || argString(args, "eventId");
					if (!appId || !eventId)
						return {
							status: "error",
							message: "describe_app_interface requires app_id and event_id.",
						};
					const profileAppIds = await getProfileAppIds();
					if (!profileAppIds.has(appId))
						return {
							status: "error",
							message: `App '${appId}' is not visible in the current profile.`,
						};
					const events = await backend.eventState.getEvents(appId);
					const event = events.find((candidate) => candidate.id === eventId);
					if (!event)
						return {
							status: "error",
							message: `Event '${eventId}' not found in app '${appId}'.`,
						};
					// The event configuration is the user-readable interface contract (chat
					// settings, REST routes, MCP tools, …) — expose it verbatim, size-capped.
					let config = parseUint8ArrayToJson(event.config) ?? {};
					const serialized = JSON.stringify(config);
					if (serialized.length > 12_000) {
						config = { truncated: true, preview: serialized.slice(0, 12_000) };
					}
					return {
						status: "ok",
						event: {
							id: event.id,
							name: event.name,
							description: event.description,
							event_type: event.event_type,
							active: event.active,
							inputs: event.inputs ?? [],
						},
						config,
					};
				}
				case "open_app_chat": {
					const appId = argString(args, "app_id") || argString(args, "appId");
					if (!appId)
						return {
							status: "error",
							message: "open_app_chat requires an app_id.",
						};
					const profileAppIds = await getProfileAppIds();
					if (!profileAppIds.has(appId))
						return {
							status: "error",
							message: `App '${appId}' is not visible in the current profile.`,
						};
					const eventId =
						argString(args, "event_id") || argString(args, "eventId");
					const events = await backend.eventState.getEvents(appId);
					const chatEvent = eventId
						? events.find(
								(event) => event.id === eventId && event.event_type === "chat",
							)
						: events.find((event) => event.event_type === "chat");
					if (!chatEvent)
						return {
							status: "error",
							message: `App '${appId}' has no chat event.`,
						};
					addInlineAppChat({
						appId,
						eventId: chatEvent.id,
						name: chatEvent.name || appId,
					});
					showConversation();
					return {
						status: "ok",
						message: `Opened '${chatEvent.name}' inline — the user can now chat with the app directly.`,
					};
				}
				case "create_app": {
					const name = argString(args, "name") || "Untitled app";
					const description = argString(args, "description");
					const meta: IMetadata = {
						name,
						description,
						tags: [],
						use_case: "",
						created_at: nowSystemTime(),
						updated_at: nowSystemTime(),
						preview_media: [],
					};
					const app = await backend.appState.createApp(meta, [], false);
					return { status: "ok", app_id: app.id, name };
				}
				case "flowpilot_board": {
					const instruction = argString(args, "instruction");
					if (!instruction)
						return {
							status: "error",
							message: "flowpilot_board requires an instruction.",
						};
					const appId = argString(args, "app_id") || argString(args, "appId");
					if (!appId)
						return {
							status: "error",
							message: "flowpilot_board requires an app_id.",
						};
					let boardId =
						argString(args, "board_id") || argString(args, "boardId");
					if (!boardId) {
						const boards = await backend.boardState.getBoards(appId);
						boardId = boards?.[0]?.id ?? "";
					}
					if (!boardId)
						return {
							status: "error",
							message: `No board found in app '${appId}'.`,
						};

					const [board, catalog] = await Promise.all([
						backend.boardState.getBoard(appId, boardId, undefined, true),
						backend.boardState.getCatalog(appId),
					]);

					// Run the board copilot as a sub-agent, using the global chat's selected model.
					const chat = useGlobalChatStore.getState();
					const modelId = flowPilotModelIdForProvider(
						normalizeAIProvider(chat.provider),
						chat.selectedModelId,
					);
					const response = await backend.boardState.copilot_chat(
						"Board",
						board,
						catalog,
						[],
						null,
						[],
						instruction,
						[],
						undefined,
						undefined,
						modelId,
					);

					// Apply the resulting FlowScript ADDITIVELY: allowDeletions=false, so the backend
					// merge blocks destructive edits instead of silently deleting existing board work.
					let appliedCommands = 0;
					let blockedDeletion = false;
					const source = extractFlowScriptSource(response.flowscript_workspace);
					if (source) {
						const applyResult = await backend.boardState.applyFlowScript(
							appId,
							boardId,
							source,
							undefined,
							catalog,
							false,
						);
						appliedCommands = applyResult.commands?.length ?? 0;
						blockedDeletion =
							applyResult.diagnostics?.[0]?.startsWith(
								"FlowScript edit would delete ",
							) ?? false;
					}

					// Show the board and keep chatting in the docked overlay (target is /flow, so
					// the conversation always needs the dock).
					router.push(`/flow?id=${boardId}&app=${appId}`);
					openOverlay();

					return {
						status: "ok",
						message: response.message,
						applied_commands: appliedCommands,
						...(blockedDeletion
							? {
									blocked_deletion: true,
									note: "Some edits would delete existing board items and were blocked. Confirm with the user before deleting.",
								}
							: {}),
					};
				}
				case "call_app_chat": {
					const appId = argString(args, "app_id") || argString(args, "appId");
					const message =
						argString(args, "message") || argString(args, "prompt");
					if (!appId)
						return {
							status: "error",
							message: "call_app_chat requires an app_id.",
						};
					if (!message)
						return {
							status: "error",
							message: "call_app_chat requires a message.",
						};

					const profileAppIds = await getProfileAppIds();
					if (!profileAppIds.has(appId))
						return {
							status: "error",
							message: `App '${appId}' is not visible in the current profile.`,
						};

					// Call the specific chat event the agent selected from list_apps metadata
					// (falling back to the app's first chat event) — events, not boards.
					const eventId =
						argString(args, "event_id") || argString(args, "eventId");
					const events = await backend.eventState.getEvents(appId);
					const chatEvent = eventId
						? events.find(
								(event) => event.id === eventId && event.event_type === "chat",
							)
						: events.find((event) => event.event_type === "chat");
					if (!chatEvent)
						return {
							status: "error",
							message: eventId
								? `App '${appId}' has no chat event '${eventId}'.`
								: `App '${appId}' has no chat event.`,
						};

					// Invoke the app's chat event through the SAME pipeline the simple chat uses
					// (executeEvent + processChatEvents), so it runs with full app-chat behavior.
					const chatId = createId();
					const runPayload = {
						id: chatEvent.node_id,
						payload: {
							chat_id: chatId,
							messages: [{ role: "user", content: message }],
							local_session: {},
							global_session: {},
							actions: [],
							tools: [],
							attachments: [],
						},
					};

					const responseMessage: IMessage = {
						id: createId(),
						appId,
						sessionId: chatId,
						inner: { role: IRole.Assistant, content: "" },
						files: [],
						tools: [],
						actions: [],
						timestamp: Date.now(),
					};
					let intermediate = Response.default();
					const attachments = new Map<string, IAttachment>();
					await backend.eventState.executeEvent(
						appId,
						chatEvent.id,
						runPayload as Parameters<typeof backend.eventState.executeEvent>[2],
						false,
						undefined,
						(batch) => {
							const result = processChatEvents(batch, {
								intermediateResponse: intermediate,
								responseMessage,
								attachments,
								tmpLocalState: null,
								tmpGlobalState: null,
								done: false,
								appId,
								eventId: chatEvent.id,
								sessionId: chatId,
							});
							intermediate = result.intermediateResponse;
						},
					);

					const text =
						typeof responseMessage.inner.content === "string"
							? responseMessage.inner.content
							: "";
					return {
						status: "ok",
						app_id: appId,
						response: text || "(the app chat returned no text)",
					};
				}
				default:
					throw new Error(`Unsupported global tool '${request.toolName}'.`);
			}
		},
		[
			backend.appState,
			backend.boardState,
			backend.eventState,
			backend.userState,
			router,
			openOverlay,
			showConversation,
			addInlineAppChat,
		],
	);

	const execute = useCallback(
		async (request: FrontendToolRequest): Promise<FrontendToolResponse> => {
			try {
				if (request.toolName === "ask_user") {
					const answer = await openDialog({ type: "ask", request, value: "" });
					if (answer === null)
						return {
							requestId: request.requestId,
							approved: false,
							error: "User dismissed the question.",
						};
					return {
						requestId: request.requestId,
						approved: true,
						result: { status: "ok", answer },
					};
				}

				const approval = request.approval;
				const sessionKey =
					approval?.sessionKey ||
					`${request.toolName}:${approval?.kind ?? "none"}`;
				const needsApproval =
					approval?.kind === "mutating" || approval?.kind === "execute";

				if (needsApproval && !approvedKeysRef.current.has(sessionKey)) {
					const outcome = await openDialog({
						type: "approval",
						request,
						remember: false,
					});
					if (!outcome || typeof outcome === "string" || !outcome.approved) {
						return {
							requestId: request.requestId,
							approved: false,
							error: "User denied the request.",
						};
					}
					if (outcome.remember) approvedKeysRef.current.add(sessionKey);
				}

				const result = await runTool(request);
				return { requestId: request.requestId, approved: true, result };
			} catch (error) {
				// approved:true + error => the bridge reports status:"error" (not a user denial).
				return {
					requestId: request.requestId,
					approved: true,
					error: error instanceof Error ? error.message : String(error),
				};
			}
		},
		[openDialog, runTool],
	);

	useEffect(() => {
		executeRef.current = execute;
	}, [execute]);

	useEffect(() => {
		if (typeof window === "undefined") return;
		let disposed = false;
		let unlisten: (() => void) | undefined;

		void (async () => {
			try {
				const [{ listen }, { invoke }] = await Promise.all([
					import("@tauri-apps/api/event"),
					import("@tauri-apps/api/core"),
				]);
				const stop = await listen<FrontendToolRequest>(
					GLOBAL_FRONTEND_TOOL_EVENT,
					async (event) => {
						const request = event.payload;
						if (!request?.requestId || !request.toolName) return;
						const response = await executeRef.current(request);
						await invoke("flowpilot_frontend_tool_result", { response });
					},
				);
				if (disposed) stop();
				else unlisten = stop;
			} catch {
				// Not running under Tauri (e.g. web build) — the global tool bridge is desktop-only.
			}
		})();

		return () => {
			disposed = true;
			unlisten?.();
		};
	}, []);

	const approvalRequest = dialog?.request;
	const approvalMeta = approvalRequest?.approval;

	return (
		<Dialog
			open={dialog !== null}
			onOpenChange={(open) => !open && resolveDialog(null)}
		>
			<DialogContent className="max-w-md">
				{dialog?.type === "ask" && (
					<>
						<DialogHeader>
							<DialogTitle>FlowPilot needs input</DialogTitle>
							<DialogDescription>
								{argString(dialog.request.arguments, "question") ||
									argString(dialog.request.arguments, "prompt") ||
									"Please provide the requested information."}
							</DialogDescription>
						</DialogHeader>
						<Textarea
							autoFocus
							value={dialog.value}
							onChange={(e) => setDialog({ ...dialog, value: e.target.value })}
							placeholder="Your answer…"
							className="min-h-24"
						/>
						<DialogFooter>
							<Button variant="ghost" onClick={() => resolveDialog(null)}>
								Cancel
							</Button>
							<Button
								onClick={() => resolveDialog(dialog.value)}
								disabled={!dialog.value.trim()}
							>
								Send
							</Button>
						</DialogFooter>
					</>
				)}
				{dialog?.type === "approval" && (
					<>
						<DialogHeader>
							<DialogTitle>
								{approvalMeta?.title || "Approve action"}
							</DialogTitle>
							<DialogDescription>
								{approvalMeta?.description ||
									`FlowPilot wants to run '${approvalRequest?.toolName}'.`}
							</DialogDescription>
						</DialogHeader>
						<div className="flex items-center gap-2 text-sm text-muted-foreground">
							<Checkbox
								id="global-tool-remember"
								checked={dialog.remember}
								onCheckedChange={(checked) =>
									setDialog({ ...dialog, remember: checked === true })
								}
							/>
							<label htmlFor="global-tool-remember">
								Don&apos;t ask again this session
							</label>
						</div>
						<DialogFooter>
							<Button
								variant="ghost"
								onClick={() =>
									resolveDialog({ approved: false, remember: false })
								}
							>
								Deny
							</Button>
							<Button
								onClick={() =>
									resolveDialog({ approved: true, remember: dialog.remember })
								}
							>
								Approve
							</Button>
						</DialogFooter>
					</>
				)}
			</DialogContent>
		</Dialog>
	);
}
