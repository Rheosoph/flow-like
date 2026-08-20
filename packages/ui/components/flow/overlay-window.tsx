"use client";

import { useTranslation } from "@flow-like/locales";
import { XIcon } from "lucide-react";
import type { CSSProperties, PointerEvent as ReactPointerEvent } from "react";
import type { ReactNode } from "react";
import { useCallback, useLayoutEffect, useRef, useState } from "react";
import { Dialog, DialogContent, DialogTitle, Input } from "../../components/ui";
import { cn } from "../../lib/utils";

/** Gap the window keeps to every viewport edge while being dragged. */
const VIEWPORT_MARGIN = 8;

interface IWindowPosition {
	x: number;
	y: number;
}

/**
 * The window the element actually lives in.
 *
 * Dialogs portal into `usePortalContainer()`, which the responsive preview points
 * at an iframe's mount node — measuring the global `window` there would clamp
 * against the host viewport instead of the one the window is rendered in.
 */
const viewportOf = (element: HTMLElement): Window =>
	element.ownerDocument?.defaultView ?? window;

const clampToViewport = (
	x: number,
	y: number,
	width: number,
	height: number,
	view: Window,
): IWindowPosition => {
	const maxX = Math.max(
		VIEWPORT_MARGIN,
		view.innerWidth - width - VIEWPORT_MARGIN,
	);
	const maxY = Math.max(
		VIEWPORT_MARGIN,
		view.innerHeight - height - VIEWPORT_MARGIN,
	);
	return {
		x: Math.min(Math.max(x, VIEWPORT_MARGIN), maxX),
		y: Math.min(Math.max(y, VIEWPORT_MARGIN), maxY),
	};
};

export interface IOverlayWindowProps {
	open: boolean;
	onOpenChange: (open: boolean) => void;
	/** Accessible name for the dialog; the visible title is the inline field. */
	title: string;
	icon: ReactNode;
	name: string;
	onNameChange: (next: string) => void;
	nameLabel: string;
	nameDisabled?: boolean;
	/** Sits beside the name — a call count, a usage count. */
	badge?: ReactNode;
	/** Extra title-bar controls, left of the close button. */
	actions?: ReactNode;
	/** The right-hand column. Hidden below `lg`, so nothing essential may live only here. */
	rail?: ReactNode;
	footer: ReactNode;
	children: ReactNode;
	className?: string;
}

/**
 * The floating editor window shared by the function and variable editors.
 *
 * It is a Radix dialog for focus trapping and Escape, wearing window chrome: a
 * title bar you drag it by, the name edited in place, and a footer that commits.
 * Dragging is plain pointer events clamped to the viewport, so the window can
 * never be lost off-screen, and it re-centres on every open.
 */
export function OverlayWindow({
	open,
	onOpenChange,
	title,
	icon,
	name,
	onNameChange,
	nameLabel,
	nameDisabled,
	badge,
	actions,
	rail,
	footer,
	children,
	className,
}: Readonly<IOverlayWindowProps>) {
	const { t } = useTranslation("flow");
	const windowRef = useRef<HTMLDivElement | null>(null);
	const dragOffset = useRef<IWindowPosition | null>(null);
	const [position, setPosition] = useState<IWindowPosition | null>(null);
	const [dragging, setDragging] = useState(false);

	// Centre on every open so the window never re-appears off to one side.
	useLayoutEffect(() => {
		if (!open) {
			setPosition(null);
			return;
		}
		const element = windowRef.current;
		if (!element) return;
		const { offsetWidth, offsetHeight } = element;
		const view = viewportOf(element);
		setPosition(
			clampToViewport(
				(view.innerWidth - offsetWidth) / 2,
				(view.innerHeight - offsetHeight) / 2,
				offsetWidth,
				offsetHeight,
				view,
			),
		);
	}, [open]);

	const startDrag = useCallback((event: ReactPointerEvent<HTMLDivElement>) => {
		if (event.button !== 0) return;
		const element = windowRef.current;
		if (!element) return;
		// Anything interactive in the bar opts out, or typing would drag the window.
		if ((event.target as HTMLElement).closest("[data-window-nodrag]")) return;
		const rect = element.getBoundingClientRect();
		dragOffset.current = {
			x: event.clientX - rect.left,
			y: event.clientY - rect.top,
		};
		setDragging(true);
		event.currentTarget.setPointerCapture(event.pointerId);
	}, []);

	const onDrag = useCallback((event: ReactPointerEvent<HTMLDivElement>) => {
		const offset = dragOffset.current;
		const element = windowRef.current;
		if (!offset || !element) return;
		setPosition(
			clampToViewport(
				event.clientX - offset.x,
				event.clientY - offset.y,
				element.offsetWidth,
				element.offsetHeight,
				viewportOf(element),
			),
		);
	}, []);

	const endDrag = useCallback((event: ReactPointerEvent<HTMLDivElement>) => {
		dragOffset.current = null;
		setDragging(false);
		if (event.currentTarget.hasPointerCapture(event.pointerId)) {
			event.currentTarget.releasePointerCapture(event.pointerId);
		}
	}, []);

	const windowStyle: CSSProperties = position
		? {
				left: `${position.x}px`,
				top: `${position.y}px`,
				transform: "none",
				translate: "none",
			}
		: {};

	return (
		<Dialog open={open} onOpenChange={onOpenChange}>
			{/* `sm:max-w-lg` sits in DialogContent's defaults and tailwind-merge keeps a
			    `sm:` variant alongside a base `max-w-none`, so the breakpoint override
			    has to be spelled out or the window renders at 512 px. */}
			<DialogContent
				ref={windowRef}
				showCloseButton={false}
				style={windowStyle}
				className={cn(
					"max-w-none sm:max-w-none w-[min(64rem,calc(100vw-2rem))] h-[min(45rem,calc(100dvh-2rem))] gap-0 overflow-hidden overflow-y-hidden rounded-2xl border border-border/40 bg-background/95 p-0 shadow-floating backdrop-blur-xl motion-reduce:animate-none",
					className,
				)}
				onDoubleClick={(event) => event.stopPropagation()}
			>
				<DialogTitle className="sr-only">{title}</DialogTitle>

				<div
					onPointerDown={startDrag}
					onPointerMove={onDrag}
					onPointerUp={endDrag}
					onPointerCancel={endDrag}
					className={cn(
						"flex shrink-0 select-none items-center gap-2 border-b border-border/40 bg-card/60 px-3 py-2",
						dragging ? "cursor-grabbing" : "cursor-grab",
					)}
				>
					<span className="flex size-7 shrink-0 items-center justify-center rounded-md border border-border/60">
						{icon}
					</span>

					<Input
						data-window-nodrag
						value={name}
						disabled={nameDisabled}
						onChange={(event) => onNameChange(event.target.value)}
						aria-label={nameLabel}
						placeholder={nameLabel}
						className="h-8 max-w-sm border-transparent bg-transparent px-2 text-sm font-semibold shadow-none hover:border-border focus-visible:border-border"
					/>

					{badge}

					<div
						className="ml-auto flex shrink-0 items-center gap-1"
						data-window-nodrag
					>
						{actions}
						<button
							type="button"
							onClick={() => onOpenChange(false)}
							aria-label={t("close", "Close")}
							className="flex size-7 items-center justify-center rounded-md text-muted-foreground transition-colors hover:bg-muted hover:text-foreground"
						>
							<XIcon className="size-4" />
						</button>
					</div>
				</div>

				<div className="flex min-h-0 flex-1 flex-row">
					<div className="flex min-w-0 flex-1 flex-col">{children}</div>
					{rail && (
						<aside className="hidden w-[19rem] shrink-0 flex-col border-l border-border/40 bg-card/40 lg:flex">
							{rail}
						</aside>
					)}
				</div>

				<footer className="flex shrink-0 items-center justify-between gap-3 border-t border-border/40 bg-card/40 px-4 py-3">
					{footer}
				</footer>
			</DialogContent>
		</Dialog>
	);
}
