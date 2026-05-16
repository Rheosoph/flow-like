"use client";
import {
	ChevronDown,
	ChevronUp,
	Eye,
	Layers,
	MessageCircle,
	MousePointerClick,
	Navigation,
	Users,
} from "lucide-react";
import { memo, useCallback, useMemo, useState } from "react";
import {
	type PeerUserInfo,
	colorFromSub,
	truncateName,
} from "../../hooks/use-peer-users";
import type { PeerPresence } from "../../hooks/use-realtime-collaboration";
import { Avatar, AvatarFallback, AvatarImage } from "../ui/avatar";
import { Badge } from "../ui/badge";
import {
	Tooltip,
	TooltipContent,
	TooltipProvider,
	TooltipTrigger,
} from "../ui/tooltip";

function layerLabel(
	layerPath: string,
	layerNames?: Map<string, string>,
): string {
	if (!layerPath || layerPath === "root") return "Root";
	const segments = layerPath.split("/");
	const last = segments[segments.length - 1];
	return layerNames?.get(last) ?? last.slice(0, 10);
}

export const FlowPresenceBar = memo(function FlowPresenceBar({
	peers,
	peerUsers,
	followingSub,
	currentLayerPath,
	layerNames,
	onToggleFollow,
	onJumpToUser,
	onJumpToLayer,
	onOpenChat,
	unreadCount,
}: {
	peers: PeerPresence[];
	peerUsers: Map<string, PeerUserInfo>;
	followingSub?: string;
	currentLayerPath: string;
	layerNames?: Map<string, string>;
	onToggleFollow: (sub: string) => void;
	onJumpToUser: (sub: string) => void;
	onJumpToLayer: (layerPath: string) => void;
	onOpenChat?: () => void;
	unreadCount?: number;
}) {
	const [expanded, setExpanded] = useState(false);

	const uniquePeers = useMemo(() => {
		const byUser = new Map<string, PeerPresence>();
		for (const p of peers) {
			if (!p.sub) continue;
			const existing = byUser.get(p.sub);
			if (!existing) {
				byUser.set(p.sub, p);
			} else {
				// Merge sessions: combine selections, keep most recent activity
				const mergedNodes = [
					...new Set([...existing.selection.nodes, ...p.selection.nodes]),
				];
				byUser.set(p.sub, {
					...existing,
					selection: { nodes: mergedNodes },
					cursor: p.cursor ?? existing.cursor,
					activeNodeId: p.activeNodeId ?? existing.activeNodeId,
					activeNodeTs:
						Math.max(p.activeNodeTs ?? 0, existing.activeNodeTs ?? 0) ||
						undefined,
				});
			}
		}
		return [...byUser.values()];
	}, [peers]);

	const peersInDifferentLayer = useMemo(
		() =>
			uniquePeers.filter(
				(p) => (p.layerPath ?? "root") !== (currentLayerPath || "root"),
			),
		[uniquePeers, currentLayerPath],
	);

	const toggleExpanded = useCallback(() => setExpanded((v) => !v), []);

	if (uniquePeers.length === 0) return null;

	const showPanel = expanded && uniquePeers.length > 0;

	return (
		<div className="flex flex-col gap-0 select-none">
			{/* Compact bar */}
			<div className="flex items-center gap-1.5 rounded-xl border border-border/50 bg-background/90 px-2 py-1.5 backdrop-blur-sm shadow-sm">
				<TooltipProvider delayDuration={200}>
					<Users className="h-3.5 w-3.5 text-muted-foreground mr-0.5" />
					{uniquePeers.slice(0, 5).map((peer) => {
						const userInfo = peer.sub ? peerUsers.get(peer.sub) : undefined;
						const color = userInfo?.color ?? colorFromSub(peer.sub);
						const displayName =
							userInfo?.truncatedName ?? truncateName(peer.sub?.slice(-8));
						const fullName = userInfo?.name ?? peer.sub?.slice(-8) ?? "User";
						const isFollowing = followingSub === peer.sub;
						const sameLayer =
							(peer.layerPath ?? "root") === (currentLayerPath || "root");

						return (
							<Tooltip key={peer.sub}>
								<TooltipTrigger asChild>
									<button
										type="button"
										onClick={() => peer.sub && onToggleFollow(peer.sub)}
										className="relative group cursor-pointer"
									>
										<Avatar
											className={`h-6 w-6 transition-all duration-150 ${isFollowing ? "ring-2" : "ring-1 ring-border/50"}`}
											style={{
												borderColor: color,
												boxShadow: isFollowing
													? `0 0 0 2px ${color}, 0 0 12px ${color}40`
													: undefined,
											}}
										>
											{userInfo?.avatarUrl && (
												<AvatarImage
													src={userInfo.avatarUrl}
													className="object-cover"
												/>
											)}
											<AvatarFallback
												className="text-[10px] font-bold"
												style={{
													background: `linear-gradient(135deg, ${color}, ${color}dd)`,
													color: "white",
												}}
											>
												{displayName.charAt(0).toUpperCase()}
											</AvatarFallback>
										</Avatar>
										{isFollowing && (
											<div
												className="absolute -bottom-0.5 -right-0.5 rounded-full p-0.5"
												style={{ backgroundColor: color }}
											>
												<Eye className="h-2 w-2 text-white" />
											</div>
										)}
										{!sameLayer && (
											<div className="absolute -top-0.5 -left-0.5 rounded-full bg-background p-0.5 shadow-sm">
												<Layers className="h-2 w-2 text-muted-foreground" />
											</div>
										)}
									</button>
								</TooltipTrigger>
								<TooltipContent side="bottom" className="text-xs max-w-56">
									<div className="flex flex-col gap-0.5">
										<span className="font-medium">{fullName}</span>
										<span className="text-muted-foreground">
											Layer: {layerLabel(peer.layerPath, layerNames)}
										</span>
										{peer.selection?.nodes?.length > 0 && (
											<span className="text-muted-foreground">
												<MousePointerClick className="h-3 w-3 inline mr-0.5" />
												{peer.selection.nodes.length} node
												{peer.selection.nodes.length > 1 ? "s" : ""} selected
											</span>
										)}
										{isFollowing ? (
											<span className="text-primary font-medium">
												Following — click to stop
											</span>
										) : (
											<span className="text-foreground">Click to follow</span>
										)}
									</div>
								</TooltipContent>
							</Tooltip>
						);
					})}
					{uniquePeers.length > 5 && (
						<span className="text-xs text-muted-foreground ml-0.5">
							+{uniquePeers.length - 5}
						</span>
					)}

					<>
						<div className="w-px h-4 bg-border/50 mx-0.5" />
						<Tooltip>
							<TooltipTrigger asChild>
								<button
									type="button"
									onClick={toggleExpanded}
									className="flex items-center gap-0.5 text-[10px] text-muted-foreground hover:text-foreground transition-colors cursor-pointer"
								>
									{peersInDifferentLayer.length > 0 && (
										<>
											<Layers className="h-3 w-3" />
											<span>{peersInDifferentLayer.length}</span>
										</>
									)}
									{expanded ? (
										<ChevronUp className="h-2.5 w-2.5" />
									) : (
										<ChevronDown className="h-2.5 w-2.5" />
									)}
								</button>
							</TooltipTrigger>
							<TooltipContent side="bottom" className="text-xs">
								{expanded
									? "Collapse"
									: peersInDifferentLayer.length > 0
										? `${peersInDifferentLayer.length} user${peersInDifferentLayer.length > 1 ? "s" : ""} in other layers`
										: "Show collaborators"}
							</TooltipContent>
						</Tooltip>
					</>

					{onOpenChat && (
						<>
							<div className="w-px h-4 bg-border/50 mx-0.5" />
							<Tooltip>
								<TooltipTrigger asChild>
									<button
										type="button"
										onClick={onOpenChat}
										className="relative flex items-center justify-center h-6 w-6 rounded-md hover:bg-muted/50 transition-colors"
									>
										<MessageCircle className="h-3.5 w-3.5 text-muted-foreground" />
										{(unreadCount ?? 0) > 0 && (
											<span className="absolute -top-0.5 -right-0.5 flex h-3.5 min-w-3.5 items-center justify-center rounded-full bg-primary px-1 text-[9px] font-bold text-primary-foreground">
												{(unreadCount ?? 0) > 99 ? "99+" : unreadCount}
											</span>
										)}
									</button>
								</TooltipTrigger>
								<TooltipContent side="bottom" className="text-xs">
									Chat
								</TooltipContent>
							</Tooltip>
						</>
					)}
				</TooltipProvider>
			</div>

			{/* Expanded collaborators panel */}
			{showPanel && (
				<CollaboratorsPanel
					peers={uniquePeers}
					peerUsers={peerUsers}
					followingSub={followingSub}
					currentLayerPath={currentLayerPath}
					layerNames={layerNames}
					onToggleFollow={onToggleFollow}
					onJumpToUser={onJumpToUser}
					onJumpToLayer={onJumpToLayer}
				/>
			)}
		</div>
	);
});

const CollaboratorsPanel = memo(function CollaboratorsPanel({
	peers,
	peerUsers,
	followingSub,
	currentLayerPath,
	layerNames,
	onToggleFollow,
	onJumpToUser,
	onJumpToLayer,
}: {
	peers: PeerPresence[];
	peerUsers: Map<string, PeerUserInfo>;
	followingSub?: string;
	currentLayerPath: string;
	layerNames?: Map<string, string>;
	onToggleFollow: (sub: string) => void;
	onJumpToUser: (sub: string) => void;
	onJumpToLayer: (layerPath: string) => void;
}) {
	const grouped = useMemo(() => {
		const byLayer = new Map<string, PeerPresence[]>();
		for (const peer of peers) {
			const key = peer.layerPath ?? "root";
			const arr = byLayer.get(key) ?? [];
			arr.push(peer);
			byLayer.set(key, arr);
		}
		// Sort: current layer first, then alphabetical
		const currentKey = currentLayerPath || "root";
		const entries = [...byLayer.entries()].sort(([a], [b]) => {
			if (a === currentKey) return -1;
			if (b === currentKey) return 1;
			return a.localeCompare(b);
		});
		return entries;
	}, [peers, currentLayerPath]);

	return (
		<div className="mt-1 w-64 max-h-80 overflow-y-auto rounded-xl border border-border/50 bg-background/95 backdrop-blur-md shadow-lg">
			<div className="px-3 py-2 border-b border-border/30">
				<h4 className="text-xs font-semibold text-foreground flex items-center gap-1.5">
					<Users className="h-3.5 w-3.5" />
					Collaborators
					<Badge
						variant="secondary"
						className="ml-auto text-[10px] px-1.5 py-0"
					>
						{peers.length}
					</Badge>
				</h4>
			</div>
			<div className="py-1">
				{grouped.map(([layerKey, layerPeers]) => {
					const isCurrentLayer = layerKey === (currentLayerPath || "root");
					const label = layerLabel(layerKey, layerNames);
					return (
						<div key={layerKey}>
							<div className="flex items-center gap-1.5 px-3 py-1">
								<Layers className="h-3 w-3 text-muted-foreground shrink-0" />
								<span className="text-[10px] font-medium text-muted-foreground truncate">
									{label}
								</span>
								{isCurrentLayer && (
									<Badge
										variant="outline"
										className="text-[9px] px-1 py-0 ml-auto"
									>
										You
									</Badge>
								)}
								{!isCurrentLayer && (
									<button
										type="button"
										onClick={() => onJumpToLayer(layerKey)}
										className="ml-auto text-[10px] text-primary hover:text-primary/80 font-medium flex items-center gap-0.5 cursor-pointer transition-colors"
									>
										<Navigation className="h-2.5 w-2.5" />
										Go
									</button>
								)}
							</div>
							{layerPeers.map((peer) => (
								<CollaboratorRow
									key={peer.sub}
									peer={peer}
									peerUsers={peerUsers}
									followingSub={followingSub}
									onToggleFollow={onToggleFollow}
									onJumpToUser={onJumpToUser}
								/>
							))}
						</div>
					);
				})}
			</div>
		</div>
	);
});

const CollaboratorRow = memo(function CollaboratorRow({
	peer,
	peerUsers,
	followingSub,
	onToggleFollow,
	onJumpToUser,
}: {
	peer: PeerPresence;
	peerUsers: Map<string, PeerUserInfo>;
	followingSub?: string;
	onToggleFollow: (sub: string) => void;
	onJumpToUser: (sub: string) => void;
}) {
	const userInfo = peer.sub ? peerUsers.get(peer.sub) : undefined;
	const color = userInfo?.color ?? colorFromSub(peer.sub);
	const name = userInfo?.truncatedName ?? truncateName(peer.sub?.slice(-8));
	const isFollowing = followingSub === peer.sub;
	const selectedCount = peer.selection?.nodes?.length ?? 0;

	return (
		<div className="flex items-center gap-2 px-3 py-1.5 hover:bg-muted/30 transition-colors group">
			<Avatar
				className={`h-6 w-6 shrink-0 transition-all ${isFollowing ? "ring-2" : "ring-1 ring-border/50"}`}
				style={{
					borderColor: color,
					boxShadow: isFollowing ? `0 0 0 2px ${color}` : undefined,
				}}
			>
				{userInfo?.avatarUrl && (
					<AvatarImage src={userInfo.avatarUrl} className="object-cover" />
				)}
				<AvatarFallback
					className="text-[10px] font-bold"
					style={{
						background: `linear-gradient(135deg, ${color}, ${color}dd)`,
						color: "white",
					}}
				>
					{name.charAt(0).toUpperCase()}
				</AvatarFallback>
			</Avatar>
			<div className="flex-1 min-w-0">
				<div className="text-xs font-medium truncate">{name}</div>
				{selectedCount > 0 && (
					<div className="text-[10px] text-muted-foreground flex items-center gap-0.5">
						<MousePointerClick className="h-2.5 w-2.5" />
						{selectedCount} selected
					</div>
				)}
			</div>
			<div className="flex items-center gap-0.5 opacity-0 group-hover:opacity-100 transition-opacity">
				<TooltipProvider delayDuration={100}>
					<Tooltip>
						<TooltipTrigger asChild>
							<button
								type="button"
								onClick={() => peer.sub && onJumpToUser(peer.sub)}
								className="p-1 rounded hover:bg-muted/60 transition-colors cursor-pointer"
							>
								<Navigation className="h-3 w-3 text-muted-foreground" />
							</button>
						</TooltipTrigger>
						<TooltipContent side="left" className="text-xs">
							Jump to user
						</TooltipContent>
					</Tooltip>
					<Tooltip>
						<TooltipTrigger asChild>
							<button
								type="button"
								onClick={() => peer.sub && onToggleFollow(peer.sub)}
								className={`p-1 rounded transition-colors cursor-pointer ${isFollowing ? "bg-primary/10" : "hover:bg-muted/60"}`}
							>
								<Eye
									className={`h-3 w-3 ${isFollowing ? "text-primary" : "text-muted-foreground"}`}
								/>
							</button>
						</TooltipTrigger>
						<TooltipContent side="left" className="text-xs">
							{isFollowing ? "Stop following" : "Follow"}
						</TooltipContent>
					</Tooltip>
				</TooltipProvider>
			</div>
		</div>
	);
});
