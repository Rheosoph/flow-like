"use client";
import { useTranslation } from "@flow-like/locales";
import { useStore } from "@xyflow/react";
import { memo, useMemo, useSyncExternalStore } from "react";
import { useShallow } from "zustand/react/shallow";
import {
	type PeerUserInfo,
	colorFromSub,
	truncateName,
} from "../../hooks/use-peer-users";
import type {
	PeerDrag,
	SignalStore,
} from "../../lib/realtime/presence-signals-store";
import { Avatar, AvatarFallback, AvatarImage } from "../ui/avatar";

/** Size of a ghost whose node is not (yet) in the local graph. */
const FALLBACK_NODE_WIDTH = 180;
const FALLBACK_NODE_HEIGHT = 56;

interface GhostDisplay {
	key: string;
	x: number;
	y: number;
	width: number;
	height: number;
}

interface PeerGhostsDisplay {
	key: string;
	color: string;
	name: string;
	initial: string;
	avatarUrl?: string;
	ghosts: GhostDisplay[];
}

/**
 * Translucent outlines of the nodes other sessions are dragging right now,
 * at their live positions. Subscribes via useSyncExternalStore so 20 Hz drag
 * ticks re-render only this overlay, never the board.
 */
export function FlowDragGhostsLayer({
	store,
	currentLayerPath,
	peerUsers,
	sub,
}: {
	store: SignalStore<PeerDrag[]>;
	currentLayerPath: string;
	peerUsers: Map<string, PeerUserInfo>;
	sub?: string;
}) {
	const drags = useSyncExternalStore(
		store.subscribe,
		store.getSnapshot,
		store.getSnapshot,
	);
	if (drags.length === 0) return null;
	return (
		<FlowDragGhosts
			drags={drags}
			currentLayerPath={currentLayerPath}
			peerUsers={peerUsers}
			sub={sub}
		/>
	);
}

const FlowDragGhosts = memo(function FlowDragGhosts({
	drags,
	currentLayerPath,
	peerUsers,
	sub,
}: {
	drags: PeerDrag[];
	currentLayerPath: string;
	peerUsers: Map<string, PeerUserInfo>;
	sub?: string;
}) {
	const { t } = useTranslation("flow");
	const { transform, nodeLookup } = useStore(
		useShallow((state) => ({
			transform: state.transform,
			nodeLookup: state.nodeLookup,
		})),
	);
	const [tx, ty, zoom] = transform;

	const peers = useMemo(
		() =>
			drags
				.filter((drag) => drag.layerPath === currentLayerPath)
				.map((drag): PeerGhostsDisplay => {
					const self = Boolean(sub && drag.sub === sub);
					const info = drag.sub ? peerUsers.get(drag.sub) : undefined;
					const peerName =
						info?.truncatedName ?? truncateName(drag.sub?.slice(-8));
					return {
						key: `${drag.sub ?? "user"}-${drag.clientId}`,
						color: info?.color ?? colorFromSub(drag.sub),
						name: self ? t("you", "You") : peerName,
						initial: peerName.charAt(0).toUpperCase(),
						avatarUrl: info?.avatarUrl,
						ghosts: drag.nodes.map((node) => {
							const measured = nodeLookup.get(node.id)?.measured;
							return {
								key: node.id,
								x: node.x * zoom + tx,
								y: node.y * zoom + ty,
								width: (measured?.width ?? FALLBACK_NODE_WIDTH) * zoom,
								height: (measured?.height ?? FALLBACK_NODE_HEIGHT) * zoom,
							};
						}),
					};
				}),
		[drags, currentLayerPath, peerUsers, sub, t, nodeLookup, tx, ty, zoom],
	);

	if (peers.length === 0) return null;

	return (
		<div className="pointer-events-none absolute inset-0 z-30">
			{peers.map((peer) => (
				<PeerGhosts key={peer.key} peer={peer} />
			))}
		</div>
	);
});

const PeerGhosts = memo(function PeerGhosts({
	peer,
}: {
	peer: PeerGhostsDisplay;
}) {
	return (
		<>
			{peer.ghosts.map((ghost, index) => (
				<div
					key={ghost.key}
					className="absolute left-0 top-0 rounded-lg border-2 border-dashed will-change-transform"
					style={{
						transform: `translate3d(${ghost.x}px, ${ghost.y}px, 0)`,
						width: ghost.width,
						height: ghost.height,
						borderColor: peer.color,
						backgroundColor: `color-mix(in srgb, ${peer.color} 12%, transparent)`,
					}}
				>
					{index === 0 && <GhostNameTag peer={peer} />}
				</div>
			))}
		</>
	);
});

const GhostNameTag = memo(function GhostNameTag({
	peer,
}: {
	peer: PeerGhostsDisplay;
}) {
	return (
		<div
			className="absolute -top-8 left-0 flex items-center gap-1.5 whitespace-nowrap rounded-full border-2 bg-background pl-0.5 pr-2 py-0.5 ring-1 ring-border select-none"
			style={{ borderColor: peer.color }}
		>
			<Avatar className="h-4 w-4 ring-2 ring-white/50">
				{peer.avatarUrl && (
					<AvatarImage src={peer.avatarUrl} className="object-cover" />
				)}
				<AvatarFallback
					className="text-[8px] font-bold text-white"
					style={{ background: peer.color }}
				>
					{peer.initial}
				</AvatarFallback>
			</Avatar>
			<span
				className="text-[11px] font-semibold max-w-24 truncate tracking-tight"
				style={{ color: peer.color }}
			>
				{peer.name}
			</span>
		</div>
	);
});
