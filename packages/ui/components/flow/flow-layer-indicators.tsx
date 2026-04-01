"use client";
import { type Node, useStore } from "@xyflow/react";
import { Layers } from "lucide-react";
import { memo, useMemo } from "react";
import { type PeerUserInfo, colorFromSub } from "../../hooks/use-peer-users";
import { Avatar, AvatarFallback, AvatarImage } from "../ui/avatar";
import {
	Tooltip,
	TooltipContent,
	TooltipProvider,
	TooltipTrigger,
} from "../ui/tooltip";

interface PeerPresence {
	clientId: number;
	/** The sub (subject) from the auth token */
	sub?: string;
	layerPath: string;
}

interface LayerIndicator {
	nodeId: string;
	layerPath: string;
	screenX: number;
	screenY: number;
	peers: Array<{
		sub?: string;
		color: string;
		name: string;
		avatarUrl?: string;
	}>;
}

export const FlowLayerIndicators = memo(function FlowLayerIndicators({
	peers,
	currentLayerPath,
	nodes,
	peerUsers,
	onJumpToLayer,
}: {
	peers: PeerPresence[];
	currentLayerPath: string;
	nodes: Node[];
	peerUsers: Map<string, PeerUserInfo>;
	onJumpToLayer?: (layerPath: string) => void;
}) {
	const transform = useStore((state) => state.transform);
	const [tx, ty, zoom] = transform;

	const indicators = useMemo(() => {
		const peersByLayer = new Map<string, PeerPresence[]>();

		const normalizedCurrentPath =
			!currentLayerPath || currentLayerPath === "root" ? "" : currentLayerPath;

		for (const peer of peers) {
			const normalizedPeerPath =
				!peer.layerPath || peer.layerPath === "root" ? "" : peer.layerPath;
			if (normalizedPeerPath === normalizedCurrentPath) continue;

			const existing = peersByLayer.get(peer.layerPath) ?? [];
			existing.push(peer);
			peersByLayer.set(peer.layerPath, existing);
		}

		if (peersByLayer.size === 0) return [];

		const result: LayerIndicator[] = [];

		for (const node of nodes) {
			if (node.type !== "layer") continue;

			const matchingPeers: PeerPresence[] = [];

			const nodePath = normalizedCurrentPath
				? `${normalizedCurrentPath}/${node.id}`
				: node.id;

			for (const [peerLayer, layerPeers] of peersByLayer.entries()) {
				const normalizedPeerLayer =
					!peerLayer || peerLayer === "root" ? "" : peerLayer;

				if (normalizedPeerLayer === nodePath) {
					matchingPeers.push(...layerPeers);
					continue;
				}

				if (normalizedPeerLayer.startsWith(`${nodePath}/`)) {
					matchingPeers.push(...layerPeers);
				}
			}

			if (matchingPeers.length === 0) continue;

			const nodeScreenX =
				(node.position.x + (node.measured?.width ?? 0)) * zoom + tx;
			const nodeScreenY = node.position.y * zoom + ty;

			result.push({
				nodeId: node.id,
				layerPath: nodePath,
				screenX: nodeScreenX,
				screenY: nodeScreenY,
				peers: matchingPeers.map((p) => {
					const userInfo = p.sub ? peerUsers.get(p.sub) : undefined;
					return {
						sub: p.sub,
						color: userInfo?.color ?? colorFromSub(p.sub),
						name: userInfo?.truncatedName ?? "User",
						avatarUrl: userInfo?.avatarUrl,
					};
				}),
			});
		}

		return result;
	}, [peers, currentLayerPath, nodes, tx, ty, zoom, peerUsers]);

	return (
		<div className="pointer-events-none absolute inset-0 z-30">
			<TooltipProvider delayDuration={200}>
				{indicators.map((indicator) => (
					<div
						key={indicator.nodeId}
						className="absolute"
						style={{
							transform: `translate(${indicator.screenX}px, ${indicator.screenY}px)`,
						}}
					>
						<Tooltip>
							<TooltipTrigger asChild>
								<button
									type="button"
									onClick={() => onJumpToLayer?.(indicator.layerPath)}
									className="pointer-events-auto flex items-center gap-1.5 rounded-full border-2 border-border/60 bg-background/95 px-2 py-1 shadow-xl backdrop-blur-md cursor-pointer hover:shadow-2xl hover:scale-105 transition-all duration-150 group"
									style={{
										borderColor: indicator.peers[0]?.color ?? "var(--border)",
										boxShadow: `0 4px 16px -4px ${indicator.peers[0]?.color ?? "transparent"}40`,
									}}
								>
									<div className="flex items-center -space-x-1.5">
										{indicator.peers.slice(0, 3).map((peer, idx) => (
											<Avatar
												key={`${peer.sub ?? "unknown"}-${idx}`}
												className="h-5 w-5 border-2 border-background shadow-sm"
												style={{
													borderColor: peer.color,
												}}
											>
												{peer.avatarUrl && (
													<AvatarImage
														src={peer.avatarUrl}
														className="object-cover"
													/>
												)}
												<AvatarFallback
													className="text-[8px] font-bold"
													style={{
														background: `linear-gradient(135deg, ${peer.color}, ${peer.color}dd)`,
														color: "white",
													}}
												>
													{peer.name.charAt(0).toUpperCase()}
												</AvatarFallback>
											</Avatar>
										))}
										{indicator.peers.length > 3 && (
											<div className="flex h-5 w-5 items-center justify-center rounded-full border-2 border-background bg-muted text-[8px] font-bold shadow-sm">
												+{indicator.peers.length - 3}
											</div>
										)}
									</div>
									<Layers className="h-3 w-3 text-muted-foreground group-hover:text-foreground transition-colors" />
								</button>
							</TooltipTrigger>
							<TooltipContent side="top" className="text-xs">
								<div className="flex flex-col gap-0.5">
									<span className="font-medium">
										{indicator.peers.length} user
										{indicator.peers.length > 1 ? "s" : ""} inside this layer
									</span>
									<span className="text-muted-foreground">
										Click to jump in
									</span>
								</div>
							</TooltipContent>
						</Tooltip>
					</div>
				))}
			</TooltipProvider>
		</div>
	);
});
