"use client";

import { useTranslation } from "@flow-like/locales";
import CopyIcon from "lucide-react/dist/esm/icons/copy.js";
import PlusIcon from "lucide-react/dist/esm/icons/plus.js";
import ReplyIcon from "lucide-react/dist/esm/icons/reply.js";
import EditIcon from "lucide-react/dist/esm/icons/square-pen.js";
import {
	type ReactNode,
	type RefObject,
	createContext,
	useContext,
	useEffect,
	useMemo,
	useState,
	useSyncExternalStore,
} from "react";
import { cn } from "../../../lib/utils";
import { Popover, PopoverAnchor, PopoverContent } from "../../ui/popover";
import { COMMON_REACTIONS, EmojiChoices } from "./message-reactions";

const LONG_PRESS_MS = 380;
const MOVE_TOLERANCE_PX = 8;
const QUICK_REACTIONS = COMMON_REACTIONS.slice(0, 6);
const MOUSE_QUERY = "(hover: hover) and (pointer: fine)";

/** Provided around `renderMenu` output so a menu can render as rows instead of a dropdown. */
export const MessageMenuSurfaceContext = createContext<{
	close: () => void;
} | null>(null);

export function useMessageMenuSurface() {
	return useContext(MessageMenuSurfaceContext);
}

let mouseQuery:
	| { matchMedia: typeof window.matchMedia; query: MediaQueryList }
	| undefined;

/** One query for every row; a replaced `matchMedia` (tests, embedders) gets a new one. */
function sharedMouseQuery(): MediaQueryList | null {
	if (typeof window === "undefined" || typeof window.matchMedia !== "function")
		return null;
	if (mouseQuery?.matchMedia !== window.matchMedia) {
		mouseQuery = {
			matchMedia: window.matchMedia,
			query: window.matchMedia(MOUSE_QUERY),
		};
	}
	return mouseQuery.query;
}

function subscribeTouchOnly(onChange: () => void) {
	const query = sharedMouseQuery();
	query?.addEventListener?.("change", onChange);
	return () => query?.removeEventListener?.("change", onChange);
}

const readTouchOnly = () => {
	const query = sharedMouseQuery();
	return query ? !query.matches : false;
};

/** Right on the first render, so a transcript doesn't render every row again once mounted. */
export function useTouchOnly() {
	return useSyncExternalStore(subscribeTouchOnly, readTouchOnly, () => false);
}

/** Calls `onTrigger` once the finger rests on the element; moving or scrolling cancels. */
export function useLongPress(
	elementRef: RefObject<HTMLElement | null>,
	enabled: boolean,
	onTrigger: () => void,
) {
	useEffect(() => {
		const element = elementRef.current;
		if (!enabled || !element) return;
		let timer: ReturnType<typeof setTimeout> | undefined;
		let origin: { x: number; y: number } | undefined;
		const cancel = () => {
			if (origin) window.removeEventListener("scroll", cancel, true);
			clearTimeout(timer);
			timer = undefined;
			origin = undefined;
			element.removeAttribute("data-fl-pressing");
		};
		let released = false;
		const swallowReleaseClick = () => {
			const stop = (event: Event) => {
				if (!element.contains(event.target as Node)) return;
				event.preventDefault();
				event.stopPropagation();
			};
			document.addEventListener("click", stop, { capture: true, once: true });
			setTimeout(() => document.removeEventListener("click", stop, true), 400);
		};
		const release = () => {
			if (released) swallowReleaseClick();
			released = false;
			cancel();
		};
		const start = (event: TouchEvent) => {
			cancel();
			if (event.touches.length !== 1) return;
			const touch = event.touches[0];
			origin = { x: touch.clientX, y: touch.clientY };
			window.addEventListener("scroll", cancel, {
				capture: true,
				passive: true,
			});
			element.setAttribute("data-fl-pressing", "");
			timer = setTimeout(() => {
				cancel();
				released = true;
				onTrigger();
			}, LONG_PRESS_MS);
		};
		const move = (event: TouchEvent) => {
			const touch = event.touches[0];
			if (
				origin &&
				touch &&
				Math.hypot(touch.clientX - origin.x, touch.clientY - origin.y) >
					MOVE_TOLERANCE_PX
			)
				cancel();
		};
		const suppressCallout = (event: Event) => event.preventDefault();
		const options = { passive: true } as const;
		element.addEventListener("touchstart", start, options);
		element.addEventListener("touchmove", move, options);
		element.addEventListener("touchend", release, options);
		element.addEventListener("touchcancel", cancel, options);
		element.addEventListener("contextmenu", suppressCallout);
		return () => {
			cancel();
			element.removeEventListener("touchstart", start);
			element.removeEventListener("touchmove", move);
			element.removeEventListener("touchend", release);
			element.removeEventListener("touchcancel", cancel);
			element.removeEventListener("contextmenu", suppressCallout);
		};
	}, [elementRef, enabled, onTrigger]);
}

export function TouchMenuRow({
	icon,
	label,
	onSelect,
	destructive,
	disabled,
}: Readonly<{
	icon: ReactNode;
	label: string;
	onSelect: () => void;
	destructive?: boolean;
	disabled?: boolean;
}>) {
	const surface = useMessageMenuSurface();
	return (
		<button
			type="button"
			disabled={disabled}
			onClick={() => {
				surface?.close();
				onSelect();
			}}
			className={cn(
				"flex min-h-11 w-full items-center gap-3 rounded-md px-3 text-left text-sm outline-none active:bg-accent disabled:opacity-50 [&_svg]:size-4 [&_svg]:shrink-0",
				destructive ? "text-destructive" : "text-foreground",
			)}
		>
			{icon}
			{label}
		</button>
	);
}

function QuickReactions({
	reactions,
	disabled,
	onSelect,
	onMore,
}: Readonly<{
	reactions?: readonly string[];
	disabled?: boolean;
	onSelect: (emoji: string) => void;
	onMore: () => void;
}>) {
	const { t } = useTranslation("chat");
	return (
		<div className="flex items-center justify-between gap-1 px-1 pb-1">
			{QUICK_REACTIONS.map((emoji) => (
				<button
					key={emoji}
					type="button"
					disabled={disabled}
					aria-label={t("toggleReaction", "React with {{emoji}}", { emoji })}
					aria-pressed={reactions?.includes(emoji) ?? false}
					onClick={() => onSelect(emoji)}
					className="flex size-10 items-center justify-center rounded-full text-2xl outline-none active:bg-accent aria-pressed:bg-primary/15 disabled:opacity-50"
				>
					{emoji}
				</button>
			))}
			<button
				type="button"
				disabled={disabled}
				aria-label={t("moreReactions", "More reactions")}
				onClick={onMore}
				className="flex size-10 items-center justify-center rounded-full bg-muted text-muted-foreground outline-none active:bg-accent disabled:opacity-50"
			>
				<PlusIcon className="size-5" />
			</button>
		</div>
	);
}

export function MessageTouchMenu({
	anchorRef,
	open,
	onOpenChange,
	align,
	reactions,
	onToggleReaction,
	disabled,
	onReply,
	onCopy,
	onEdit,
	extra,
}: Readonly<{
	anchorRef: RefObject<HTMLElement | null>;
	open: boolean;
	onOpenChange: (open: boolean) => void;
	align: "start" | "end";
	reactions?: readonly string[];
	onToggleReaction?: (emoji: string) => void;
	disabled?: boolean;
	onReply?: () => void;
	onCopy: () => void;
	onEdit?: () => void;
	extra?: ReactNode;
}>) {
	const { t } = useTranslation("chat");
	const [picking, setPicking] = useState(false);
	useEffect(() => {
		if (!open) setPicking(false);
	}, [open]);
	const anchor = useMemo(
		() => ({
			current: {
				getBoundingClientRect: () =>
					anchorRef.current?.getBoundingClientRect() ?? new DOMRect(),
			},
		}),
		[anchorRef],
	);
	const close = () => onOpenChange(false);
	const react = (emoji: string) => {
		close();
		onToggleReaction?.(emoji);
	};
	return (
		<Popover open={open} onOpenChange={onOpenChange}>
			<PopoverAnchor virtualRef={anchor} />
			<PopoverContent
				data-fl-chat-touch-menu
				side="bottom"
				align={align}
				sideOffset={8}
				collisionPadding={12}
				aria-label={t("messageActions", "Message actions")}
				className="w-72 max-w-[calc(100vw-1.5rem)] select-none bg-popover p-2 backdrop-blur-none motion-reduce:animate-none"
				onOpenAutoFocus={(event) => event.preventDefault()}
				onCloseAutoFocus={(event) => event.preventDefault()}
			>
				{picking && onToggleReaction ? (
					<div className="space-y-2">
						<EmojiChoices
							reactions={reactions}
							disabled={disabled}
							onSelect={react}
						/>
					</div>
				) : (
					<>
						{onToggleReaction && (
							<QuickReactions
								reactions={reactions}
								disabled={disabled}
								onSelect={react}
								onMore={() => setPicking(true)}
							/>
						)}
						{onToggleReaction && <div className="my-1 h-px bg-border/60" />}
						<MessageMenuSurfaceContext.Provider value={{ close }}>
							{onReply && (
								<TouchMenuRow
									icon={<ReplyIcon aria-hidden />}
									label={t("touchMenuReply", "Reply")}
									onSelect={onReply}
									disabled={disabled}
								/>
							)}
							<TouchMenuRow
								icon={<CopyIcon aria-hidden />}
								label={t("touchMenuCopy", "Copy")}
								onSelect={onCopy}
							/>
							{onEdit && (
								<TouchMenuRow
									icon={<EditIcon aria-hidden />}
									label={t("touchMenuEdit", "Edit")}
									onSelect={onEdit}
								/>
							)}
							{extra}
						</MessageMenuSurfaceContext.Provider>
					</>
				)}
			</PopoverContent>
		</Popover>
	);
}
