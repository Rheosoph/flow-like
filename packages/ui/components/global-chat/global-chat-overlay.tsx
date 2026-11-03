"use client";

import { useTranslation } from "@flow-like/locales";
import { AnimatePresence, motion } from "framer-motion";
import { Maximize2Icon, XIcon } from "lucide-react";
import { usePathname, useRouter } from "next/navigation";
import {
	type CSSProperties,
	useCallback,
	useEffect,
	useMemo,
	useRef,
	useState,
} from "react";
import { createPortal } from "react-dom";
import { cn } from "../../lib/utils";
import { useAssistantSurface } from "../../state/assistant-surface";
import { useBackend, useSignedIn } from "../../state/backend-state";
import { migrateFlowPilotHistory } from "../../state/global-chat/global-chat-migration";
import {
	readPersistedOverlayMode,
	selectToolPromptHere,
	useGlobalChatStore,
} from "../../state/global-chat/global-chat-store";
import { BotChatFrame, type BotPanelControl } from "../bots-tab/bot-chat-frame";
import type { BotPanelActions } from "../bots-tab/bot-details";
import type { BotPanelTab } from "../bots-tab/bot-panel-tabs";
import {
	BOTS_ROUTE,
	dockMoveRoute,
	dockRoute,
	dockSelectionForChatBot,
} from "../bots-tab/dock-selection";
import { BotEditorDialog } from "../bots-tab/editor/bot-editor-dialog";
import { Button } from "../ui/button";
import { useDraftRelay } from "./chat-bot-avatar";
import { ChatBotReactionProvider } from "./chat-bot-reaction";
import { GlobalChatBody } from "./global-chat-body";
import { useAdoptAssistantChat, useGlobalChatBot } from "./use-global-chat-bot";

// Dock geometry. Sizes are in px; MARGIN matches the fixed bottom-6/right-6 inset so the dock never
// touches the viewport edge (which would hard-cut the panel shadow).
const MARGIN = 24;
const MIN_W = 360;
const MIN_H = 420;
const DEFAULT_W = 440;
const DEFAULT_H = 640;
const WORKSPACE_W = 1040;
const WORKSPACE_H = 760;
// Dragging the height past this fraction of the viewport snaps the dock to a full-height sheet.
const SHEET_RATIO = 0.7;
const SIZE_KEY = "flow-like:global-chat:overlay-size";

interface OverlaySize {
	w: number;
	h: number;
	sheet: boolean;
}

function loadOverlaySize(): OverlaySize | null {
	try {
		const raw = localStorage.getItem(SIZE_KEY);
		if (!raw) return null;
		const parsed = JSON.parse(raw);
		if (typeof parsed?.w === "number" && typeof parsed?.h === "number") {
			return { w: parsed.w, h: parsed.h, sheet: Boolean(parsed.sheet) };
		}
	} catch {
		// ignore malformed persisted size
	}
	return null;
}

/**
 * The overlay's panel is a sheet over its chat: another bot's chat and a reopened overlay start
 * with it closed, and it gives way when this chat's run asks for you.
 */
function useOverlayPanel(botId: string | null, open: boolean): BotPanelControl {
	const [shown, setShown] = useState<{
		botId: string | null;
		tab: BotPanelTab | null;
	}>({ botId, tab: null });
	const onTabChange = useCallback(
		(tab: BotPanelTab | null) => setShown({ botId, tab }),
		[botId],
	);
	const asking = useGlobalChatStore(selectToolPromptHere);
	useEffect(() => {
		if (asking || !open) setShown((current) => ({ ...current, tab: null }));
	}, [asking, open]);
	const tab = shown.botId === botId ? shown.tab : null;
	return useMemo(() => ({ tab, onTabChange }), [tab, onTabChange]);
}

/** Edit opens the editor over the page; a copy and a move continue on the Bots tab. */
function useOverlayPanelActions(
	edit: (botId: string) => void,
): BotPanelActions {
	const router = useRouter();
	const closeOverlay = useGlobalChatStore((s) => s.closeOverlay);
	const signedIn = useSignedIn();
	const movable = signedIn && Boolean(useBackend().botState?.local);
	return useMemo(() => {
		const goTo = (href: string) => {
			router.push(href);
			closeOverlay();
		};
		return {
			onEdit: (bot) => edit(bot.id),
			onDuplicated: (copy) => goTo(dockRoute({ type: "bot", botId: copy.id })),
			onMove: movable
				? (bot) => goTo(dockMoveRoute({ type: "bot", botId: bot.id }))
				: undefined,
		};
	}, [closeOverlay, edit, movable, router]);
}

/**
 * Docked bottom-right overlay for the global chat; its pill shows the bot the open conversation
 * belongs to (Flux or one of the caller's bots) and opens that bot's panel as a sheet inside the
 * overlay. When the agent navigates the user to a different view (via the navigate_view tool), the
 * Bots tab conversation morphs into this floating panel so the user keeps chatting while seeing the
 * destination. Rendered into document.body so it escapes sidebar/layout transforms, mirroring the
 * RPA recording dock.
 *
 * The dock is resizable by dragging its top-left corner; dragging the height past ~70% of the viewport
 * snaps it into a full-height sheet. The chosen size persists across sessions (double-click the corner
 * to reset).
 */
export function GlobalChatOverlay() {
	const { t } = useTranslation("chat");
	const draft = useDraftRelay();
	const chatBot = useGlobalChatBot();
	useAdoptAssistantChat();
	const mode = useGlobalChatStore((s) => s.mode);
	const panel = useOverlayPanel(chatBot.botId, mode === "overlay");
	const [editing, setEditing] = useState<string | null>(null);
	const panelActions = useOverlayPanelActions(setEditing);
	const router = useRouter();
	const pathname = usePathname();
	const closeOverlay = useGlobalChatStore((s) => s.closeOverlay);
	const dismissOverlay = useGlobalChatStore((s) => s.dismissOverlay);
	const openOverlay = useGlobalChatStore((s) => s.openOverlay);
	const openOverlayIfAllowed = useGlobalChatStore(
		(s) => s.openOverlayIfAllowed,
	);
	// A FlowScript workspace needs a much wider dock (chat + code side by side); it sets the preset
	// unless the user has picked their own size.
	const hasWorkspace = useGlobalChatStore((s) =>
		Boolean(s.flowscriptWorkspace),
	);

	// If the USER navigates away from the Bots tab while a response is streaming, no surface would
	// be left showing it — the reply would silently stream into the store. Dock the overlay on
	// route changes mid-stream (only on actual navigation, so closing the dock stays possible).
	const prevPathnameRef = useRef(pathname);
	useEffect(() => {
		if (prevPathnameRef.current === pathname) return;
		prevPathnameRef.current = pathname;
		const state = useGlobalChatStore.getState();
		if (
			state.isStreaming &&
			state.mode === "closed" &&
			pathname !== BOTS_ROUTE
		) {
			openOverlayIfAllowed();
		}
	}, [pathname, openOverlayIfAllowed]);

	// Surfaces (board/widget builder) request the assistant via a counter — open the
	// overlay on every bump, skipping whatever value was current on mount. A prompt
	// attached to the request (e.g. "Explain these nodes") becomes a chat draft, which
	// GlobalChatBody auto-sends once in whichever variant is mounted.
	const openAssistantRequested = useAssistantSurface(
		(s) => s.openAssistantRequested,
	);
	const lastOpenRequestRef = useRef(openAssistantRequested);
	useEffect(() => {
		if (openAssistantRequested === lastOpenRequestRef.current) return;
		lastOpenRequestRef.current = openAssistantRequested;
		const prompt = useAssistantSurface.getState().consumeAssistantPrompt();
		if (prompt) useGlobalChatStore.getState().setDraft({ prompt });
		openOverlay();
	}, [openAssistantRequested, openOverlay]);

	// One-time best-effort import of legacy board/widget FlowPilot history into the global chat DB.
	useEffect(() => {
		void migrateFlowPilotHistory();
	}, []);

	// Restore the dock across a hard reload: if it was open before the reload (and we're not on the
	// Bots tab, whose own dock renders the conversation), re-open it. Re-opening re-mounts the
	// chat surface, which then restores the transcript from IndexedDB and re-attaches to any run
	// that was still streaming when the reload hit (global_chat_resume).
	useEffect(() => {
		if (
			readPersistedOverlayMode() === "overlay" &&
			window.location.pathname !== BOTS_ROUTE
		) {
			openOverlay();
		}
	}, [openOverlay]);

	// --- resizable dock -------------------------------------------------------
	const [size, setSize] = useState<OverlaySize | null>(null);
	const [dragging, setDragging] = useState(false);
	const [viewport, setViewport] = useState(() => ({
		w: typeof window === "undefined" ? 1280 : window.innerWidth,
		h: typeof window === "undefined" ? 800 : window.innerHeight,
	}));
	const dragStart = useRef<{
		x: number;
		y: number;
		w: number;
		h: number;
	} | null>(null);

	useEffect(() => {
		setSize(loadOverlaySize());
	}, []);

	useEffect(() => {
		const onResize = () =>
			setViewport({ w: window.innerWidth, h: window.innerHeight });
		window.addEventListener("resize", onResize);
		return () => window.removeEventListener("resize", onResize);
	}, []);

	const maxW = Math.max(MIN_W, viewport.w - MARGIN * 2);
	const fullH = Math.max(MIN_H, viewport.h - MARGIN * 2);
	// A non-sheet dock tops out just below the sheet threshold, so there is a clean gap between a
	// "tall dock" and the full-height sheet — and the default never accidentally reads as a sheet.
	const sheetThreshold = viewport.h * SHEET_RATIO;
	const sheetCapH = Math.max(MIN_H, Math.floor(sheetThreshold));
	const preset: OverlaySize = hasWorkspace
		? { w: WORKSPACE_W, h: WORKSPACE_H, sheet: false }
		: { w: DEFAULT_W, h: DEFAULT_H, sheet: false };
	const active = size ?? preset;
	const width = Math.min(active.w, maxW);
	const height = active.sheet
		? fullH
		: Math.min(Math.max(active.h, MIN_H), sheetCapH);

	// Persist the user's chosen size once a drag settles (not on every drag frame).
	useEffect(() => {
		if (!size || dragging) return;
		try {
			localStorage.setItem(SIZE_KEY, JSON.stringify(size));
		} catch {
			// persistence is best-effort
		}
	}, [size, dragging]);

	const onHandleDown = useCallback(
		(e: React.PointerEvent) => {
			e.preventDefault();
			e.currentTarget.setPointerCapture(e.pointerId);
			dragStart.current = { x: e.clientX, y: e.clientY, w: width, h: height };
			setDragging(true);
		},
		[width, height],
	);

	const onHandleMove = useCallback(
		(e: React.PointerEvent) => {
			const start = dragStart.current;
			if (!start) return;
			// Top-left corner: dragging up/left grows the dock (anchored bottom-right).
			const nextW = Math.min(
				Math.max(start.w + (start.x - e.clientX), MIN_W),
				maxW,
			);
			const rawH = start.h + (start.y - e.clientY);
			const sheet = rawH >= sheetThreshold;
			const nextH = sheet ? fullH : Math.min(Math.max(rawH, MIN_H), sheetCapH);
			setSize({ w: nextW, h: nextH, sheet });
		},
		[maxW, fullH, sheetCapH, sheetThreshold],
	);

	const onHandleUp = useCallback((e: React.PointerEvent) => {
		dragStart.current = null;
		setDragging(false);
		e.currentTarget.releasePointerCapture?.(e.pointerId);
	}, []);

	const resetSize = useCallback(() => {
		setSize(null);
		try {
			localStorage.removeItem(SIZE_KEY);
		} catch {
			// best-effort
		}
	}, []);

	const windowButtonClass =
		"size-8 rounded-full text-muted-foreground outline-none hover:text-foreground focus-visible:ring-2 focus-visible:ring-primary/40 focus-visible:ring-offset-0";
	const windowButtons = (
		<>
			<Button
				variant="ghost"
				size="icon"
				className={windowButtonClass}
				aria-label={t("openFullChat", "Open full chat")}
				title={t("openFullChat", "Open full chat")}
				onClick={() => {
					router.push(
						dockRoute(dockSelectionForChatBot(chatBot.botId), panel.tab),
					);
					closeOverlay();
				}}
			>
				<Maximize2Icon className="size-3.5" />
			</Button>
			<Button
				variant="ghost"
				size="icon"
				className={windowButtonClass}
				aria-label={t("closeChat", "Close chat")}
				title={t("close", "Close")}
				onClick={dismissOverlay}
			>
				<XIcon className="size-4" />
			</Button>
		</>
	);

	const content = (
		<AnimatePresence>
			{mode === "overlay" && (
				<motion.div
					// Balloon out of the bottom-right corner where the FlowPilot bubble launcher sits,
					// so opening the chat reads as that small bubble morphing into the panel.
					initial={{ opacity: 0, scale: 0.32 }}
					animate={{ opacity: 1, scale: 1 }}
					exit={{ opacity: 0, scale: 0.32 }}
					transition={{ type: "spring", stiffness: 340, damping: 30 }}
					className={cn(
						"fixed bottom-6 right-6 z-9999 pointer-events-none origin-bottom-right",
						editing && "invisible",
					)}
				>
					<div
						style={
							{
								width,
								height,
								transition: dragging
									? "none"
									: "width 0.25s ease, height 0.25s ease",
								// A floating dock is not at the screen edge, so the device safe-area
								// insets that pad the shared chatbox must not apply here — that inset is
								// what made the input's bottom margin exceed its side margins. Keep the
								// composer's bottom padding symmetric with its 12px sides; the page uses
								// the larger global default instead.
								"--fl-safe-bottom": "0px",
								"--fl-safe-top": "0px",
								"--fl-chat-pad-bottom": "0.75rem",
							} as CSSProperties
						}
						className={`pointer-events-auto relative flex max-h-[calc(100vh-3rem)] max-w-[calc(100vw-3rem)] flex-col overflow-hidden rounded-xl border border-border/70 bg-background shadow-2xl shadow-black/25 ring-1 ring-black/5 dark:border-white/10 dark:ring-white/10 ${dragging ? "select-none" : ""}`}
					>
						{/* hairline top sheen for a bit of glass depth */}
						<div className="pointer-events-none absolute inset-x-0 top-0 z-40 h-px bg-linear-to-r from-transparent via-foreground/15 to-transparent" />

						{/* resize handle — top-left corner */}
						<button
							type="button"
							onPointerDown={onHandleDown}
							onPointerMove={onHandleMove}
							onPointerUp={onHandleUp}
							onDoubleClick={resetSize}
							aria-label={t("resizeFlowpilotDock", "Resize FlowPilot dock")}
							title={t(
								"dragToResizeDoubleclickToReset",
								"Drag to resize · double-click to reset",
							)}
							className="group absolute left-0 top-0 z-40 flex size-6 cursor-nwse-resize items-start justify-start p-1.5 outline-none"
						>
							<svg
								width="9"
								height="9"
								viewBox="0 0 9 9"
								fill="none"
								aria-hidden="true"
								className="text-muted-foreground/40 transition-colors group-hover:text-primary"
							>
								<path
									d="M1 1v3.5M1 1h3.5"
									stroke="currentColor"
									strokeWidth="1.5"
									strokeLinecap="round"
								/>
							</svg>
						</button>

						<ChatBotReactionProvider>
							<BotChatFrame
								view={chatBot}
								relay={draft.listener}
								panel={panel}
								actions={panelActions}
								end={windowButtons}
							>
								<GlobalChatBody
									variant="overlay"
									onDraftChange={draft.report}
									pillInset
								/>
							</BotChatFrame>
						</ChatBotReactionProvider>
					</div>
				</motion.div>
			)}
		</AnimatePresence>
	);

	const editor = (
		<BotEditorDialog
			open={editing !== null}
			onOpenChange={(open) => {
				if (!open) setEditing(null);
			}}
			botId={editing}
			onDuplicated={panelActions.onDuplicated}
		/>
	);

	if (typeof document === "undefined") return content;
	return (
		<>
			{createPortal(content, document.body)}
			{editor}
		</>
	);
}
