"use client";

import { useTranslation } from "@flow-like/locales";
import { ArrowUp, Check, Crosshair, type LucideIcon, Zap } from "lucide-react";
import {
	type FormEvent,
	type MouseEvent,
	type PointerEvent,
	type Ref,
	useEffect,
	useRef,
	useState,
} from "react";
import type { GooPoint, GooRect } from "../../lib/bot-goo/scene";
import { PERCH_AVATAR } from "../../lib/bot-goo/scene";
import { BotAvatar } from "../bots/bot-avatar";
import type { BotAgentState, BotAppearance, BotMood } from "../bots/types";
import { Button } from "../ui/button";

/** What a bot's face shows. */
export interface GooActivity {
	agentState: BotAgentState;
	mood?: BotMood;
}

export interface BotHandlers {
	onPointerDown: (event: PointerEvent<HTMLElement>) => void;
	onPointerMove: (event: PointerEvent<HTMLElement>) => void;
	onPointerUp: (event: PointerEvent<HTMLElement>) => void;
	onPointerCancel: (event: PointerEvent<HTMLElement>) => void;
	onPointerEnter: () => void;
	onPointerLeave: () => void;
	onContextMenu: (event: MouseEvent<HTMLElement>) => void;
	/** Keyboard presses only; the pointer handlers own mouse clicks. */
	onClick: (event: MouseEvent<HTMLElement>) => void;
}

export function GooBotNode({
	name,
	appearance,
	autoApprove = false,
	activity,
	gaze,
	nodeRef,
	handlers,
}: Readonly<{
	name: string;
	appearance: Partial<BotAppearance>;
	autoApprove?: boolean;
	activity: GooActivity;
	gaze: GooPoint | null;
	nodeRef: Ref<HTMLButtonElement>;
	handlers: BotHandlers;
}>) {
	const { t } = useTranslation("bots");
	const working =
		activity.agentState === "working" || activity.mood === "thinking";
	const waiting = activity.agentState === "needs-you";
	return (
		<button
			ref={nodeRef}
			type="button"
			aria-label={name}
			className="absolute left-0 top-0 cursor-grab touch-none rounded-full border-0 bg-transparent p-0 outline-none focus-visible:ring-2 focus-visible:ring-ring active:cursor-grabbing"
			style={{
				width: PERCH_AVATAR,
				height: PERCH_AVATAR,
				transformOrigin: "50% 60%",
				willChange: "transform",
				filter:
					"drop-shadow(0 calc(var(--lift, 0) * 14px) calc(var(--lift, 0) * 10px + 0.5px) rgb(0 0 0 / calc(var(--lift, 0) * 0.55)))",
			}}
			aria-busy={working || undefined}
			{...handlers}
		>
			<BotAvatar
				appearance={appearance}
				size={PERCH_AVATAR}
				agentState={activity.agentState}
				mood={activity.mood}
				gx={gaze?.x}
				gy={gaze?.y}
				decorative
			/>
			{waiting && (
				<span
					className="pointer-events-none absolute -top-0.5 -right-0.5 size-2.5 rounded-full bg-primary ring-2 ring-[#09090b]"
					aria-hidden
				/>
			)}
			{autoApprove && (
				<span
					className="pointer-events-none absolute -right-1 -bottom-1 flex size-3.5 items-center justify-center rounded-full bg-foreground text-background shadow-sm"
					title={t("notch.autoApproveOn", "Approves automatically")}
				>
					<Zap className="size-2.5 fill-current" aria-hidden />
				</span>
			)}
		</button>
	);
}

export function GooPrompt({
	nodeRef,
	botName,
	target,
	onSubmit,
	onCancel,
}: Readonly<{
	nodeRef: Ref<HTMLFormElement>;
	botName: string;
	target: string | null;
	onSubmit: (text: string) => void;
	onCancel: () => void;
}>) {
	const { t } = useTranslation("bots");
	const [text, setText] = useState("");
	const input = useRef<HTMLInputElement>(null);

	useEffect(() => {
		const timer = window.setTimeout(() => input.current?.focus(), 160);
		return () => window.clearTimeout(timer);
	}, []);

	const submit = (event: FormEvent) => {
		event.preventDefault();
		const value = text.trim();
		if (value) onSubmit(value);
	};

	return (
		<form
			ref={nodeRef}
			onSubmit={submit}
			className="absolute left-0 top-0 flex items-center gap-2 rounded-full pl-11 pr-1.5 opacity-0 transition-shadow duration-300 focus-within:shadow-[0_0_0_1px_color-mix(in_oklab,var(--foreground)_18%,transparent),0_0_24px_-4px_color-mix(in_oklab,var(--primary)_55%,transparent)]"
		>
			{target && (
				<span className="pointer-events-none absolute bottom-full left-11 mb-1.5 flex max-w-[calc(100%-4rem)] items-center gap-1 truncate rounded-full bg-popover/90 px-2 py-0.5 text-[11px] text-muted-foreground shadow-sm">
					<Crosshair className="size-3 shrink-0" aria-hidden />
					<span className="truncate">{target}</span>
				</span>
			)}
			<input
				ref={input}
				value={text}
				onChange={(event) => setText(event.target.value)}
				onKeyDown={(event) => {
					if (event.key === "Escape") onCancel();
				}}
				placeholder={
					target
						? t("notch.promptHere", "What should {{name}} do here?", {
								name: botName,
							})
						: t("notch.prompt", "What should {{name}} do?", {
								name: botName,
							})
				}
				aria-label={t("notch.promptLabel", "Task for {{name}}", {
					name: botName,
				})}
				className="min-w-0 flex-1 bg-transparent text-sm text-foreground outline-none placeholder:text-muted-foreground"
			/>
			<Button
				type="submit"
				size="icon"
				className="size-8 shrink-0 rounded-full bg-foreground text-background transition-transform hover:scale-105 hover:bg-foreground disabled:bg-muted disabled:text-muted-foreground disabled:opacity-100"
				disabled={!text.trim()}
				aria-label={t("notch.send", "Send")}
			>
				<ArrowUp className="size-4" />
			</Button>
		</form>
	);
}

const LANDING = 42;

/** What a dragged bot is over: its outline, its name, and where the bot will sit. */
export function DropFrame({
	bounds,
	label,
	refused,
	landing,
	rounded = "rounded-xl",
	icon: Icon = Crosshair,
	labelAbove = false,
}: Readonly<{
	bounds: GooRect;
	label: string;
	refused: boolean;
	landing: GooPoint | null;
	rounded?: string;
	/** What kind of thing it is; a crosshair by default. */
	icon?: LucideIcon;
	/** Over the landing spot rather than under it, where it would hide a small target's title. */
	labelAbove?: boolean;
}>) {
	return (
		<div
			className={`pointer-events-none absolute border-2 animate-in fade-in-0 zoom-in-[0.985] duration-150 ${rounded} ${
				refused
					? "border-dashed border-destructive"
					: "border-foreground/75 shadow-[0_0_0_5px_color-mix(in_oklab,var(--foreground)_7%,transparent),0_0_48px_-8px_color-mix(in_oklab,var(--primary)_50%,transparent)]"
			}`}
			style={{
				left: bounds.x,
				top: bounds.y,
				width: bounds.width,
				height: bounds.height,
			}}
		>
			<div
				className={`absolute flex max-w-[80%] min-w-max -translate-x-1/2 items-center gap-1.5 truncate rounded-full border border-foreground/10 bg-popover/95 px-3 py-1 text-xs shadow-lg backdrop-blur-xl ${refused ? "text-destructive" : "text-popover-foreground"}`}
				style={{
					left:
						landing && !refused
							? Math.min(
									Math.max(landing.x - bounds.x, 90),
									Math.max(90, bounds.width - 90),
								)
							: bounds.width / 2,
					top:
						landing && !refused
							? landing.y -
								bounds.y +
								(labelAbove ? -LANDING / 2 - 34 : LANDING / 2 + 8)
							: 8,
				}}
			>
				{!refused && <Icon className="size-3 shrink-0" aria-hidden />}
				<span className="truncate">{label}</span>
			</div>
			{landing && !refused && (
				<span
					className="absolute rounded-full border-2 border-dashed border-foreground/70 bg-foreground/10 animate-in fade-in-0 zoom-in-50 duration-200"
					style={{
						left: landing.x - bounds.x - LANDING / 2,
						top: landing.y - bounds.y - LANDING / 2,
						width: LANDING,
						height: LANDING,
					}}
					aria-hidden
				>
					<span className="absolute inset-3 animate-ping rounded-full bg-foreground/40" />
				</span>
			)}
		</div>
	);
}

const HEARTS = [
	{ dx: -22, rise: 46, delay: 0, scale: 0.8 },
	{ dx: 0, rise: 64, delay: 70, scale: 1 },
	{ dx: 20, rise: 50, delay: 140, scale: 0.7 },
] as const;

/** A few hearts float out of the den after a good pull, then fade; `direction` is where they drift. */
export function HeartBurst({
	at,
	direction = { x: 0, y: 1 },
	onDone,
}: Readonly<{ at: GooPoint; direction?: GooPoint; onDone: () => void }>) {
	const nodes = useRef<(HTMLSpanElement | null)[]>([]);
	useEffect(() => {
		const offset = (dx: number, rise: number) => ({
			x: dx * Math.abs(direction.y) + rise * direction.x,
			y: dx * Math.abs(direction.x) + rise * direction.y,
		});
		const animations = HEARTS.map((heart, index) => {
			const half = offset(heart.dx / 2, heart.rise / 2);
			const end = offset(heart.dx, heart.rise);
			return nodes.current[index]?.animate(
				[
					{ opacity: 0, transform: "translate(0, 0) scale(0.4)" },
					{
						opacity: 1,
						transform: `translate(${half.x}px, ${half.y}px) scale(${heart.scale})`,
						offset: 0.35,
					},
					{
						opacity: 0,
						transform: `translate(${end.x}px, ${end.y}px) scale(${heart.scale * 0.9})`,
					},
				],
				{
					duration: 900,
					delay: heart.delay,
					easing: "cubic-bezier(.2,.8,.3,1)",
				},
			);
		});
		const timer = window.setTimeout(onDone, 1_150);
		return () => {
			window.clearTimeout(timer);
			for (const animation of animations) animation?.cancel();
		};
	}, [onDone, direction.x, direction.y]);
	return (
		<div
			className="pointer-events-none absolute"
			style={{ left: at.x, top: at.y }}
			aria-hidden
		>
			{HEARTS.map((heart, index) => (
				<span
					key={heart.dx}
					ref={(node) => {
						nodes.current[index] = node;
					}}
					className="absolute -left-3 -top-3 text-2xl text-primary opacity-0 drop-shadow-[0_1px_6px_var(--primary)]"
				>
					♥
				</span>
			))}
		</div>
	);
}

export interface GooMenuItem {
	label: string;
	onSelect: () => void;
	destructive?: boolean;
	/** A switch: shows its state with a check. */
	checked?: boolean;
}

export function GooMenu({
	nodeRef,
	at,
	items,
	onClose,
}: Readonly<{
	nodeRef: Ref<HTMLDivElement>;
	at: GooPoint;
	items: readonly GooMenuItem[];
	onClose: () => void;
}>) {
	const first = useRef<HTMLButtonElement>(null);
	useEffect(() => first.current?.focus(), []);
	return (
		<div
			ref={nodeRef}
			role="menu"
			className="absolute min-w-52 rounded-xl border border-foreground/10 bg-popover/95 p-1 text-popover-foreground shadow-[0_18px_40px_-12px_rgb(0_0_0/0.65)] backdrop-blur-xl animate-in fade-in-0 zoom-in-95 duration-150"
			style={{ left: at.x, top: at.y }}
			onKeyDown={(event) => {
				if (event.key === "Escape") onClose();
			}}
		>
			{items.map((item, index) => (
				<button
					key={item.label}
					ref={index === 0 ? first : undefined}
					type="button"
					role={item.checked === undefined ? "menuitem" : "menuitemcheckbox"}
					aria-checked={item.checked}
					className={`flex w-full items-center gap-2 rounded-lg px-2.5 py-1.5 text-left text-sm outline-none hover:bg-accent focus-visible:bg-accent ${item.destructive ? "text-destructive" : ""}`}
					onClick={() => {
						onClose();
						item.onSelect();
					}}
				>
					<span className="flex-1">{item.label}</span>
					{item.checked && <Check className="size-3.5 shrink-0" aria-hidden />}
				</button>
			))}
		</div>
	);
}
