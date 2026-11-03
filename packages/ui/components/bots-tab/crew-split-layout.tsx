"use client";

import { useTranslation } from "@flow-like/locales";
import { GripVerticalIcon } from "lucide-react";
import {
	type ReactNode,
	createContext,
	useContext,
	useEffect,
	useId,
	useLayoutEffect,
	useRef,
	useState,
} from "react";
import { cn } from "../../lib/utils";

const WIDTH_KEY = "flow-like:bots:chat-width";
const DEFAULT_WIDTH = 420;
const MIN_CHAT_WIDTH = 360;
const MIN_DIRECTORY_WIDTH = 280;
/** A bot's overview panel when it opens beside the chat. */
export const BOT_PANEL_WIDTH = 400;

/** Beside the chat, or over it as a full-height sheet. */
export type DockPanelLayout = "column" | "sheet";

const DockPanelLayoutContext = createContext<DockPanelLayout>("sheet");

/** A column while the crew, the chat and the panel fit side by side; a sheet otherwise and outside the Bots tab. */
export function dockPanelLayout(containerWidth: number): DockPanelLayout {
	return containerWidth >=
		MIN_DIRECTORY_WIDTH + MIN_CHAT_WIDTH + BOT_PANEL_WIDTH
		? "column"
		: "sheet";
}

export function useDockPanelLayout(): DockPanelLayout {
	return useContext(DockPanelLayoutContext);
}

/**
 * Keeps the conversation mounted as its divider moves or the window changes size. An open panel
 * beside the chat widens the dock at the crew's expense; the divider still sizes the chat.
 */
export function CrewSplitLayout({
	children,
	dock,
	panelOpen = false,
	onPanelLayoutChange,
}: Readonly<{
	children: ReactNode;
	dock?: ReactNode;
	/** The dock shows a bot's overview panel. */
	panelOpen?: boolean;
	onPanelLayoutChange?: (layout: DockPanelLayout) => void;
}>) {
	const { t } = useTranslation("bots");
	const container = useRef<HTMLDivElement>(null);
	const chat = useRef<HTMLElement>(null);
	const drag = useRef<{ x: number; width: number } | null>(null);
	const chatId = useId();
	const [width, setWidth] = useState(DEFAULT_WIDTH);
	const [containerWidth, setContainerWidth] = useState(0);
	const [dragging, setDragging] = useState(false);
	const [loaded, setLoaded] = useState(false);
	const panelLayout = dockPanelLayout(containerWidth);
	const panelWidth =
		dock && panelOpen && panelLayout === "column" ? BOT_PANEL_WIDTH : 0;
	const maxWidth = Math.max(
		MIN_CHAT_WIDTH,
		containerWidth - MIN_DIRECTORY_WIDTH - panelWidth,
	);
	const boundedWidth = Math.min(width, maxWidth);
	const resize = (next: number) =>
		setWidth(Math.min(maxWidth, Math.max(MIN_CHAT_WIDTH, next)));

	useLayoutEffect(() => {
		const element = container.current;
		if (!element) return;
		const measure = () =>
			setContainerWidth(element.getBoundingClientRect().width);
		measure();
		const observer = new ResizeObserver(measure);
		observer.observe(element);
		return () => observer.disconnect();
	}, []);

	useEffect(() => {
		try {
			const stored = Number(localStorage.getItem(WIDTH_KEY));
			if (Number.isFinite(stored) && stored >= MIN_CHAT_WIDTH) setWidth(stored);
		} catch {
			// Resizing still works when browser storage is unavailable.
		}
		setLoaded(true);
	}, []);

	useEffect(() => {
		if (!loaded || dragging) return;
		try {
			localStorage.setItem(WIDTH_KEY, String(width));
		} catch {
			// The current layout does not depend on saving the preference.
		}
	}, [width, loaded, dragging]);

	useEffect(() => {
		onPanelLayoutChange?.(panelLayout);
	}, [panelLayout, onPanelLayoutChange]);

	return (
		<div
			ref={container}
			className={cn(
				"@container/bots flex h-full min-h-0 w-full flex-1 overflow-hidden",
				dragging && "cursor-col-resize select-none",
			)}
		>
			<div
				className={cn(
					"@container/crew min-w-0 flex-1 overflow-y-auto",
					dock && "@max-[820px]/bots:hidden",
				)}
			>
				{children}
			</div>
			{dock && (
				<aside
					ref={chat}
					id={chatId}
					aria-label={t("crew.dock.label", "Conversation")}
					style={{ width: boundedWidth + panelWidth }}
					className="relative flex min-w-0 shrink-0 flex-col border-l bg-card @max-[820px]/bots:w-full! @max-[820px]/bots:border-l-0"
				>
					{/* biome-ignore lint/a11y/useSemanticElements: This focusable window splitter has interactive children and range semantics. */}
					<div
						role="separator"
						tabIndex={0}
						aria-label={t("crew.dock.resize", "Resize chat")}
						aria-orientation="vertical"
						aria-controls={chatId}
						aria-valuemin={MIN_CHAT_WIDTH}
						aria-valuemax={Math.round(maxWidth)}
						aria-valuenow={Math.round(boundedWidth)}
						aria-valuetext={t(
							"crew.dock.width",
							"Chat width: {{width}} pixels",
							{
								width: Math.round(boundedWidth),
							},
						)}
						title={t(
							"crew.dock.resizeHint",
							"Drag to resize. Double-click to reset.",
						)}
						className="group absolute inset-y-0 -left-1.5 z-40 flex w-3 touch-none cursor-col-resize items-center justify-center outline-none @max-[820px]/bots:hidden"
						onPointerDown={(event) => {
							if (event.button !== 0) return;
							event.preventDefault();
							event.currentTarget.focus();
							event.currentTarget.setPointerCapture(event.pointerId);
							drag.current = {
								x: event.clientX,
								width:
									(chat.current?.getBoundingClientRect().width ??
										boundedWidth + panelWidth) - panelWidth,
							};
							setDragging(true);
						}}
						onPointerMove={(event) => {
							if (drag.current)
								resize(drag.current.width + drag.current.x - event.clientX);
						}}
						onPointerUp={(event) => {
							if (event.currentTarget.hasPointerCapture(event.pointerId))
								event.currentTarget.releasePointerCapture(event.pointerId);
						}}
						onLostPointerCapture={() => {
							drag.current = null;
							setDragging(false);
						}}
						onDoubleClick={() => resize(DEFAULT_WIDTH)}
						onKeyDown={(event) => {
							const step = event.shiftKey ? 80 : 20;
							if (event.key === "ArrowLeft") resize(boundedWidth + step);
							else if (event.key === "ArrowRight") resize(boundedWidth - step);
							else if (event.key === "Home") resize(MIN_CHAT_WIDTH);
							else if (event.key === "End") resize(maxWidth);
							else return;
							event.preventDefault();
						}}
					>
						<span className="absolute inset-y-0 w-0.5 transition-colors group-hover:bg-primary/50 group-focus-visible:bg-primary" />
						<span className="relative flex h-9 w-3 items-center justify-center rounded-full border bg-card text-muted-foreground group-hover:border-primary/50 group-hover:text-primary group-focus-visible:ring-2 group-focus-visible:ring-ring">
							<GripVerticalIcon className="size-3" aria-hidden />
						</span>
					</div>
					<DockPanelLayoutContext.Provider value={panelLayout}>
						{dock}
					</DockPanelLayoutContext.Provider>
				</aside>
			)}
		</div>
	);
}
