"use client";

import { useTranslation } from "@flow-like/locales";
import { useMutation, useQuery } from "@tanstack/react-query";
import {
	GlobeIcon,
	HandIcon,
	MonitorIcon,
	MonitorOffIcon,
	MousePointerClickIcon,
	SquareIcon,
} from "lucide-react";
import { useCallback, useRef } from "react";
import { useBackend } from "../../state/backend-state";
import type {
	IBot,
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

const ACTIVE_POLL_MS = 2000;
const ASLEEP_POLL_MS = 5000;
const CONTROL_POLL_MS = 600;
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
	if (state === "active" && view?.image) {
		const takenOver = Boolean(view.taken_over);
		const canTakeOver = Boolean(onTakeOver) && view.can_control !== false;
		const driving = takenOver && view.can_control === true && Boolean(onInput);
		const alt = t("computer.alt", "What {{name}} sees right now", { name });
		const KindIcon = view.kind === "browser" ? GlobeIcon : MonitorIcon;
		return (
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

/** Device bots' screens open over a tunnel only the desktop app has a client for. */
export function ComputerDesktopOnly({ name }: Readonly<{ name: string }>) {
	const { t } = useTranslation("bots");
	return (
		<div className="flex flex-1 flex-col items-center justify-center gap-3 px-6 py-10 text-center">
			<span className="flex size-11 items-center justify-center rounded-2xl bg-muted text-muted-foreground [&_svg]:size-5">
				<MonitorOffIcon aria-hidden />
			</span>
			<p className="max-w-72 text-sm font-medium">
				{t("computer.webOnlyDesktop", "Watch and take over in the desktop app")}
			</p>
			<p className="max-w-72 text-xs text-muted-foreground">
				{t(
					"computer.webOnlyDesktopHint",
					"{{name}} runs on a device. Its screen is end-to-end encrypted and opens only in the desktop app.",
					{ name },
				)}
			</p>
		</div>
	);
}

/**
 * The Computer tab: the latest picture of what the bot's browser or screen shows, pulled every
 * couple of seconds while it runs (faster while the owner is in control). A device bot's picture
 * comes over the encrypted device tunnel and can be taken over; where the app has no tunnel client
 * (the web app) it says so.
 */
export function BotComputerPane({
	bot,
	name,
}: Readonly<{ bot: IBot; name: string }>) {
	const host = useBackend().botState?.computer;
	const deviceId = bot.location === "device" ? bot.device_id : null;
	const query = useQuery({
		queryKey: ["bot-computer", bot.id, deviceId],
		queryFn: () =>
			host ? host.view(bot.id, deviceId) : Promise.resolve(undefined),
		enabled: Boolean(host),
		refetchInterval: (current) =>
			current.state.data?.taken_over
				? CONTROL_POLL_MS
				: current.state.data?.state === "active"
					? ACTIVE_POLL_MS
					: ASLEEP_POLL_MS,
		refetchIntervalInBackground: false,
	});
	const act = useMutation({
		mutationFn: async (run: () => Promise<void>) => {
			await run();
			await query.refetch();
		},
	});
	const send = useCallback(
		(event: IBotInputEvent) => {
			void host?.input(bot.id, event, deviceId).catch(() => {});
		},
		[host, bot.id, deviceId],
	);
	if (!host && deviceId) return <ComputerDesktopOnly name={name} />;
	return (
		<ComputerStage
			state={computerStageState(query.data, query.isError)}
			view={query.data}
			name={name}
			stopping={act.isPending}
			onStop={() =>
				act.mutate(() => host?.stop(bot.id, deviceId) ?? Promise.resolve())
			}
			onRetry={() => void query.refetch()}
			onTakeOver={() =>
				act.mutate(() => host?.takeOver(bot.id, deviceId) ?? Promise.resolve())
			}
			onHandBack={() =>
				act.mutate(() => host?.handBack(bot.id, deviceId) ?? Promise.resolve())
			}
			onInput={deviceId ? send : undefined}
		/>
	);
}
