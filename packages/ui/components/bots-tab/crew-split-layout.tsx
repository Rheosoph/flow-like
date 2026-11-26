"use client";

import { useTranslation } from "@flow-like/locales";
import { GripVerticalIcon } from "lucide-react";
import {
	type ReactNode,
	createContext,
	useCallback,
	useContext,
	useEffect,
	useId,
	useLayoutEffect,
	useRef,
	useState,
} from "react";
import { cn } from "../../lib/utils";
import { useEdgeSwipeBack } from "./use-edge-swipe-back";

const WIDTH_KEY = "flow-like:bots:chat-width";
const DEFAULT_WIDTH = 420;
const MIN_CHAT_WIDTH = 360;
const MIN_DIRECTORY_WIDTH = 280;
/** The fixed list column of a list-first layout. */
export const LIST_COLUMN_WIDTH = 340;
/** A bot's overview panel when it opens beside the chat. */
export const BOT_PANEL_WIDTH = 400;
const PHONE_BREAKPOINT = 820;
const SLIDE_MS = 300;
const SLIDE_EASE = "cubic-bezier(0.32,0.72,0,1)";
const LIST_PARALLAX_PERCENT = 28;
const DIM_OPACITY = 0.28;

const prefersReducedMotion = () =>
	typeof window !== "undefined" &&
	window.matchMedia?.("(prefers-reduced-motion: reduce)").matches === true;

const noop = () => {};

/** Beside the chat, or over it as a full-height sheet. */
export type DockPanelLayout = "column" | "sheet";

const DockPanelLayoutContext = createContext<DockPanelLayout>("sheet");

/** A column while the crew, the chat and the panel fit side by side; a sheet otherwise and outside the Bots tab. */
export function dockPanelLayout(
	containerWidth: number,
	directoryWidth = MIN_DIRECTORY_WIDTH,
): DockPanelLayout {
	return containerWidth >= directoryWidth + MIN_CHAT_WIDTH + BOT_PANEL_WIDTH
		? "column"
		: "sheet";
}

export function useDockPanelLayout(): DockPanelLayout {
	return useContext(DockPanelLayoutContext);
}

const DockCompactContext = createContext(false);

/** The dock fills the view (a phone, or a window too narrow for the list beside it). */
export function useDockCompact(): boolean {
	return useContext(DockCompactContext);
}

/**
 * Keeps the conversation mounted as its divider moves or the window changes size. An open panel
 * beside the chat widens the dock at the crew's expense; the divider still sizes the chat.
 *
 * Only the columns scroll. `container-type` does not contain positioned descendants in WebKit, so
 * the root and the crew column are positioned and the root clips: otherwise anything absolute in the
 * crew list lands in the app shell's scroller, and a wheel over a short chat shifts the whole tab.
 */
export function CrewSplitLayout({
	children,
	dock,
	panelOpen = false,
	onPanelLayoutChange,
	onCloseDock,
	listFirst = false,
	emptyDock,
	header,
}: Readonly<{
	children: ReactNode;
	dock?: ReactNode;
	/** The list is a fixed column and the dock fills the rest, beside it whenever it fits. */
	listFirst?: boolean;
	/** Fills the dock's place beside the list while nothing is open; a phone shows the list alone. */
	emptyDock?: ReactNode;
	/** Spans the width above the columns; a phone drops it while the dock covers the list. */
	header?: ReactNode;
	/** The dock shows a bot's overview panel. */
	panelOpen?: boolean;
	onPanelLayoutChange?: (layout: DockPanelLayout) => void;
	/** Closes the dock; the edge swipe on a phone calls it like the back button does. */
	onCloseDock?: () => void;
}>) {
	const { t } = useTranslation("bots");
	const container = useRef<HTMLDivElement>(null);
	const chat = useRef<HTMLElement>(null);
	const list = useRef<HTMLDivElement>(null);
	const dim = useRef<HTMLDivElement>(null);
	const lastDock = useRef<ReactNode>(null);
	const hadDock = useRef(Boolean(dock));
	const [exiting, setExiting] = useState(false);
	const [chatElement, setChatElement] = useState<HTMLElement | null>(null);
	const drag = useRef<{ x: number; width: number } | null>(null);
	const chatId = useId();
	const [width, setWidth] = useState(DEFAULT_WIDTH);
	const [containerWidth, setContainerWidth] = useState(0);
	const [dragging, setDragging] = useState(false);
	const [loaded, setLoaded] = useState(false);
	const panelLayout = dockPanelLayout(
		containerWidth,
		listFirst ? LIST_COLUMN_WIDTH : MIN_DIRECTORY_WIDTH,
	);
	const phone = containerWidth > 0 && containerWidth < PHONE_BREAKPOINT;
	if (dock) lastDock.current = dock;
	const [hadDockLastRender, setHadDockLastRender] = useState(Boolean(dock));
	if (hadDockLastRender !== Boolean(dock)) {
		setHadDockLastRender(Boolean(dock));
		if (!dock && phone && !prefersReducedMotion()) setExiting(true);
	}
	const shownDock =
		dock ??
		(exiting ? lastDock.current : undefined) ??
		(listFirst && !phone ? emptyDock : undefined);
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

	const setChat = useCallback((node: HTMLElement | null) => {
		chat.current = node;
		setChatElement(node);
	}, []);

	const hideListTimer = useRef(0);
	const slideTo = useCallback((progress: number, animated: boolean) => {
		const transition = (property: string) =>
			animated ? `${property} ${SLIDE_MS}ms ${SLIDE_EASE}` : "none";
		const dockElement = chat.current;
		const listElement = list.current;
		const dimElement = dim.current;
		// Covered, the list keeps its layout and scroll but stops painting its animated bots.
		window.clearTimeout(hideListTimer.current);
		if (listElement) {
			if (progress < 1) listElement.style.removeProperty("visibility");
			else if (!animated) listElement.style.visibility = "hidden";
			else
				hideListTimer.current = window.setTimeout(() => {
					listElement.style.visibility = "hidden";
				}, SLIDE_MS);
		}
		if (dockElement) {
			dockElement.style.transition = transition("transform");
			dockElement.style.transform = `translate3d(${(1 - progress) * 100}%,0,0)`;
		}
		if (listElement) {
			listElement.style.transition = transition("transform");
			listElement.style.transform = `translate3d(${-progress * LIST_PARALLAX_PERCENT}%,0,0)`;
		}
		if (dimElement) {
			dimElement.style.transition = transition("opacity");
			dimElement.style.opacity = String(progress * DIM_OPACITY);
		}
	}, []);

	const clearSlide = useCallback(() => {
		window.clearTimeout(hideListTimer.current);
		for (const element of [chat.current, list.current, dim.current]) {
			element?.style.removeProperty("transition");
			element?.style.removeProperty("transform");
			element?.style.removeProperty("opacity");
			element?.style.removeProperty("visibility");
		}
	}, []);

	const hasDock = Boolean(dock);
	useLayoutEffect(() => {
		const opening = hasDock && !hadDock.current;
		hadDock.current = hasDock;
		if (!phone) {
			clearSlide();
			setExiting(false);
			return;
		}
		const animated = !prefersReducedMotion();
		if (hasDock) {
			setExiting(false);
			if (!opening || !animated) {
				slideTo(1, false);
				return;
			}
			if (!chat.current?.style.transform) slideTo(0, false);
			let inner = 0;
			const outer = requestAnimationFrame(() => {
				inner = requestAnimationFrame(() => slideTo(1, true));
			});
			return () => {
				cancelAnimationFrame(outer);
				cancelAnimationFrame(inner);
			};
		}
		if (!animated || !chat.current) {
			slideTo(0, false);
			setExiting(false);
			return;
		}
		slideTo(0, true);
		const timer = window.setTimeout(() => setExiting(false), SLIDE_MS + 40);
		return () => window.clearTimeout(timer);
	}, [hasDock, phone, slideTo, clearSlide]);

	useEffect(() => {
		const element = container.current;
		element?.addEventListener("touchstart", noop, { passive: true });
		return () => element?.removeEventListener("touchstart", noop);
	}, []);

	useEffect(() => {
		const element = list.current;
		if (!element) return;
		if (phone && hasDock) element.setAttribute("inert", "");
		else element.removeAttribute("inert");
	}, [phone, hasDock]);

	useEdgeSwipeBack(chatElement, phone && hasDock && Boolean(onCloseDock), {
		onDrag: (fraction) => slideTo(1 - fraction, false),
		onEnd: (commit) => {
			if (commit) {
				slideTo(0, true);
				onCloseDock?.();
			} else slideTo(1, true);
		},
	});

	useEffect(() => {
		onPanelLayoutChange?.(panelLayout);
	}, [panelLayout, onPanelLayoutChange]);

	return (
		<div
			ref={container}
			className={cn(
				"@container/bots flex h-full min-h-0 w-full flex-1 flex-col overflow-clip",
				dragging && "cursor-col-resize select-none",
			)}
		>
			{header && !(phone && shownDock) && header}
			<div className="relative flex min-h-0 flex-1 overflow-clip">
				<div
					ref={list}
					className={cn(
						"@container/crew relative min-w-0 flex-1 overflow-y-auto",
						listFirst &&
							"@min-[820px]/bots:w-85 @min-[820px]/bots:flex-none @min-[820px]/bots:border-r",
					)}
				>
					{children}
				</div>
				{shownDock && (
					<div
						ref={dim}
						aria-hidden
						className="pointer-events-none absolute inset-0 z-20 bg-black opacity-0 @min-[820px]/bots:hidden"
					/>
				)}
				{shownDock && (
					<aside
						ref={setChat}
						id={chatId}
						aria-label={t("crew.dock.label", "Conversation")}
						style={listFirst ? undefined : { width: boundedWidth + panelWidth }}
						className={cn(
							"relative flex min-w-0 flex-col bg-card @max-[820px]/bots:absolute! @max-[820px]/bots:inset-0 @max-[820px]/bots:z-30 @max-[820px]/bots:w-full! @max-[820px]/bots:border-l-0 @max-[820px]/bots:shadow-[-10px_0_24px_-14px_rgb(0_0_0/0.45)]",
							listFirst ? "flex-1" : "shrink-0 border-l",
						)}
					>
						{!listFirst && (
							// biome-ignore lint/a11y/useSemanticElements: This focusable window splitter has interactive children and range semantics.
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
									else if (event.key === "ArrowRight")
										resize(boundedWidth - step);
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
						)}
						<DockPanelLayoutContext.Provider value={panelLayout}>
							<DockCompactContext.Provider value={phone}>
								{shownDock}
							</DockCompactContext.Provider>
						</DockPanelLayoutContext.Provider>
					</aside>
				)}
			</div>
		</div>
	);
}
