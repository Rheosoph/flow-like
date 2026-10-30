"use client";

import { useTranslation } from "@flow-like/locales";
import Maximize2 from "lucide-react/dist/esm/icons/maximize-2.js";
import Minimize2 from "lucide-react/dist/esm/icons/minimize-2.js";
import PanelRightClose from "lucide-react/dist/esm/icons/panel-right-close.js";
import PanelRightOpen from "lucide-react/dist/esm/icons/panel-right-open.js";
import X from "lucide-react/dist/esm/icons/x.js";
import {
	type ReactNode,
	useCallback,
	useEffect,
	useId,
	useLayoutEffect,
	useRef,
	useState,
} from "react";
import type { ImperativePanelGroupHandle } from "react-resizable-panels";
import { isRuntimeNamespace } from "../../../lib/service-runtime/session-scope";
import { Button } from "../../ui/button";
import {
	ResizableHandle,
	ResizablePanel,
	ResizablePanelGroup,
} from "../../ui/resizable";
import type { IChatMainWidget } from "./chat-db";
import { MessageWidget } from "./message-widgets";

type Display = "open" | "collapsed" | "maximized";

export interface MainWidgetPaneProps {
	mainWidget?: IChatMainWidget;
	sessionId: string;
	appId?: string;
	boardId?: string;
	eventId?: string;
	snapshots?: boolean;
	children: ReactNode;
}

/** Keeps the chat and widget mounted while the divider or display controls move them. */
export function MainWidgetPane({
	mainWidget,
	sessionId,
	appId,
	boardId,
	eventId,
	snapshots = true,
	children,
}: MainWidgetPaneProps) {
	const { t } = useTranslation("chat");
	const id = useId();
	const groupRef = useRef<ImperativePanelGroupHandle>(null);
	const reopenRef = useRef<HTMLButtonElement>(null);
	const dialogRef = useRef<HTMLDialogElement>(null);
	const modalRef = useRef(false);
	const lastOpenSize = useRef(50);
	const previousSession = useRef(sessionId);
	const widget = mainWidget?.widget;
	const hasWidget = Boolean(widget);
	const [display, setDisplay] = useState<Display>(
		hasWidget ? (mainWidget?.display ?? "open") : "collapsed",
	);
	const [isMobile, setIsMobile] = useState(false);
	const [mobileOpen, setMobileOpen] = useState(false);
	const title = mainWidget?.title?.trim() || t("mainWidget", "Main widget");
	const runtime = isRuntimeNamespace(appId);

	useLayoutEffect(() => {
		const query = window.matchMedia("(max-width: 767px)");
		const update = () => setIsMobile(query.matches);
		update();
		query.addEventListener("change", update);
		return () => query.removeEventListener("change", update);
	}, []);

	const changeDisplay = useCallback((next: Display) => setDisplay(next), []);

	// Commands change the layout once. Data updates and final response replays
	// keep the user's chosen size and collapsed state.
	// biome-ignore lint/correctness/useExhaustiveDependencies: command and widget identity trigger new display directives, while replay updates preserve the user's layout.
	useEffect(() => {
		if (previousSession.current !== sessionId) {
			previousSession.current = sessionId;
			lastOpenSize.current = 50;
			setMobileOpen(false);
		}
		if (!hasWidget) setMobileOpen(false);
		changeDisplay(hasWidget ? (mainWidget?.display ?? "open") : "collapsed");
	}, [
		sessionId,
		widget?.instance_id,
		widget?.surface_id,
		mainWidget?.command_id,
		mainWidget?.display,
		hasWidget,
		changeDisplay,
	]);

	// A narrow screen keeps the whole chat available. Only its button opens
	// the widget; workflow display commands still control the desktop pane.
	// biome-ignore lint/correctness/useExhaustiveDependencies: crossing the breakpoint closes any mobile dialog without changing the desktop preference.
	useEffect(() => {
		setMobileOpen(false);
	}, [isMobile]);

	useEffect(() => {
		const size = lastOpenSize.current;
		const group = groupRef.current;
		if (group?.getLayout().length !== 2) return;
		group.setLayout(
			isMobile || !hasWidget || display === "collapsed"
				? [100, 0]
				: display === "maximized"
					? [0, 100]
					: [100 - size, size],
		);
	}, [isMobile, hasWidget, display]);

	// The same dialog hosts the renderer in both layouts. Native modal dialogs
	// enter the top layer without moving their DOM, preserving browser frames.
	useEffect(() => {
		const dialog = dialogRef.current;
		if (!dialog) return;
		const shouldOpen = hasWidget && (!isMobile || mobileOpen);
		const modal = isMobile && mobileOpen;
		if (dialog.open && (!shouldOpen || modalRef.current !== modal)) {
			dialog.close();
		}
		if (shouldOpen && !dialog.open) {
			if (modal) dialog.showModal();
			else dialog.open = true;
		}
		modalRef.current = modal;
	}, [isMobile, mobileOpen, hasWidget]);

	const onLayout = useCallback(
		([chatSize, widgetSize]: number[]) => {
			if (!hasWidget || isMobile) return;
			if (widgetSize === 0) {
				setDisplay("collapsed");
			} else if (chatSize === 0) {
				setDisplay("maximized");
			} else {
				lastOpenSize.current = widgetSize;
				setDisplay("open");
			}
		},
		[hasWidget, isMobile],
	);

	const collapsed =
		!hasWidget || (isMobile ? !mobileOpen : display === "collapsed");
	const maximized = !isMobile && hasWidget && display === "maximized";

	return (
		<div
			className="relative flex h-full min-h-0 w-full min-w-0 flex-1 overflow-hidden"
			data-main-widget-display={
				hasWidget
					? isMobile
						? mobileOpen
							? "dialog"
							: "collapsed"
						: display
					: "empty"
			}
			onKeyDown={(event) => {
				if (event.key === "Escape" && maximized && !event.defaultPrevented) {
					event.preventDefault();
					changeDisplay("open");
				}
			}}
		>
			<ResizablePanelGroup
				ref={groupRef}
				direction="horizontal"
				onLayout={onLayout}
				className="min-h-0 min-w-0"
			>
				<ResizablePanel
					id={`${id}-chat`}
					order={1}
					defaultSize={hasWidget ? 50 : 100}
					minSize={20}
					collapsible
					collapsedSize={0}
					className="flex min-h-0 min-w-0 flex-col"
				>
					<div
						className="flex h-full min-h-0 min-w-0 flex-1 flex-col"
						inert={maximized}
						aria-hidden={maximized || undefined}
					>
						{children}
					</div>
				</ResizablePanel>
				<ResizableHandle
					withHandle
					aria-label={t("resizeMainWidget", "Resize main widget")}
					disabled={isMobile || collapsed || maximized}
					style={{
						display: isMobile || collapsed || maximized ? "none" : undefined,
					}}
				/>
				<ResizablePanel
					id={`${id}-widget`}
					order={2}
					defaultSize={hasWidget ? 50 : 0}
					minSize={20}
					collapsible
					collapsedSize={0}
					className="min-h-0 min-w-0"
				>
					<dialog
						ref={dialogRef}
						id={`${id}-content`}
						aria-labelledby={`${id}-title`}
						aria-modal={isMobile && mobileOpen ? true : undefined}
						aria-hidden={collapsed || undefined}
						inert={collapsed}
						className="border-0 bg-background p-0 text-foreground backdrop:bg-black/50"
						style={{
							position: isMobile ? "fixed" : "static",
							inset: isMobile ? 0 : undefined,
							margin: isMobile ? "auto" : 0,
							width: isMobile ? "calc(100vw - 1rem)" : "100%",
							height: isMobile ? "calc(100dvh - 1rem)" : "100%",
							maxWidth: "none",
							maxHeight: "none",
							borderRadius: isMobile ? "0.75rem" : 0,
						}}
						onCancel={(event) => {
							event.preventDefault();
							setMobileOpen(false);
						}}
						onClose={() => {
							if (!dialogRef.current?.open) setMobileOpen(false);
						}}
					>
						<section className="flex h-full min-h-0 flex-col">
							<div className="flex h-12 shrink-0 items-center gap-2 border-b px-3">
								<span
									id={`${id}-title`}
									className="min-w-0 flex-1 truncate text-sm font-medium"
								>
									{title}
								</span>
								{!isMobile && (
									<Button
										type="button"
										variant="ghost"
										size="icon"
										className="size-8"
										title={
											maximized
												? t("restoreMainWidget", "Restore main widget")
												: t("maximizeMainWidget", "Maximize main widget")
										}
										aria-label={
											maximized
												? t("restoreMainWidget", "Restore main widget")
												: t("maximizeMainWidget", "Maximize main widget")
										}
										onClick={() =>
											changeDisplay(maximized ? "open" : "maximized")
										}
									>
										{maximized ? (
											<Minimize2 className="size-4" />
										) : (
											<Maximize2 className="size-4" />
										)}
									</Button>
								)}
								<Button
									type="button"
									variant="ghost"
									size="icon"
									className="size-8"
									title={
										isMobile
											? t("closeMainWidget", "Close main widget")
											: t("collapseMainWidget", "Collapse main widget")
									}
									aria-label={
										isMobile
											? t("closeMainWidget", "Close main widget")
											: t("collapseMainWidget", "Collapse main widget")
									}
									onClick={() => {
										if (isMobile) setMobileOpen(false);
										else changeDisplay("collapsed");
										requestAnimationFrame(() => reopenRef.current?.focus());
									}}
								>
									{isMobile ? (
										<X className="size-4" />
									) : (
										<PanelRightClose className="size-4" />
									)}
								</Button>
							</div>
							<div className="min-h-0 flex-1 overflow-hidden">
								{widget && (
									<MessageWidget
										key={`${sessionId}:${widget.instance_id}`}
										widget={widget}
										appId={runtime ? appId : (widget.origin?.appId ?? appId)}
										boardId={
											runtime ? boardId : (widget.origin?.boardId ?? boardId)
										}
										eventId={
											runtime ? eventId : (widget.origin?.eventId ?? eventId)
										}
										snapshots={snapshots}
										pane
									/>
								)}
							</div>
						</section>
					</dialog>
				</ResizablePanel>
			</ResizablePanelGroup>
			{hasWidget && (isMobile || collapsed) && (
				<Button
					ref={reopenRef}
					type="button"
					variant="outline"
					size="sm"
					className="absolute right-3 top-3 z-20 max-w-[70%] gap-2 rounded-full bg-background shadow-sm"
					aria-label={t("showMainWidget", "Show main widget")}
					aria-controls={`${id}-content`}
					aria-expanded={isMobile ? mobileOpen : false}
					aria-haspopup={isMobile ? "dialog" : undefined}
					onClick={() =>
						isMobile ? setMobileOpen(true) : changeDisplay("open")
					}
				>
					<PanelRightOpen className="size-4 shrink-0" />
					<span className="truncate">{title}</span>
				</Button>
			)}
		</div>
	);
}
