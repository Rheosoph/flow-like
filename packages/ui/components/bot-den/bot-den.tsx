"use client";

import { useTranslation } from "@flow-like/locales";
import { useRouter } from "next/navigation";
import {
	type PointerEvent,
	type RefObject,
	useCallback,
	useEffect,
	useMemo,
	useRef,
	useState,
} from "react";
import {
	type DenActivity,
	type DenBot,
	type DenBounds,
	denActivities,
	denCenter,
	denLength,
	inView,
	isArea,
	perchOn,
	perchShare,
	sideDenShape,
} from "../../lib/bot-den/model";
import {
	type DenHit,
	denHitAt,
	denTargetKey,
	denTargetRect,
	focusDenTarget,
	highlightDenTarget,
} from "../../lib/bot-den/targets";
import { DUSK_GOO, INK_GOO } from "../../lib/bot-goo/goo-renderer";
import type {
	BotPlace,
	DenShape,
	GooMotion,
	GooPoint,
	GooRect,
} from "../../lib/bot-goo/scene";
import { PAGE_SNAPSHOT_IGNORE_ATTRIBUTE } from "../../lib/page-snapshot";
import type { IAppPointedAt } from "../../lib/pointed-at";
import { useBotDenRequest } from "../../state/bot-den";
import { botConversationId } from "../../state/global-chat/global-chat-bots";
import { useGlobalChatStore } from "../../state/global-chat/global-chat-store";
import { switchGlobalChatBot } from "../../state/global-chat/global-chat-stream";
import {
	type BotHandlers,
	DropFrame,
	GooBotNode,
	GooMenu,
	type GooMenuItem,
	GooPrompt,
	HeartBurst,
} from "../bot-goo/goo-parts";
import {
	type GooStage,
	type StageModel,
	useGooStage,
} from "../bot-goo/goo-stage";
import { BOTS_ROUTE } from "../bots-tab/dock-selection";
import { pointedAtIcon } from "../interfaces/chat-default/pointed-at-chip";
import {
	type DenLayout,
	useDarkTheme,
	useDenCrew,
	useDenLayout,
} from "./use-den";

const DRAG_START = 5;
const DEN_DROP_MARGIN = 28;
const DEN_HOVER_MARGIN = 22;
/** The bots look at the pointer within this distance of the den. */
const NEAR = 240;
const PULL_FOR_HEARTS = 70;
const PILL_WIDTH = 380;
/** The pill rises off a small target so the target's title stays readable. */
const PILL_LIFT = 18;
/** The flight and the splat get their moment before the prompt stretches out. */
const LANDING_MS = 420;
const LANDING_STEP = 6;
/** A target that stays gone this long sends its bot home. */
const GONE_MS = 700;
/** After its run ends a bot lingers on its target this long. */
const LINGER_MS = 2_400;
/** A sent task gets this long to start a run before its bot gives up waiting. */
const START_GRACE_MS = 9_000;
const LIFECYCLE_TICK_MS = 500;
const GAZE_GRID = 8;
const IDLE: DenActivity = { agentState: "idle" };

interface DenPerch {
	target: IAppPointedAt;
	/** Where on its target it sits, as shares of the target's width and height. */
	share: GooPoint;
	/** When its task went out; null while its prompt is still open. */
	sentAt: number | null;
}

interface Press {
	kind: "bot" | "den";
	key: string | null;
	pointerId: number;
	start: GooPoint;
	dragging: boolean;
}

interface DragView {
	key: string;
	hit: DenHit | null;
	overDen: boolean;
	/** The pointer at the last hit test: where on the target the bot would land. */
	point: GooPoint;
}

type Open =
	| { kind: "prompt"; key: string }
	| { kind: "menu"; key: string | null; at: GooPoint };

function within(rect: GooRect, point: GooPoint, margin = 0): boolean {
	return (
		point.x >= rect.x - margin &&
		point.x <= rect.x + rect.width + margin &&
		point.y >= rect.y - margin &&
		point.y <= rect.y + rect.height + margin
	);
}

function distanceTo(rect: GooRect, point: GooPoint): number {
	const dx = Math.max(rect.x - point.x, 0, point.x - (rect.x + rect.width));
	const dy = Math.max(rect.y - point.y, 0, point.y - (rect.y + rect.height));
	return Math.hypot(dx, dy);
}

function onGrid(point: GooPoint): GooPoint {
	return {
		x: Math.round(point.x / GAZE_GRID) * GAZE_GRID,
		y: Math.round(point.y / GAZE_GRID) * GAZE_GRID,
	};
}

function gazeAt(from: GooPoint, to: GooPoint): GooPoint {
	const clamp = (value: number) => Math.max(-1, Math.min(1, value));
	return {
		x: Math.round(clamp((to.x - from.x) / 140) * 4) / 4,
		y: Math.round(clamp((to.y - from.y) / 60) * 4) / 4,
	};
}

function samePlace(a: BotPlace | undefined, b: BotPlace): boolean {
	if (a?.kind !== "perch" || b.kind !== "perch") return false;
	return (
		a.targetId === b.targetId &&
		a.visible === b.visible &&
		a.edgeY === b.edgeY &&
		Math.abs(a.x - b.x) < 0.25 &&
		Math.abs(a.y - b.y) < 0.25
	);
}

/** A perched bot on its target, wherever the target is now. */
function perchPlace(
	perch: DenPerch,
	rect: GooRect,
	bounds: DenBounds,
): BotPlace {
	const spot = perchOn(rect, perch.share, bounds.top + 4);
	return {
		kind: "perch",
		targetId: denTargetKey(perch.target),
		x: spot.x,
		y: spot.y,
		edgeY: spot.edgeY,
		visible: inView(rect, bounds),
	};
}

/**
 * Follows the perched bots' targets every frame (the board pans and zooms under them) and reports
 * targets that disappeared, so their bots go home.
 */
function usePerchFollowing(
	stage: GooStage,
	perches: ReadonlyMap<string, DenPerch>,
	model: StageModel | null,
	bounds: DenBounds | null,
	onGone: (key: string) => void,
): void {
	const state = useRef({ perches, model, bounds, onGone });
	state.current = { perches, model, bounds, onGone };
	const missing = useRef(new Map<string, number>());

	useEffect(() => {
		stage.beforeFrame = () => {
			const { perches, model, bounds, onGone } = state.current;
			if (!model || !bounds || perches.size === 0) return false;
			const now = performance.now();
			let changed = false;
			const places = new Map(model.places);
			for (const [key, perch] of perches) {
				const rect = denTargetRect(perch.target);
				const previous = places.get(key);
				if (!rect) {
					const since = missing.current.get(key) ?? now;
					missing.current.set(key, since);
					if (now - since > GONE_MS) {
						missing.current.delete(key);
						queueMicrotask(() => onGone(key));
					}
					if (previous?.kind === "perch" && previous.visible) {
						places.set(key, { ...previous, visible: false });
						changed = true;
					}
					continue;
				}
				missing.current.delete(key);
				const place = perchPlace(perch, rect, bounds);
				if (!samePlace(previous, place)) {
					places.set(key, place);
					changed = true;
				}
			}
			if (changed) {
				state.current.model = { ...model, places };
				stage.setModel(state.current.model);
			}
			return true;
		};
		stage.kick();
		return () => {
			stage.beforeFrame = null;
		};
	}, [stage]);
}

/** The den where the builders live, on the right edge of the surface that asked for it. */
export function BotDen() {
	const request = useBotDenRequest();
	return request ? <Den anchor={request.anchor} /> : null;
}

function Den({
	anchor,
}: Readonly<{ anchor: RefObject<HTMLElement | null> | null }>) {
	const { t } = useTranslation("bots");
	const router = useRouter();
	const crew = useDenCrew();
	const layout = useDenLayout(anchor);
	const runs = useGlobalChatStore((state) => state.runs);
	const toolPrompt = useGlobalChatStore((state) => state.toolPrompt);
	const autoMode = useGlobalChatStore((state) => state.autoMode);
	const botAutoMode = useGlobalChatStore((state) => state.botAutoMode);
	const { stage, canvasRef, nodeRef } = useGooStage();
	const root = useRef<HTMLDivElement>(null);
	const dark = useDarkTheme();
	useEffect(() => stage.setLook(dark ? DUSK_GOO : INK_GOO), [stage, dark]);

	const keys = useMemo(() => crew.map((bot) => bot.key), [crew]);
	const byKey = useMemo(
		() => new Map(crew.map((bot) => [bot.key, bot])),
		[crew],
	);
	const activities = useMemo(
		() => denActivities(keys, runs, toolPrompt),
		[keys, runs, toolPrompt],
	);

	const [perches, setPerches] = useState<ReadonlyMap<string, DenPerch>>(
		() => new Map(),
	);
	const [open, setOpen] = useState<Open | null>(null);
	const [drag, setDrag] = useState<DragView | null>(null);
	const [hoverBot, setHoverBot] = useState<string | null>(null);
	const [pointer, setPointer] = useState<GooPoint | null>(null);
	const [hearts, setHearts] = useState(0);
	const dismissHearts = useCallback(() => setHearts(0), []);
	const press = useRef<Press | null>(null);
	const dragView = useRef<DragView | null>(null);
	const landingTimer = useRef<number | undefined>(undefined);
	const busySince = useRef(new Map<string, number>());
	useEffect(() => () => window.clearTimeout(landingTimer.current), []);

	const den = useDenShape(layout, keys, pointer, drag !== null);
	const tucked = den === null;
	const lastDen = useRef<DenShape | null>(null);
	if (den) lastDen.current = den;
	const shape = den ?? tuckedShape(lastDen.current, layout, keys);

	const sendHome = useCallback(
		(key: string) => {
			stage.land(key, null);
			busySince.current.delete(key);
			setPerches((current) => {
				if (!current.has(key)) return current;
				const next = new Map(current);
				next.delete(key);
				return next;
			});
			setOpen((current) =>
				current?.kind === "prompt" && current.key === key ? null : current,
			);
		},
		[stage],
	);

	const promptKey = open?.kind === "prompt" ? open.key : null;
	const model = useMemo<StageModel | null>(() => {
		if (!shape || !layout) return null;
		const places = new Map<string, BotPlace>();
		let lift = 0;
		for (const [key, perch] of perches) {
			const rect = denTargetRect(perch.target);
			if (!rect) continue;
			places.set(key, perchPlace(perch, rect, layout.bounds));
			if (key === promptKey && !isArea(rect)) lift = PILL_LIFT;
		}
		const activity = new Map<string, GooMotion>();
		for (const [key, state] of activities) {
			if (state.agentState === "working") activity.set(key, "working");
			if (state.agentState === "needs-you") activity.set(key, "waiting");
		}
		return {
			den: shape,
			places,
			activity,
			pointer,
			pill: promptKey
				? {
						botId: promptKey,
						width: Math.min(PILL_WIDTH, layout.screenWidth - 16),
						lift,
					}
				: null,
			hoverBot,
			screenWidth: layout.screenWidth,
			tucked,
		};
	}, [
		shape,
		layout,
		perches,
		promptKey,
		hoverBot,
		tucked,
		activities,
		pointer,
	]);

	useEffect(() => {
		if (model) stage.setModel(model);
	}, [stage, model]);
	useEffect(() => stage.kick());
	usePerchFollowing(stage, perches, model, layout?.bounds ?? null, sendHome);

	const activitiesRef = useRef(activities);
	activitiesRef.current = activities;
	const perchesRef = useRef(perches);
	perchesRef.current = perches;
	const anySent = [...perches.values()].some((perch) => perch.sentAt !== null);
	useEffect(() => {
		if (!anySent) return;
		const timer = window.setInterval(() => {
			const now = Date.now();
			for (const [key, perch] of perchesRef.current) {
				if (perch.sentAt === null) continue;
				const state = activitiesRef.current.get(key)?.agentState ?? "idle";
				if (state !== "idle") {
					busySince.current.set(key, now);
					continue;
				}
				const busy = busySince.current.get(key);
				const waited = busy
					? now - busy > LINGER_MS
					: now - perch.sentAt > START_GRACE_MS;
				if (waited) sendHome(key);
			}
		}, LIFECYCLE_TICK_MS);
		return () => window.clearInterval(timer);
	}, [anySent, sendHome]);

	const restRect = den?.rect ?? null;
	useEffect(() => {
		if (!restRect) {
			setPointer(null);
			return;
		}
		const onMove = (event: globalThis.PointerEvent) => {
			const point = { x: event.clientX, y: event.clientY };
			const next = distanceTo(restRect, point) < NEAR ? onGrid(point) : null;
			setPointer((current) =>
				current?.x === next?.x && current?.y === next?.y ? current : next,
			);
		};
		window.addEventListener("pointermove", onMove, { passive: true });
		return () => window.removeEventListener("pointermove", onMove);
	}, [restRect]);

	const cancelPrompt = useCallback(
		(key: string) => {
			const perch = perchesRef.current.get(key);
			setOpen(null);
			if (perch && perch.sentAt === null) sendHome(key);
		},
		[sendHome],
	);

	useEffect(() => {
		if (!open) return;
		const onDown = (event: globalThis.PointerEvent) => {
			if (root.current?.contains(event.target as Node)) return;
			if (open.kind === "prompt") cancelPrompt(open.key);
			else setOpen(null);
		};
		window.addEventListener("pointerdown", onDown, true);
		return () => window.removeEventListener("pointerdown", onDown, true);
	}, [open, cancelPrompt]);

	const isDenElement = useCallback(
		(element: Element) => root.current?.contains(element) ?? false,
		[],
	);

	const openChat = (key: string | null) => {
		const bot = key ? byKey.get(key) : undefined;
		if (bot) void switchGlobalChatBot(bot.chatBotId);
		useGlobalChatStore.getState().openOverlay();
	};

	const autoApproves = (bot: DenBot) =>
		bot.chatBotId === null ? autoMode : botAutoMode[bot.chatBotId] === true;

	const setAutoApprove = (bot: DenBot, enabled: boolean) =>
		useGlobalChatStore
			.getState()
			.setAutoMode(
				enabled,
				bot.chatBotId ? botConversationId(bot.chatBotId) : undefined,
			);

	const submit = (key: string, text: string) => {
		const bot = byKey.get(key);
		const perch = perchesRef.current.get(key);
		setOpen(null);
		if (!bot) return;
		if (perch) focusDenTarget(perch.target);
		const chat = useGlobalChatStore.getState();
		chat.setDraft({
			prompt: text,
			botId: bot.chatBotId,
			...(perch ? { pointedAt: perch.target } : {}),
		});
		chat.openOverlay();
		if (perch)
			setPerches((current) =>
				new Map(current).set(key, { ...perch, sentAt: Date.now() }),
			);
	};

	const trackDrag = (current: Press, key: string, point: GooPoint) => {
		current.dragging = true;
		const overDen = den ? within(den.rect, point, DEN_DROP_MARGIN) : false;
		const hit = overDen ? null : denHitAt(point.x, point.y, isDenElement);
		const view = dragView.current;
		const targetKey = hit ? denTargetKey(hit.target) : null;
		const sameTarget =
			(view?.hit ? denTargetKey(view.hit.target) : null) === targetKey;
		if (!view || !sameTarget) highlightDenTarget(hit?.target ?? null);
		if (
			view &&
			view.overDen === overDen &&
			sameTarget &&
			Math.hypot(view.point.x - point.x, view.point.y - point.y) < LANDING_STEP
		)
			return;
		dragView.current = { key, hit, overDen, point };
		setDrag(dragView.current);
	};

	const drop = (key: string, point: GooPoint, view: DragView | null) => {
		highlightDenTarget(null);
		const overDen = den ? within(den.rect, point, DEN_DROP_MARGIN) : false;
		const hit = overDen
			? null
			: (view?.hit ?? denHitAt(point.x, point.y, isDenElement));
		if (!hit || !layout) {
			sendHome(key);
			return;
		}
		const share = perchShare(hit.rect, point);
		const perch: DenPerch = { target: hit.target, share, sentAt: null };
		stage.land(key, perchPlace(perch, hit.rect, layout.bounds));
		setPerches((current) => new Map(current).set(key, perch));
		busySince.current.delete(key);
		window.clearTimeout(landingTimer.current);
		landingTimer.current = window.setTimeout(() => {
			if (!press.current) setOpen({ kind: "prompt", key });
		}, LANDING_MS);
	};

	const beginPress = (
		event: PointerEvent<HTMLElement>,
		kind: Press["kind"],
		key: string | null,
	) => {
		if (event.button !== 0) return;
		event.preventDefault();
		event.currentTarget.setPointerCapture(event.pointerId);
		press.current = {
			kind,
			key,
			pointerId: event.pointerId,
			start: { x: event.clientX, y: event.clientY },
			dragging: false,
		};
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
		const key = current.key;
		if (!key) return;
		if (
			!current.dragging &&
			Math.hypot(point.x - current.start.x, point.y - current.start.y) <
				DRAG_START
		)
			return;
		if (!current.dragging)
			setOpen((state) =>
				state?.kind === "prompt" && state.key === key ? null : state,
			);
		stage.setDrag({ botId: key, point });
		trackDrag(current, key, point);
	};

	const releasePress = (event: PointerEvent<HTMLElement>): Press | null => {
		const current = press.current;
		if (current?.pointerId !== event.pointerId) return null;
		press.current = null;
		if (event.currentTarget.hasPointerCapture(event.pointerId))
			event.currentTarget.releasePointerCapture(event.pointerId);
		return current;
	};

	const endPress = (event: PointerEvent<HTMLElement>) => {
		const current = releasePress(event);
		if (!current) return;
		const point = { x: event.clientX, y: event.clientY };
		if (current.kind === "den") {
			stage.setPull(null);
			const pulled = current.start.x - point.x;
			if (pulled > PULL_FOR_HEARTS) setHearts((count) => count + 1);
			else if (Math.hypot(pulled, point.y - current.start.y) < DRAG_START)
				openChat(null);
			return;
		}
		const key = current.key;
		if (!key) return;
		if (!current.dragging) {
			if (perchesRef.current.get(key)?.sentAt === null)
				setOpen({ kind: "prompt", key });
			else openChat(key);
			return;
		}
		const view = dragView.current;
		dragView.current = null;
		setDrag(null);
		drop(key, point, view);
	};

	const cancelPress = (event: PointerEvent<HTMLElement>) => {
		const current = releasePress(event);
		if (!current) return;
		stage.setPull(null);
		if (current.key && current.dragging) {
			dragView.current = null;
			setDrag(null);
			highlightDenTarget(null);
			const perch = perchesRef.current.get(current.key);
			const rect = perch ? denTargetRect(perch.target) : null;
			stage.land(
				current.key,
				perch && rect && layout ? perchPlace(perch, rect, layout.bounds) : null,
			);
		}
	};

	const handlersFor = (key: string): BotHandlers => ({
		onPointerDown: (event) => beginPress(event, "bot", key),
		onPointerMove: movePress,
		onPointerUp: endPress,
		onPointerCancel: cancelPress,
		onPointerEnter: () => setHoverBot(key),
		onPointerLeave: () =>
			setHoverBot((current) => (current === key ? null : current)),
		onContextMenu: (event) => {
			event.preventDefault();
			setOpen({
				kind: "menu",
				key,
				at: { x: event.clientX, y: event.clientY },
			});
		},
		onClick: (event) => {
			if (event.detail === 0) openChat(key);
		},
	});

	const menuItems = (key: string | null): GooMenuItem[] => {
		const bot = key ? byKey.get(key) : undefined;
		if (!bot)
			return [
				{
					label: t("notch.openChat", "Open chat"),
					onSelect: () => openChat(null),
				},
				{
					label: t("notch.openBots", "Open Bots"),
					onSelect: () => router.push(BOTS_ROUTE),
				},
			];
		const items: GooMenuItem[] = [
			{
				label: t("notch.openChat", "Open chat"),
				onSelect: () => openChat(bot.key),
			},
			{
				label: t("notch.autoApprove", "Approve automatically"),
				checked: autoApproves(bot),
				onSelect: () => setAutoApprove(bot, !autoApproves(bot)),
			},
		];
		if (perches.has(bot.key))
			items.push({
				label: t("notch.sendHome", "Send home"),
				onSelect: () => sendHome(bot.key),
			});
		return items;
	};

	if (!layout || !shape) return null;

	const promptBot = promptKey ? byKey.get(promptKey) : undefined;
	const promptTarget = promptKey ? perches.get(promptKey)?.target : undefined;
	const dropHit = drag?.hit && !drag.overDen ? drag.hit : null;
	const slots = new Map(shape.slots.map((slot) => [slot.botId, slot]));

	return (
		<div
			ref={root}
			className="pointer-events-none fixed inset-0 z-9997 select-none overflow-hidden text-foreground"
			{...{ [PAGE_SNAPSHOT_IGNORE_ATTRIBUTE]: "" }}
		>
			<canvas
				ref={canvasRef}
				className="pointer-events-none absolute left-0 top-0"
				aria-hidden
			/>
			<div
				ref={nodeRef("den")}
				className={`absolute left-0 top-0 touch-none ${tucked ? "pointer-events-none" : "pointer-events-auto cursor-grab active:cursor-grabbing"}`}
				style={{ willChange: "transform" }}
				title={t(
					"den.hint",
					"Drag a bot onto a node, a page or anything in this app to ask about it",
				)}
				onPointerDown={(event) => beginPress(event, "den", null)}
				onPointerMove={movePress}
				onPointerUp={endPress}
				onPointerCancel={cancelPress}
				onContextMenu={(event) => {
					event.preventDefault();
					setOpen({
						kind: "menu",
						key: null,
						at: { x: event.clientX, y: event.clientY },
					});
				}}
				aria-hidden
			/>
			{shape.slots.map((slot) =>
				!tucked && (perches.has(slot.botId) || drag?.key === slot.botId) ? (
					<span
						key={`socket:${slot.botId}`}
						className="pointer-events-none absolute size-2 rounded-full border border-white/45"
						style={{ left: slot.x - 4, top: slot.y - 4 }}
						aria-hidden
					/>
				) : null,
			)}
			{crew.map((bot) => {
				const slot = slots.get(bot.key);
				return (
					<GooBotNode
						key={bot.key}
						name={bot.name}
						appearance={bot.appearance}
						autoApprove={autoApproves(bot)}
						activity={activities.get(bot.key) ?? IDLE}
						gaze={
							pointer && slot && !perches.has(bot.key)
								? gazeAt(slot, pointer)
								: null
						}
						nodeRef={nodeRef(`bot:${bot.key}`)}
						handlers={handlersFor(bot.key)}
					/>
				);
			})}
			{promptKey && promptBot && (
				<div className="dark contents">
					<GooPrompt
						key={promptKey}
						nodeRef={nodeRef(`pill:${promptKey}`)}
						botName={promptBot.name}
						target={promptTarget?.label ?? null}
						onSubmit={(text) => submit(promptKey, text)}
						onCancel={() => cancelPrompt(promptKey)}
					/>
				</div>
			)}
			{drag && dropHit && (
				<DropFrame
					bounds={dropHit.rect}
					label={dropHit.target.label}
					icon={pointedAtIcon(dropHit.target)}
					refused={false}
					landing={(() => {
						const spot = perchOn(
							dropHit.rect,
							perchShare(dropHit.rect, drag.point),
							layout.bounds.top + 4,
						);
						return { x: spot.x, y: spot.y };
					})()}
					labelAbove={!isArea(dropHit.rect)}
					rounded="rounded-lg"
				/>
			)}
			{open?.kind === "menu" && (
				<GooMenu
					nodeRef={nodeRef("overlay:menu")}
					at={{
						x: Math.min(open.at.x, layout.screenWidth - 216),
						y: open.at.y + 6,
					}}
					items={menuItems(open.key)}
					onClose={() => setOpen(null)}
				/>
			)}
			{hearts > 0 && den && (
				<HeartBurst
					key={hearts}
					at={{ x: den.rect.x - 4, y: den.rect.y + den.rect.height / 2 }}
					direction={{ x: -1, y: 0 }}
					onDone={dismissHearts}
				/>
			)}
		</div>
	);
}

/** The den's shape where it has room; null while something covers its whole edge. */
function useDenShape(
	layout: DenLayout | null,
	keys: readonly string[],
	pointer: GooPoint | null,
	dragging: boolean,
): DenShape | null {
	const center = layout
		? denCenter(layout.bounds, denLength(keys.length, false), layout.avoid)
		: null;
	const right = layout?.bounds.right ?? 0;
	const rest =
		center === null ? null : sideDenShape(right, center, keys, false);
	const hover = Boolean(
		pointer && rest && within(rest.rect, pointer, DEN_HOVER_MARGIN),
	);
	return useMemo(
		() =>
			center === null
				? null
				: sideDenShape(right, center, keys, hover || dragging),
		[right, center, keys, hover, dragging],
	);
}

/** Where a tucked den shrinks to: its last place, flat against the edge. */
function tuckedShape(
	last: DenShape | null,
	layout: DenLayout | null,
	keys: readonly string[],
): DenShape | null {
	if (!layout) return null;
	const base =
		last ??
		sideDenShape(
			layout.bounds.right,
			(layout.bounds.top + layout.bounds.bottom) / 2,
			keys,
			false,
		);
	const center = base.rect.y + base.rect.height / 2;
	return {
		...base,
		rect: { x: layout.bounds.right, y: center, width: 0, height: 0 },
		slots: base.slots.map((slot) => ({ ...slot, x: layout.bounds.right })),
	};
}
