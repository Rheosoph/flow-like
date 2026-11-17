"use client";

import { useTranslation } from "@flow-like/locales";
import {
	GlobeIcon,
	HandIcon,
	Maximize2Icon,
	Minimize2Icon,
	MonitorIcon,
	MonitorOffIcon,
	MousePointerClickIcon,
	SquareIcon,
} from "lucide-react";
import { type ReactNode, useEffect, useRef, useState } from "react";
import { createPortal } from "react-dom";
import type {
	IBotComputerView,
	IBotInputEvent,
} from "../../state/backend-state/bot-state";
import { Button } from "../ui/button";
import {
	buttonName,
	keyToInput,
	mapToFrame,
	wheelToInput,
} from "./computer-input";
import {
	ComputerObservationStatus,
	ObservedComputerImage,
	useComputerFrameAge,
} from "./computer-observation";

const MOVE_EVERY_MS = 40;
const ESCAPE_HOLD_MS = 1000;

export type ComputerStageState = "asleep" | "active" | "failed";

/** What the tab shows: a sleeping computer, the bot's latest picture, or a read that failed. */
export function computerStageState(
	view: IBotComputerView | undefined,
	failed: boolean,
): ComputerStageState {
	if (view?.state === "active" && view.image) return "active";
	return failed && !view ? "failed" : "asleep";
}

/**
 * The picture while the owner drives it: pointer, wheel and keyboard on the image go to the bot's
 * screen as pixels of the frame. Holding Esc hands back; a tap sends Escape.
 */
export function ControlSurface({
	view,
	alt,
	onInput,
	onHandBack,
}: Readonly<{
	view: IBotComputerView;
	alt: string;
	onInput?: (event: IBotInputEvent) => void;
	onHandBack?: () => void;
}>) {
	const image = useRef<HTMLImageElement>(null);
	const lastMove = useRef(0);
	const escapeHold = useRef<ReturnType<typeof setTimeout> | null>(null);
	const frame = {
		width: view.width ?? image.current?.naturalWidth ?? 0,
		height: view.height ?? image.current?.naturalHeight ?? 0,
	};

	const place = (
		event: { clientX: number; clientY: number },
		clamp = false,
	): { x: number; y: number } | null => {
		const element = image.current;
		if (!element) return null;
		return mapToFrame(event, element.getBoundingClientRect(), frame, clamp);
	};

	const clearEscapeHold = () => {
		if (escapeHold.current) clearTimeout(escapeHold.current);
		escapeHold.current = null;
	};

	return (
		<div
			role="application"
			// biome-ignore lint/a11y/noNoninteractiveTabindex: the remote screen takes keyboard focus while the owner drives it
			tabIndex={0}
			aria-label={alt}
			className="flex h-full w-full cursor-crosshair items-center justify-center outline-none focus-visible:ring-2 focus-visible:ring-primary"
			onPointerDown={(event) => {
				event.currentTarget.focus();
				const at = place(event);
				if (!at) return;
				event.currentTarget.setPointerCapture?.(event.pointerId);
				onInput?.({
					type: "pointer_down",
					...at,
					button: buttonName(event.button),
				});
			}}
			onPointerUp={(event) => {
				const at = place(event, true);
				if (at)
					onInput?.({
						type: "pointer_up",
						...at,
						button: buttonName(event.button),
					});
			}}
			onPointerMove={(event) => {
				const now = Date.now();
				if (now - lastMove.current < MOVE_EVERY_MS) return;
				const at = place(event);
				if (!at) return;
				lastMove.current = now;
				onInput?.({ type: "pointer_move", ...at });
			}}
			onContextMenu={(event) => event.preventDefault()}
			onWheel={(event) => {
				const at = place(event);
				if (at) onInput?.(wheelToInput(event, at));
			}}
			onBlur={clearEscapeHold}
			onKeyDown={(event) => {
				event.preventDefault();
				if (event.key === "Escape") {
					if (!event.repeat && !escapeHold.current) {
						escapeHold.current = setTimeout(() => {
							escapeHold.current = null;
							onHandBack?.();
						}, ESCAPE_HOLD_MS);
					}
					return;
				}
				const input = keyToInput(event);
				if (input) onInput?.(input);
			}}
			onKeyUp={(event) => {
				if (event.key === "Escape" && escapeHold.current) {
					clearEscapeHold();
					onInput?.({ type: "key", chord: "Escape" });
				}
			}}
		>
			<img
				ref={image}
				src={view.image ?? undefined}
				alt={alt}
				draggable={false}
				className="max-h-full max-w-full select-none object-contain"
			/>
		</div>
	);
}

function MaximizedLayer({ children }: Readonly<{ children: ReactNode }>) {
	return createPortal(
		<div className="fixed inset-0 z-[100] flex flex-col bg-background pt-safe pb-safe px-safe">
			{children}
		</div>,
		document.body,
	);
}

export function ComputerStage({
	state,
	view,
	name,
	stopping,
	onStop,
	onRetry,
	onTakeOver,
	onHandBack,
	onInput,
}: Readonly<{
	state: ComputerStageState;
	view?: IBotComputerView;
	name: string;
	stopping?: boolean;
	onStop: () => void;
	onRetry: () => void;
	onTakeOver?: () => void;
	onHandBack?: () => void;
	onInput?: (event: IBotInputEvent) => void;
}>) {
	const { t } = useTranslation("bots");
	const age = useComputerFrameAge(view);
	const [maximized, setMaximized] = useState(false);
	const isActive = state === "active" && Boolean(view?.image);
	useEffect(() => {
		if (!isActive) setMaximized(false);
	}, [isActive]);
	useEffect(() => {
		if (!maximized) return;
		const onKey = (event: KeyboardEvent) => {
			if (event.key !== "Escape") return;
			const target = event.target as Element | null;
			if (target?.closest?.('[role="application"]')) return;
			setMaximized(false);
		};
		document.addEventListener("keydown", onKey);
		return () => document.removeEventListener("keydown", onKey);
	}, [maximized]);
	if (state === "active" && view?.image) {
		const takenOver = Boolean(view.taken_over);
		const canTakeOver = Boolean(onTakeOver) && view.can_control !== false;
		const driving = takenOver && view.can_control === true && Boolean(onInput);
		const alt = t("computer.alt", "What {{name}} sees right now", { name });
		const KindIcon = view.kind === "browser" ? GlobeIcon : MonitorIcon;
		const MaximizeIcon = maximized ? Minimize2Icon : Maximize2Icon;
		const maximizeLabel = maximized
			? t("computer.restore", "Restore size")
			: t("computer.maximize", "Maximize");
		const content = (
			<div className="flex min-h-0 flex-1 flex-col gap-2 p-3">
				<div className="flex items-center gap-2 text-xs text-muted-foreground">
					<KindIcon className="size-3.5 shrink-0" aria-hidden />
					<span className="min-w-0 flex-1 truncate" title={view.caption ?? ""}>
						{view.caption ||
							(view.kind === "browser"
								? t("computer.browser", "Browser")
								: t("computer.screen", "Screen"))}
					</span>
					{takenOver ? (
						<Button
							type="button"
							size="sm"
							className="h-7 gap-1.5 px-2 text-xs"
							onClick={onHandBack}
						>
							<HandIcon className="size-3" aria-hidden />
							{t("computer.handBack", "Hand back")}
						</Button>
					) : canTakeOver ? (
						<Button
							type="button"
							variant="outline"
							size="sm"
							className="h-7 gap-1.5 px-2 text-xs"
							onClick={onTakeOver}
						>
							<MousePointerClickIcon className="size-3" aria-hidden />
							{t("computer.takeOver", "Take over")}
						</Button>
					) : view.can_control === false && onTakeOver ? (
						<span className="rounded-full bg-muted px-2 py-0.5 text-[11px]">
							{t("computer.watchOnly", "Watch only")}
						</span>
					) : null}
					<Button
						type="button"
						variant="outline"
						size="sm"
						className="h-7 gap-1.5 px-2 text-xs"
						disabled={stopping}
						onClick={onStop}
					>
						<SquareIcon className="size-3 fill-current" aria-hidden />
						{t("computer.stop", "Stop")}
					</Button>
					<Button
						type="button"
						variant="outline"
						size="icon"
						className="size-7"
						aria-label={maximizeLabel}
						title={maximizeLabel}
						onClick={() => setMaximized((open) => !open)}
					>
						<MaximizeIcon className="size-3.5" aria-hidden />
					</Button>
				</div>
				{takenOver && (
					<p className="rounded-md bg-primary/10 px-2.5 py-1.5 text-xs text-foreground">
						<span className="font-medium">
							{t("computer.inControl", "You're in control")}
						</span>{" "}
						{driving
							? t(
									"computer.inControlHint",
									"Click the picture, then use your mouse and keyboard. Hold Esc to hand back.",
								)
							: t(
									"computer.inControlLocal",
									"{{name}} is paused. Use your own mouse and keyboard, then hand back.",
									{ name },
								)}
					</p>
				)}
				<ComputerObservationStatus view={view} age={age} />
				{view.notice && (
					<output className="text-xs text-destructive">{view.notice}</output>
				)}
				<div
					className={`flex min-h-0 flex-1 items-center justify-center overflow-hidden rounded-lg border bg-muted/40 ${takenOver ? "border-primary ring-2 ring-primary/40" : ""}`}
				>
					{driving ? (
						<ControlSurface
							view={view}
							alt={alt}
							onInput={onInput}
							onHandBack={onHandBack}
						/>
					) : (
						<ObservedComputerImage view={view} alt={alt} age={age} />
					)}
				</div>
			</div>
		);
		return maximized ? <MaximizedLayer>{content}</MaximizedLayer> : content;
	}
	const failed = state === "failed";
	return (
		<div className="flex flex-1 flex-col items-center justify-center gap-3 px-6 py-10 text-center">
			<span className="flex size-11 items-center justify-center rounded-2xl bg-muted text-muted-foreground [&_svg]:size-5">
				<MonitorOffIcon aria-hidden />
			</span>
			<p className="max-w-72 text-sm font-medium">
				{failed
					? t("computer.failed", "Couldn't reach {{name}}'s computer.", {
							name,
						})
					: t("computer.asleep", "Computer is asleep")}
			</p>
			{!failed && (
				<p className="max-w-72 text-xs text-muted-foreground">
					{t(
						"computer.asleepHint",
						"{{name}} wakes it when it opens a browser or uses this computer.",
						{ name },
					)}
				</p>
			)}
			{failed && (
				<Button type="button" variant="outline" size="sm" onClick={onRetry}>
					{t("crew.retry", "Try again")}
				</Button>
			)}
			{view?.taken_over && onHandBack && (
				<Button type="button" size="sm" onClick={onHandBack}>
					{t("computer.handBack", "Hand back")}
				</Button>
			)}
		</div>
	);
}
