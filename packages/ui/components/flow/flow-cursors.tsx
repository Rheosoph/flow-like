"use client";
import { useStore } from "@xyflow/react";
import { ArrowRight } from "lucide-react";
import { memo, useMemo, useSyncExternalStore } from "react";
import {
	type PeerUserInfo,
	colorFromSub,
	truncateName,
} from "../../hooks/use-peer-users";
import type { CursorStore } from "../../hooks/use-realtime-collaboration";
import { Avatar, AvatarFallback, AvatarImage } from "../ui/avatar";

interface CursorPeer {
	clientId: number;
	cursor?: { x: number; y: number };
	/** The sub (subject) from the auth token */
	sub?: string;
	layerPath: string;
}

interface CursorDisplay {
	key: string;
	sub?: string;
	color: string;
	name: string;
	avatarUrl?: string;
	x: number;
	y: number;
	offScreen: boolean;
	edgeX?: number;
	edgeY?: number;
	angle?: number;
}

function calculateEdgePosition(
	screenX: number,
	screenY: number,
	viewportWidth: number,
	viewportHeight: number,
	margin: number,
): { edgeX: number; edgeY: number; angle: number } {
	const centerX = viewportWidth / 2;
	const centerY = viewportHeight / 2;
	const dx = screenX - centerX;
	const dy = screenY - centerY;
	const angle = Math.atan2(dy, dx);

	const edgeMargin = 20;
	const clampedY = Math.max(
		edgeMargin,
		Math.min(viewportHeight - edgeMargin, screenY),
	);
	const clampedX = Math.max(
		edgeMargin,
		Math.min(viewportWidth - edgeMargin, screenX),
	);

	let edgeX: number;
	let edgeY: number;

	const isLeftEdge = screenX < margin;
	const isRightEdge = screenX > viewportWidth - margin;
	const isTopEdge = screenY < margin;
	const isBottomEdge = screenY > viewportHeight - margin;

	if (isLeftEdge) {
		edgeX = edgeMargin;
		edgeY = clampedY;
	} else if (isRightEdge) {
		edgeX = viewportWidth - edgeMargin;
		edgeY = clampedY;
	} else if (isTopEdge) {
		edgeX = clampedX;
		edgeY = edgeMargin;
	} else if (isBottomEdge) {
		edgeX = clampedX;
		edgeY = viewportHeight - edgeMargin;
	} else {
		// Shouldn't reach here, but default to bottom edge
		edgeX = clampedX;
		edgeY = viewportHeight - edgeMargin;
	}

	return { edgeX, edgeY, angle };
}

export const FlowCursors = memo(function FlowCursors({
	peers,
	currentLayerPath,
	peerUsers,
	className,
}: {
	peers: CursorPeer[];
	currentLayerPath: string;
	/** Map of sub -> user info for displaying names */
	peerUsers: Map<string, PeerUserInfo>;
	className?: string;
}) {
	const transform = useStore((state) => state.transform);
	const [tx, ty, zoom] = transform;

	const cursors = useMemo(() => {
		if (typeof window === "undefined") return [];

		const viewportWidth = window.innerWidth;
		const viewportHeight = window.innerHeight;
		const margin = 80;

		return peers
			.filter((peer) => peer.cursor && peer.layerPath === currentLayerPath)
			.map((peer) => {
				const cursor = peer.cursor!;
				const screenX = cursor.x * zoom + tx;
				const screenY = cursor.y * zoom + ty;

				const offScreen =
					screenX < margin ||
					screenX > viewportWidth - margin ||
					screenY < margin ||
					screenY > viewportHeight - margin;

				const edge = offScreen
					? calculateEdgePosition(
							screenX,
							screenY,
							viewportWidth,
							viewportHeight,
							margin,
						)
					: undefined;

				// Get user info from cache
				const userInfo = peer.sub ? peerUsers.get(peer.sub) : undefined;

				return {
					key: `${peer.sub ?? "user"}-${peer.clientId}`,
					sub: peer.sub,
					color: userInfo?.color ?? colorFromSub(peer.sub),
					name: userInfo?.truncatedName ?? truncateName(peer.sub?.slice(-8)),
					avatarUrl: userInfo?.avatarUrl,
					x: screenX,
					y: screenY,
					offScreen,
					edgeX: edge?.edgeX,
					edgeY: edge?.edgeY,
					angle: edge?.angle,
				} satisfies CursorDisplay;
			});
	}, [peers, currentLayerPath, tx, ty, zoom, peerUsers]);

	return (
		<div
			className={
				"pointer-events-none absolute inset-0 z-40 " + (className ?? "")
			}
		>
			{cursors.map((cursor) =>
				cursor.offScreen ? (
					<EdgeIndicator key={cursor.key} cursor={cursor} />
				) : (
					<RemoteCursor key={cursor.key} cursor={cursor} />
				),
			)}
		</div>
	);
});

const EdgeIndicator = memo(function EdgeIndicator({
	cursor,
}: {
	cursor: {
		key: string;
		color: string;
		name: string;
		avatarUrl?: string;
		edgeX?: number;
		edgeY?: number;
		angle?: number;
	};
}) {
	if (!cursor.edgeX || !cursor.edgeY) return null;

	return (
		<div
			className="absolute transition-transform duration-150 ease-out"
			style={{ transform: `translate(${cursor.edgeX}px, ${cursor.edgeY}px)` }}
		>
			<div
				className="flex items-center gap-2 rounded-full border-2 bg-background/90 pl-1 pr-2.5 py-1.5 shadow-xl backdrop-blur-md ring-1 ring-white/20 animate-pulse"
				style={{
					borderColor: cursor.color,
					boxShadow: `0 4px 20px -4px ${cursor.color}50, 0 8px 16px -8px rgba(0,0,0,0.3)`,
				}}
			>
				<Avatar
					className="h-5 w-5 ring-2 ring-white/50 shadow-sm"
					style={{ borderColor: cursor.color }}
				>
					{cursor.avatarUrl && (
						<AvatarImage src={cursor.avatarUrl} className="object-cover" />
					)}
					<AvatarFallback
						className="text-[9px] font-bold"
						style={{
							background: `linear-gradient(135deg, ${cursor.color}, ${cursor.color}dd)`,
							color: "white",
							textShadow: "0 1px 2px rgba(0,0,0,0.2)",
						}}
					>
						{cursor.name.charAt(0).toUpperCase()}
					</AvatarFallback>
				</Avatar>
				<span
					className="text-xs font-semibold max-w-20 truncate tracking-tight"
					style={{
						color: cursor.color,
						textShadow: `0 0 20px ${cursor.color}30`,
					}}
				>
					{cursor.name}
				</span>
				<ArrowRight
					className="h-3.5 w-3.5 transition-transform"
					style={{
						color: cursor.color,
						transform:
							cursor.angle !== undefined
								? `rotate(${cursor.angle}rad)`
								: undefined,
						filter: `drop-shadow(0 0 4px ${cursor.color}50)`,
					}}
				/>
			</div>
		</div>
	);
});

const RemoteCursor = memo(function RemoteCursor({
	cursor,
}: {
	cursor: {
		key: string;
		color: string;
		name: string;
		avatarUrl?: string;
		x: number;
		y: number;
	};
}) {
	return (
		<div
			className="absolute transition-transform duration-75 ease-out"
			style={{ transform: `translate(${cursor.x}px, ${cursor.y}px)` }}
		>
			<div className="flex items-start gap-0.5 select-none">
				<CursorPointer color={cursor.color} />
				<div
					className="flex items-center gap-2 rounded-full border-2 bg-background/90 pl-1 pr-2.5 py-1 shadow-xl backdrop-blur-md ring-1 ring-white/20 transition-all duration-150"
					style={{
						borderColor: cursor.color,
						boxShadow: `0 4px 20px -4px ${cursor.color}40, 0 8px 16px -8px rgba(0,0,0,0.3)`,
					}}
				>
					<Avatar
						className="h-5 w-5 ring-2 ring-white/50 shadow-sm"
						style={{ borderColor: cursor.color }}
					>
						{cursor.avatarUrl && (
							<AvatarImage src={cursor.avatarUrl} className="object-cover" />
						)}
						<AvatarFallback
							className="text-[9px] font-bold"
							style={{
								background: `linear-gradient(135deg, ${cursor.color}, ${cursor.color}dd)`,
								color: "white",
								textShadow: "0 1px 2px rgba(0,0,0,0.2)",
							}}
						>
							{cursor.name.charAt(0).toUpperCase()}
						</AvatarFallback>
					</Avatar>
					<span
						className="text-xs font-semibold max-w-24 truncate tracking-tight"
						style={{
							color: cursor.color,
							textShadow: `0 0 20px ${cursor.color}30`,
						}}
					>
						{cursor.name}
					</span>
				</div>
			</div>
		</div>
	);
});

/**
 * Subscribes to the high-frequency cursor store via useSyncExternalStore so that
 * remote cursor motion re-renders only this overlay — never the parent FlowBoard.
 */
export function FlowCursorsLayer({
	store,
	currentLayerPath,
	peerUsers,
	className,
}: {
	store: CursorStore;
	currentLayerPath: string;
	peerUsers: Map<string, PeerUserInfo>;
	className?: string;
}) {
	const peers = useSyncExternalStore(
		store.subscribe,
		store.getSnapshot,
		store.getSnapshot,
	);
	if (peers.length === 0) return null;
	return (
		<FlowCursors
			peers={peers}
			currentLayerPath={currentLayerPath}
			peerUsers={peerUsers}
			className={className}
		/>
	);
}

const CursorPointer = memo(function CursorPointer({
	color,
}: { color: string }) {
	return (
		<svg
			width="24"
			height="24"
			viewBox="0 0 24 24"
			fill="none"
			xmlns="http://www.w3.org/2000/svg"
			className="drop-shadow-lg"
			style={{ filter: `drop-shadow(0 2px 4px ${color}50)` }}
		>
			<path
				d="M4 3L19 12L11 13.5L7 21L4 3Z"
				fill={color}
				stroke="white"
				strokeWidth="2"
				strokeLinejoin="round"
			/>
		</svg>
	);
});
