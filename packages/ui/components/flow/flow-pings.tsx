"use client";
import { useTranslation } from "@flow-like/locales";
import { useStore } from "@xyflow/react";
import { LocateFixedIcon } from "lucide-react";
import {
	memo,
	useEffect,
	useMemo,
	useState,
	useSyncExternalStore,
} from "react";
import { useShallow } from "zustand/react/shallow";
import {
	type PeerUserInfo,
	colorFromSub,
	truncateName,
} from "../../hooks/use-peer-users";
import {
	PING_TTL_MS,
	type PingEmoji,
} from "../../lib/realtime/presence-signals";
import type {
	PeerPing,
	SignalStore,
} from "../../lib/realtime/presence-signals-store";
import { Avatar, AvatarFallback, AvatarImage } from "../ui/avatar";
import { calculateEdgePosition } from "./flow-cursors";

/** A ping fades out over its last half second. */
const FADE_MS = 500;
/** Age is re-sampled this often while any ping is live. */
const CLOCK_TICK_MS = 100;
const OFFSCREEN_MARGIN = 80;

interface PingDisplay {
	key: string;
	color: string;
	name: string;
	initial: string;
	avatarUrl?: string;
	emoji?: PingEmoji;
	opacity: number;
	x: number;
	y: number;
	offScreen: boolean;
	edgeX?: number;
	edgeY?: number;
}

/**
 * Ripples where peers (and the local user) pinged on the current layer, plus an
 * edge pill for pings on this layer that sit outside the visible area. Only
 * mounted while a ping is live, so its clock interval costs nothing otherwise.
 */
export function FlowPingsLayer({
	store,
	currentLayerPath,
	peerUsers,
	sub,
}: {
	store: SignalStore<PeerPing[]>;
	currentLayerPath: string;
	peerUsers: Map<string, PeerUserInfo>;
	sub?: string;
}) {
	const pings = useSyncExternalStore(
		store.subscribe,
		store.getSnapshot,
		store.getSnapshot,
	);
	if (pings.length === 0) return null;
	return (
		<FlowPings
			pings={pings}
			currentLayerPath={currentLayerPath}
			peerUsers={peerUsers}
			sub={sub}
		/>
	);
}

function useClockTick(intervalMs: number): number {
	const [now, setNow] = useState(() => Date.now());
	useEffect(() => {
		const id = setInterval(() => setNow(Date.now()), intervalMs);
		return () => clearInterval(id);
	}, [intervalMs]);
	return now;
}

const FlowPings = memo(function FlowPings({
	pings,
	currentLayerPath,
	peerUsers,
	sub,
}: {
	pings: PeerPing[];
	currentLayerPath: string;
	peerUsers: Map<string, PeerUserInfo>;
	sub?: string;
}) {
	const { t } = useTranslation("flow");
	const { transform, width, height } = useStore(
		useShallow((state) => ({
			transform: state.transform,
			width: state.width,
			height: state.height,
		})),
	);
	const [tx, ty, zoom] = transform;
	const now = useClockTick(CLOCK_TICK_MS);

	const items = useMemo(() => {
		if (typeof window === "undefined") return [];
		const viewportWidth = width || window.innerWidth;
		const viewportHeight = height || window.innerHeight;
		return pings
			.filter((ping) => ping.layerPath === currentLayerPath)
			.map((ping): PingDisplay => {
				const self = Boolean(sub && ping.sub === sub);
				const info = ping.sub ? peerUsers.get(ping.sub) : undefined;
				const peerName =
					info?.truncatedName ?? truncateName(ping.sub?.slice(-8));
				const remaining = PING_TTL_MS - (now - ping.seenAt);
				const screenX = ping.x * zoom + tx;
				const screenY = ping.y * zoom + ty;
				const offScreen =
					screenX < OFFSCREEN_MARGIN ||
					screenX > viewportWidth - OFFSCREEN_MARGIN ||
					screenY < OFFSCREEN_MARGIN ||
					screenY > viewportHeight - OFFSCREEN_MARGIN;
				const edge = offScreen
					? calculateEdgePosition(
							screenX,
							screenY,
							viewportWidth,
							viewportHeight,
							OFFSCREEN_MARGIN,
						)
					: undefined;
				return {
					key: ping.key,
					color: info?.color ?? colorFromSub(ping.sub),
					name: self ? t("you", "You") : peerName,
					initial: peerName.charAt(0).toUpperCase(),
					avatarUrl: info?.avatarUrl,
					emoji: ping.emoji,
					opacity: remaining >= FADE_MS ? 1 : Math.max(0, remaining / FADE_MS),
					x: screenX,
					y: screenY,
					offScreen,
					edgeX: edge?.edgeX,
					edgeY: edge?.edgeY,
				};
			});
	}, [
		pings,
		currentLayerPath,
		peerUsers,
		sub,
		t,
		now,
		tx,
		ty,
		zoom,
		width,
		height,
	]);

	if (items.length === 0) return null;

	return (
		<div className="pointer-events-none absolute inset-0 z-30">
			{items.map((item) =>
				item.offScreen ? (
					<PingEdgeIndicator key={item.key} ping={item} />
				) : (
					<PingRipple key={item.key} ping={item} />
				),
			)}
		</div>
	);
});

const PingRipple = memo(function PingRipple({ ping }: { ping: PingDisplay }) {
	const { t } = useTranslation("flow");
	return (
		<div
			className="absolute left-0 top-0 transition-opacity duration-100 ease-linear will-change-transform motion-reduce:transition-none"
			style={{
				transform: `translate3d(${ping.x}px, ${ping.y}px, 0)`,
				opacity: ping.opacity,
			}}
		>
			<span className="sr-only">
				{t("presencePingFrom", "{{name}} pinged here", { name: ping.name })}
			</span>
			<span
				className="absolute -left-6 -top-6 h-12 w-12 rounded-full border-2 animate-ping motion-reduce:animate-none"
				style={{ borderColor: ping.color, opacity: 0.55 }}
			/>
			<span
				className="absolute -left-4 -top-4 h-8 w-8 rounded-full border-2 animate-ping [animation-delay:250ms] motion-reduce:animate-none"
				style={{ borderColor: ping.color }}
			/>
			<span
				className="absolute -left-1.5 -top-1.5 h-3 w-3 rounded-full ring-2 ring-white/80"
				style={{ backgroundColor: ping.color }}
			/>
			{ping.emoji && (
				<span
					className="absolute -top-10 left-1/2 -translate-x-1/2 text-2xl leading-none select-none drop-shadow-sm"
					aria-hidden="true"
				>
					{ping.emoji}
				</span>
			)}
			<div
				className="absolute left-3 top-3 flex items-center gap-1.5 whitespace-nowrap rounded-full border-2 bg-background pl-0.5 pr-2 py-0.5 ring-1 ring-border select-none"
				style={{ borderColor: ping.color }}
			>
				<PingAvatar ping={ping} />
				<span
					className="text-[11px] font-semibold max-w-24 truncate tracking-tight"
					style={{ color: ping.color }}
				>
					{ping.name}
				</span>
			</div>
		</div>
	);
});

const PingEdgeIndicator = memo(function PingEdgeIndicator({
	ping,
}: {
	ping: PingDisplay;
}) {
	if (ping.edgeX === undefined || ping.edgeY === undefined) return null;
	return (
		<div
			className="absolute left-0 top-0 transition-opacity duration-100 ease-linear motion-reduce:transition-none"
			style={{
				transform: `translate(${ping.edgeX}px, ${ping.edgeY}px)`,
				opacity: ping.opacity,
			}}
		>
			<div
				className="flex items-center gap-2 rounded-full border-2 bg-background/90 pl-1 pr-2.5 py-1.5 shadow-xl backdrop-blur-md ring-1 ring-white/20 animate-pulse motion-reduce:animate-none"
				style={{
					borderColor: ping.color,
					boxShadow: `0 4px 20px -4px color-mix(in srgb, ${ping.color} 30%, transparent), 0 8px 16px -8px rgba(0,0,0,0.3)`,
				}}
			>
				<PingAvatar ping={ping} />
				<span
					className="text-xs font-semibold max-w-20 truncate tracking-tight"
					style={{ color: ping.color }}
				>
					{ping.name}
				</span>
				{ping.emoji && (
					<span className="text-sm leading-none" aria-hidden="true">
						{ping.emoji}
					</span>
				)}
				<LocateFixedIcon
					className="h-3.5 w-3.5"
					style={{
						color: ping.color,
						filter: `drop-shadow(0 0 4px color-mix(in srgb, ${ping.color} 30%, transparent))`,
					}}
				/>
			</div>
		</div>
	);
});

const PingAvatar = memo(function PingAvatar({ ping }: { ping: PingDisplay }) {
	return (
		<Avatar className="h-5 w-5 ring-2 ring-white/50 shadow-sm">
			{ping.avatarUrl && (
				<AvatarImage src={ping.avatarUrl} className="object-cover" />
			)}
			<AvatarFallback
				className="text-[9px] font-bold text-white"
				style={{ background: ping.color }}
			>
				{ping.initial}
			</AvatarFallback>
		</Avatar>
	);
});
