"use client";

import {
	type BotHandlers,
	DropFrame,
	GooBotNode,
	GooMenu,
	type GooMenuItem,
	GooPrompt,
	HeartBurst,
} from "@flow-like/flow-like-ui/components/bot-goo/goo-parts";
import {
	type HitboxSink,
	useGooStage,
} from "@flow-like/flow-like-ui/components/bot-goo/goo-stage";
import {
	type BotPlace,
	type GooMotion,
	PILL_HEIGHT,
} from "@flow-like/flow-like-ui/lib/bot-goo/scene";
import { useTranslation } from "@flow-like/locales";
import {
	type PointerEvent,
	useCallback,
	useEffect,
	useMemo,
	useRef,
	useState,
} from "react";
import { notchCommands } from "../../lib/bot-notch/commands";
import {
	type BotActivity,
	acceptsTarget,
	botActivity,
	contains,
	denShape,
	perchAnchor,
	perchCenter,
	pointedAtOf,
	shownInNotch,
	targetLabel,
} from "../../lib/bot-notch/model";
import {
	NOTCH_EVENTS,
	type NotchPoint,
	type NotchPointer,
	type NotchWindow,
} from "../../lib/bot-notch/protocol";
import { NotchCardView } from "./notch-parts";
import {
	useHostId,
	useHostTurns,
	useNotchBridge,
	useNotchLayout,
	useNotchSettings,
	usePerchFrames,
	useTauriEvent,
} from "./use-notch-data";
import { useNotchTasks } from "./use-notch-tasks";
import { usePerches } from "./use-perches";

const DRAG_START = 5;
/** How close the pointer must come before the notch starts to notice it. */
const PROXIMITY = 56;
const HOVER_QUERY_MS = 70;
const DEN_DROP_MARGIN = 28;
const DEN_HOVER_MARGIN = 22;
const PULL_FOR_HEARTS = 70;
const PILL_WIDTH = 330;
const PILL_WIDTH_TARGET = 420;
const TAB_WAIT_MS = 3_000;
/** The flight to the edge and the splat get their moment before the prompt stretches out. */
const LANDING_MS = 420;
const SETTLE_TICK_MS = 1_000;
const GAZE_GRID = 8;
const IDLE: BotActivity = { agentState: "idle", turn: null };

interface Press {
	kind: "bot" | "den";
	botId: string | null;
	pointerId: number;
	start: NotchPoint;
	dragging: boolean;
	lastQuery: number;
	hover: NotchWindow | null;
}

type Open =
	| { kind: "prompt"; botId: string }
	| { kind: "menu"; botId: string | null; at: NotchPoint };

interface DragView {
	botId: string;
	hover: NotchWindow | null;
	overDen: boolean;
	/** The pointer's x at the last window check: where along the edge the bot would land. */
	x: number;
}

const LANDING_STEP = 6;

function withTimeout<T>(promise: Promise<T>, ms: number): Promise<T | null> {
	return Promise.race([
		promise,
		new Promise<null>((resolve) => window.setTimeout(() => resolve(null), ms)),
	]);
}

function onGrid(point: NotchPoint): NotchPoint {
	return {
		x: Math.round(point.x / GAZE_GRID) * GAZE_GRID,
		y: Math.round(point.y / GAZE_GRID) * GAZE_GRID,
	};
}

function gazeAt(from: NotchPoint, to: NotchPoint): NotchPoint {
	const clamp = (value: number) => Math.max(-1, Math.min(1, value));
	return {
		x: Math.round(clamp((to.x - from.x) / 140) * 4) / 4,
		y: Math.round(clamp((to.y - from.y) / 60) * 4) / 4,
	};
}

const sendHitboxes: HitboxSink = (rects, capture) =>
	void notchCommands
		.setHitboxes(rects, capture, PROXIMITY)
		.catch((error) =>
			console.warn("[BotNotch] The notch's hitboxes were refused:", error),
		);

function settling(turn: { finished_at?: string | null }, now: number): boolean {
	if (!turn.finished_at) return false;
	const finished = Date.parse(turn.finished_at);
	return Number.isFinite(finished) && now - finished < 9_000;
}

/** The den at the screen's top edge where the desktop bots live, and everything they do outside it. */
export function BotNotch() {
	const { t } = useTranslation("bots");
	const layout = useNotchLayout();
	const settings = useNotchSettings();
	const hostId = useHostId();
	const { snapshot, call } = useNotchBridge();
	const turns = useHostTurns();
	const frames = usePerchFrames();
	const { stage, canvasRef, nodeRef } = useGooStage(sendHitboxes);

	const hiddenBots = settings?.hiddenBots;
	const bots = useMemo(
		() => shownInNotch(snapshot.bots, hiddenBots ?? []),
		[snapshot.bots, hiddenBots],
	);
	const botIds = useMemo(() => bots.map((bot) => bot.id), [bots]);
	const shown = useMemo(() => new Set(botIds), [botIds]);
	const byId = useMemo(() => new Map(bots.map((bot) => [bot.id, bot])), [bots]);

	const { perches, perch, unperch } = usePerches(shown, frames);
	const messages = useMemo(
		() => ({
			sendFailed: (error: string) =>
				t("notch.sendFailed", "The task didn't reach the bot: {{error}}", {
					error,
				}),
			failed: t(
				"notch.failed",
				"The bot couldn't finish. Open the chat to see why.",
			),
		}),
		[t],
	);
	const { tasks, cards, send, answer, dismiss } = useNotchTasks(
		call,
		turns,
		shown,
		messages,
	);

	const [open, setOpen] = useState<Open | null>(null);
	const [hoverBot, setHoverBot] = useState<string | null>(null);
	const [pointer, setPointer] = useState<NotchPoint | null>(null);
	const [drag, setDrag] = useState<DragView | null>(null);
	const [hearts, setHearts] = useState(0);
	const dismissHearts = useCallback(() => setHearts(0), []);
	const [now, setNow] = useState(() => Date.now());
	const press = useRef<Press | null>(null);
	const landingTimer = useRef<number | undefined>(undefined);
	useEffect(() => () => window.clearTimeout(landingTimer.current), []);

	const visible = Boolean(settings?.enabled && layout && bots.length > 0);
	useEffect(() => {
		void notchCommands
			.setVisible(visible)
			.catch((error) =>
				console.warn("[BotNotch] The notch could not be shown:", error),
			);
	}, [visible]);

	const anySettling = useMemo(
		() => [...turns.values()].some((turn) => settling(turn, Date.now())),
		[turns],
	);
	useEffect(() => {
		if (!anySettling) return;
		const timer = window.setInterval(() => setNow(Date.now()), SETTLE_TICK_MS);
		return () => window.clearInterval(timer);
	}, [anySettling]);

	const dragging = drag !== null;
	const denHover = Boolean(
		pointer &&
			layout &&
			contains(denShape(layout, botIds, false).rect, pointer, DEN_HOVER_MARGIN),
	);
	const den = useMemo(
		() => (layout ? denShape(layout, botIds, denHover || dragging) : null),
		[layout, botIds, denHover, dragging],
	);

	const places = useMemo(() => {
		const map = new Map<string, BotPlace>();
		if (!layout || !den) return map;
		const topLimit = layout.menuBarHeight + 2;
		for (const [botId, seat] of perches) {
			const tracked = frames.get(botId);
			const frame = tracked?.windowId === seat.window.id ? tracked : undefined;
			const bounds = frame?.bounds ?? seat.window.bounds;
			const center = perchCenter(bounds, seat.anchorX, topLimit);
			map.set(botId, {
				kind: "perch",
				targetId: seat.window.id,
				...center,
				edgeY: center.y < bounds.y ? bounds.y : null,
				visible: frame ? frame.visible && frame.tabActive : true,
			});
		}
		if (open?.kind === "prompt" && !map.has(open.botId)) {
			const slot = den.slots.find(
				(candidate) => candidate.botId === open.botId,
			);
			if (slot)
				map.set(open.botId, {
					kind: "drop",
					x: slot.x,
					y: den.rect.height + PILL_HEIGHT / 2 + 14,
				});
		}
		return map;
	}, [layout, den, perches, frames, open]);

	const promptBot = open?.kind === "prompt" ? open.botId : null;
	const pillWidth = Math.min(
		promptBot && perches.has(promptBot) ? PILL_WIDTH_TARGET : PILL_WIDTH,
		(layout?.width ?? PILL_WIDTH + 16) - 16,
	);

	useEffect(() => stage.kick());

	useEffect(() => {
		const close = () => setOpen(null);
		window.addEventListener("blur", close);
		return () => window.removeEventListener("blur", close);
	}, []);

	useTauriEvent<NotchPointer>(NOTCH_EVENTS.pointer, (point) => {
		if (!point.near) {
			setPointer(null);
			return;
		}
		const next = onGrid(point);
		setPointer((current) =>
			current?.x === next.x && current.y === next.y ? current : next,
		);
	});

	const activities = useMemo(
		() =>
			new Map(
				bots.map((bot) => [bot.id, botActivity(turns.values(), bot.id, now)]),
			),
		[bots, turns, now],
	);
	const motions = useMemo(() => {
		const map = new Map<string, GooMotion>();
		for (const [botId, activity] of activities) {
			if (activity.agentState === "needs-you") map.set(botId, "waiting");
			else if (activity.agentState === "working") map.set(botId, "working");
			else if (activity.mood === "thinking") map.set(botId, "thinking");
		}
		return map;
	}, [activities]);

	useEffect(() => {
		if (!den || !layout) return;
		stage.setModel({
			den,
			places,
			pill: promptBot ? { botId: promptBot, width: pillWidth } : null,
			hoverBot,
			screenWidth: layout.width,
			activity: motions,
			pointer,
		});
	}, [
		stage,
		den,
		layout,
		places,
		promptBot,
		pillWidth,
		hoverBot,
		motions,
		pointer,
	]);

	const shake = (botId: string) =>
		stage
			.node(`bot:${botId}`)
			?.animate(
				[
					{ translate: "0 0" },
					{ translate: "-6px 0" },
					{ translate: "6px 0" },
					{ translate: "-4px 0" },
					{ translate: "0 0" },
				],
				{ duration: 360, easing: "ease-out" },
			);

	const landingFor = (window: NotchWindow, botId: string, dropX: number) => {
		const taken = [...perches.values()]
			.filter((seat) => seat.window.id === window.id && seat.botId !== botId)
			.map((seat) => seat.anchorX);
		const anchorX = perchAnchor(window.bounds, dropX, taken);
		const center = perchCenter(
			window.bounds,
			anchorX,
			(layout?.menuBarHeight ?? 0) + 2,
		);
		const place: BotPlace = {
			kind: "perch",
			targetId: window.id,
			...center,
			edgeY: center.y < window.bounds.y ? window.bounds.y : null,
			visible: true,
		};
		return place;
	};

	const dropBot = async (
		botId: string,
		point: NotchPoint,
		hover: NotchWindow | null,
	) => {
		const bot = byId.get(botId);
		const overDen = den ? contains(den.rect, point, DEN_DROP_MARGIN) : false;
		const target =
			overDen || !bot
				? null
				: hover && contains(hover.bounds, point)
					? hover
					: await notchCommands.windowAt(point.x, point.y).catch(() => null);
		if (!bot || !target) {
			stage.land(botId, null);
			unperch(botId);
			return;
		}
		if (!acceptsTarget(bot, target)) {
			stage.land(botId, places.get(botId) ?? null);
			shake(botId);
			return;
		}
		stage.land(botId, landingFor(target, botId, point.x));
		perch(botId, target, point.x);
		window.clearTimeout(landingTimer.current);
		landingTimer.current = window.setTimeout(() => {
			if (!press.current) setOpen({ kind: "prompt", botId });
		}, LANDING_MS);
	};

	const trackHover = (current: Press, point: NotchPoint) => {
		const overDen = den ? contains(den.rect, point, DEN_DROP_MARGIN) : false;
		setDrag((view) =>
			view && view.overDen !== overDen ? { ...view, overDen } : view,
		);
		if (overDen) return;
		const stamp = performance.now();
		if (stamp - current.lastQuery < HOVER_QUERY_MS) return;
		current.lastQuery = stamp;
		notchCommands
			.windowAt(point.x, point.y)
			.then((window) => {
				if (press.current !== current) return;
				current.hover = window;
				setDrag((view) =>
					view &&
					(view.hover?.id !== window?.id ||
						view.hover?.title !== window?.title ||
						Math.abs(view.x - point.x) >= LANDING_STEP)
						? { ...view, hover: window, x: point.x }
						: view,
				);
			})
			.catch(() => undefined);
	};

	const beginPress = (
		event: PointerEvent<HTMLElement>,
		kind: Press["kind"],
		botId: string | null,
	) => {
		if (event.button !== 0) return;
		event.preventDefault();
		event.currentTarget.setPointerCapture(event.pointerId);
		press.current = {
			kind,
			botId,
			pointerId: event.pointerId,
			start: { x: event.clientX, y: event.clientY },
			dragging: false,
			lastQuery: 0,
			hover: null,
		};
		stage.setCapture(true);
	};

	const movePress = (event: PointerEvent<HTMLElement>) => {
		const current = press.current;
		if (current?.pointerId !== event.pointerId) return;
		const point = { x: event.clientX, y: event.clientY };
		if (current.kind === "den") {
			stage.setPull({
				x: point.x - current.start.x,
				y: point.y - current.start.y,
			});
			return;
		}
		const botId = current.botId;
		if (!botId) return;
		if (!current.dragging) {
			const moved = Math.hypot(
				point.x - current.start.x,
				point.y - current.start.y,
			);
			if (moved < DRAG_START) return;
			current.dragging = true;
			setOpen(null);
			setDrag({ botId, hover: null, overDen: false, x: point.x });
		}
		stage.setDrag({ botId, point });
		trackHover(current, point);
	};

	const releasePress = (event: PointerEvent<HTMLElement>): Press | null => {
		const current = press.current;
		if (current?.pointerId !== event.pointerId) return null;
		press.current = null;
		if (event.currentTarget.hasPointerCapture(event.pointerId))
			event.currentTarget.releasePointerCapture(event.pointerId);
		stage.setCapture(false);
		return current;
	};

	const endPress = (event: PointerEvent<HTMLElement>) => {
		const current = releasePress(event);
		if (!current) return;
		const point = { x: event.clientX, y: event.clientY };
		if (current.kind === "den") {
			stage.setPull(null);
			const pulled = point.y - current.start.y;
			if (pulled > PULL_FOR_HEARTS) setHearts((count) => count + 1);
			else if (Math.hypot(point.x - current.start.x, pulled) < DRAG_START)
				setOpen({ kind: "menu", botId: null, at: point });
			return;
		}
		const botId = current.botId;
		if (!botId) return;
		if (!current.dragging) {
			setOpen((state) =>
				state?.kind === "prompt" && state.botId === botId
					? null
					: { kind: "prompt", botId },
			);
			return;
		}
		setDrag(null);
		void dropBot(botId, point, current.hover);
	};

	const cancelPress = (event: PointerEvent<HTMLElement>) => {
		const current = releasePress(event);
		if (!current) return;
		stage.setPull(null);
		if (current.botId && current.dragging) {
			setDrag(null);
			stage.land(current.botId, places.get(current.botId) ?? null);
		}
	};

	const handlersFor = (botId: string): BotHandlers => ({
		onPointerDown: (event) => beginPress(event, "bot", botId),
		onPointerMove: movePress,
		onPointerUp: endPress,
		onPointerCancel: cancelPress,
		onPointerEnter: () => setHoverBot(botId),
		onPointerLeave: () =>
			setHoverBot((current) => (current === botId ? null : current)),
		onContextMenu: (event) => {
			event.preventDefault();
			setOpen({
				kind: "menu",
				botId,
				at: { x: event.clientX, y: event.clientY },
			});
		},
		onClick: (event) => {
			if (event.detail === 0) setOpen({ kind: "prompt", botId });
		},
	});

	const submit = async (botId: string, text: string) => {
		const seat = perches.get(botId);
		setOpen(null);
		let pointedAt = null;
		if (seat && hostId) {
			const tab =
				seat.tab !== undefined
					? seat.tab
					: await withTimeout(
							notchCommands.activeTab(seat.window.id),
							TAB_WAIT_MS,
						).catch(() => null);
			pointedAt = pointedAtOf(seat.window, tab ?? null, hostId);
		}
		if (seat)
			void notchCommands.focusWindow(seat.window.id).catch(() => undefined);
		await send(botId, text, pointedAt);
	};

	const openChat = (botId: string | null) =>
		void call({
			kind: "open",
			botId,
			conversationId: botId ? (tasks.get(botId)?.conversationId ?? null) : null,
		}).catch((error) =>
			console.warn("[BotNotch] Could not open the chat:", error),
		);

	const setAutoApprove = (botId: string, enabled: boolean) =>
		call({ kind: "autoApprove", botId, enabled }).catch((error) =>
			console.warn("[BotNotch] Auto approval did not switch:", error),
		);

	/** "Don't ask again": the bot approves on its own from now on, starting with this card. */
	const approveFromNowOn = async (botId: string) => {
		await setAutoApprove(botId, true);
		await answer(botId, "once");
	};

	const menuItems = (botId: string | null): GooMenuItem[] => {
		if (!botId)
			return [
				{
					label: t("notch.openBots", "Open Bots"),
					onSelect: () => openChat(null),
				},
				{
					label: t("notch.hideNotch", "Hide the notch"),
					onSelect: () => void notchCommands.setEnabled(false),
				},
			];
		const bot = byId.get(botId);
		const items: GooMenuItem[] = [
			{
				label: t("notch.openChat", "Open chat"),
				onSelect: () => openChat(botId),
			},
		];
		if (bot)
			items.push({
				label: t("notch.autoApprove", "Approve automatically"),
				checked: bot.autoApprove,
				onSelect: () => setAutoApprove(botId, !bot.autoApprove),
			});
		if (perches.has(botId))
			items.push({
				label: t("notch.sendHome", "Send home"),
				onSelect: () => {
					stage.land(botId, null);
					unperch(botId);
				},
			});
		const turn = activities.get(botId)?.turn;
		if (
			turn &&
			(turn.status === "running" ||
				turn.status === "queued" ||
				turn.status === "waiting")
		)
			items.push({
				label: t("notch.stop", "Stop"),
				destructive: true,
				onSelect: () =>
					void call({ kind: "stop", turn }).catch((error) =>
						console.warn("[BotNotch] The turn did not stop:", error),
					),
			});
		if (bot)
			items.push({
				label: t("notch.hide", "Hide from the notch"),
				onSelect: () =>
					void notchCommands.setHiddenBots([
						...(settings?.hiddenBots ?? []),
						bot.key,
					]),
			});
		return items;
	};

	const statusOf = (botId: string): string | null => {
		if (hoverBot !== botId) return null;
		const activity = activities.get(botId);
		const turn = activity?.turn;
		const seat = perches.get(botId);
		if (turn?.status === "running")
			return turn.status_text?.trim() || t("notch.working", "Working…");
		if (turn?.status === "queued") return t("notch.thinking", "Thinking…");
		if (seat) return targetLabel(seat.window, seat.tab ?? null);
		return byId.get(botId)?.name ?? null;
	};

	const dropLabel = (view: DragView): string | null => {
		if (!view.hover) return null;
		const bot = byId.get(view.botId);
		if (bot && !acceptsTarget(bot, view.hover))
			return t("notch.browserOnly", "{{name}} can only work in a browser", {
				name: bot.name,
			});
		return targetLabel(view.hover, null);
	};

	if (!layout || !den)
		return <canvas ref={canvasRef} className="hidden" aria-hidden />;

	const promptTarget = promptBot ? perches.get(promptBot) : undefined;
	const dragBot = drag ? byId.get(drag.botId) : undefined;

	return (
		<div className="dark fixed inset-0 select-none overflow-hidden text-foreground">
			<canvas
				ref={canvasRef}
				className="pointer-events-none absolute left-0 top-0"
				aria-hidden
			/>
			<div
				ref={nodeRef("den")}
				className="absolute left-0 top-0 cursor-grab touch-none active:cursor-grabbing"
				style={{ willChange: "transform" }}
				onPointerDown={(event) => beginPress(event, "den", null)}
				onPointerMove={movePress}
				onPointerUp={endPress}
				onPointerCancel={cancelPress}
				onContextMenu={(event) => {
					event.preventDefault();
					setOpen({
						kind: "menu",
						botId: null,
						at: { x: event.clientX, y: event.clientY },
					});
				}}
				aria-hidden
			/>
			{den.slots.map((slot) =>
				perches.has(slot.botId) && drag?.botId !== slot.botId ? (
					<span
						key={`socket:${slot.botId}`}
						className="pointer-events-none absolute size-2 rounded-full border border-muted-foreground/60"
						style={{ left: slot.x - 4, top: slot.y - 4 }}
						aria-hidden
					/>
				) : null,
			)}
			{bots.map((bot) => (
				<GooBotNode
					key={bot.id}
					name={bot.name}
					appearance={bot.appearance}
					autoApprove={bot.autoApprove}
					activity={activities.get(bot.id) ?? IDLE}
					gaze={
						pointer && !perches.has(bot.id)
							? gazeAt(
									den.slots.find((slot) => slot.botId === bot.id) ?? pointer,
									pointer,
								)
							: null
					}
					nodeRef={nodeRef(`bot:${bot.id}`)}
					handlers={handlersFor(bot.id)}
				/>
			))}
			{promptBot && byId.has(promptBot) && (
				<GooPrompt
					key={promptBot}
					nodeRef={nodeRef(`pill:${promptBot}`)}
					botName={byId.get(promptBot)?.name ?? ""}
					target={
						promptTarget
							? targetLabel(promptTarget.window, promptTarget.tab ?? null)
							: null
					}
					onSubmit={(text) => void submit(promptBot, text)}
					onCancel={() => setOpen(null)}
				/>
			)}
			{bots.map((bot) => {
				if (drag?.botId === bot.id) return null;
				const card = cards.get(bot.id) ?? null;
				const status = card ? null : statusOf(bot.id);
				if (!card && !status) return null;
				return (
					<NotchCardView
						key={`card:${bot.id}`}
						nodeRef={nodeRef(`card:${bot.id}`)}
						bot={bot}
						card={card}
						status={status}
						onAnswer={(decision) => void answer(bot.id, decision)}
						onAutoApprove={() => void approveFromNowOn(bot.id)}
						onDismiss={() => dismiss(bot.id)}
						onOpen={() => {
							dismiss(bot.id);
							openChat(bot.id);
						}}
					/>
				);
			})}
			{drag?.hover && !drag.overDen && (
				<DropFrame
					bounds={drag.hover.bounds}
					label={dropLabel(drag) ?? ""}
					refused={Boolean(dragBot && !acceptsTarget(dragBot, drag.hover))}
					landing={landingFor(drag.hover, drag.botId, drag.x)}
				/>
			)}
			{open?.kind === "menu" && (
				<GooMenu
					nodeRef={nodeRef("overlay:menu")}
					at={{
						x: Math.min(open.at.x, layout.width - 200),
						y: open.at.y + 6,
					}}
					items={menuItems(open.botId)}
					onClose={() => setOpen(null)}
				/>
			)}
			{hearts > 0 && (
				<HeartBurst
					key={hearts}
					at={{ x: den.rect.x + den.rect.width / 2, y: den.rect.height + 4 }}
					onDone={dismissHearts}
				/>
			)}
		</div>
	);
}
