"use client";

import { useTranslation } from "@flow-like/locales";
import {
	BracesIcon,
	EyeIcon,
	EyeOffIcon,
	FileCode2Icon,
	HistoryIcon,
	LayersIcon,
	MegaphoneIcon,
	MessageCircleIcon,
	MousePointerClickIcon,
	NavigationIcon,
	PencilLineIcon,
	PlayIcon,
	UsersIcon,
	XIcon,
} from "lucide-react";
import {
	type FocusEvent,
	type ReactNode,
	memo,
	useCallback,
	useEffect,
	useMemo,
	useRef,
	useState,
} from "react";
import {
	type PeerUserInfo,
	colorFromSub,
	truncateName,
} from "../../hooks/use-peer-users";
import {
	type IBoardModule,
	MAIN_FILE_ID,
	MAIN_FILE_LABEL,
} from "../../lib/flow-modules";
import type { PeerPresence } from "../../lib/realtime/peer-presence";
import {
	type EditVerb,
	PING_EMOJI,
	type PingEmoji,
} from "../../lib/realtime/presence-signals";
import { cn } from "../../lib/utils";
import { Avatar, AvatarFallback, AvatarImage } from "../ui/avatar";
import { Badge } from "../ui/badge";
import {
	type PresenceActivity,
	type PresenceCollaborator,
	type PresenceEvent,
	type PresenceLastEdit,
	type PresenceLastRun,
	describeActivity,
	mergeCollaborators,
	presenceEventRemainingMs,
	presenceHighlightIds,
	presenceStats,
	sortCollaborators,
} from "./flow-presence-bar-model";
import { BoardStatusItem } from "./shell/board-status-bar";

/** Faces shown in the status bar before the count takes over. */
const FACEPILE_LIMIT = 4;
/** Typing marks expire within seconds, so live views re-check at this rate. */
const LIVE_REFRESH_MS = 1000;
/** The join/leave notice fades over its last stretch; matches `duration-500`. */
const EVENT_FADE_MS = 500;

export interface FlowPresenceBarProps {
	peers: PeerPresence[];
	peerUsers: Map<string, PeerUserInfo>;
	/** The local user; their other windows are listed as "You". */
	sub?: string;
	followingSub?: string;
	currentLayerPath: string;
	layerNames?: Map<string, string>;
	modules?: IBoardModule[];
	/** Display name of a node, for "editing <node>". */
	resolveNodeName?: (nodeId: string) => string | undefined;
	/** Local-clock time a user last did anything; drives the idle badge. */
	getLastActiveAt?: (sub: string) => number | undefined;
	/** Local-clock predicates; cheap enough to poll every second. */
	isTypingInEditor?: (sub: string) => boolean;
	isTypingInChat?: (sub: string) => boolean;
	isAway?: (sub: string) => boolean;
	onToggleFollow: (sub: string) => void;
	onStopFollowing?: () => void;
	onJumpToUser: (sub: string) => void;
	onJumpToLayer: (layerPath: string) => void;
	onFocusNode?: (nodeId: string) => void;
	/** Open the FlowScript panel at the node a peer's cursor sits on. */
	onOpenInCode?: (nodeId: string) => void;
	/** Light up a collaborator's nodes on the canvas while their row is hovered. */
	onHighlightNodes?: (nodeIds?: string[]) => void;
	onOpenChat?: () => void;
	unreadCount?: number;
	/** Peers' shared FlowScript scope node ids, keyed by sub. */
	peerScopes?: Map<string, string[]>;
	/** Join a peer's shared scope (opens the FlowScript panel on those nodes). */
	onJoinScope?: (nodeIds: string[]) => void;
	/** "Bring everyone here": broadcast the local viewport; absent = hide the button. */
	onSummon?: () => void;
	/** Send an emoji reaction at the local cursor. */
	onReact?: (emoji: PingEmoji) => void;
	/** Latest join/leave, shown briefly beside the count. */
	presenceEvent?: PresenceEvent;
}

interface PresenceIdentity {
	color: string;
	name: string;
	shortName: string;
	avatarUrl?: string;
	known: boolean;
}

function identityOf(
	sub: string,
	peerUsers: Map<string, PeerUserInfo>,
): PresenceIdentity {
	const info = peerUsers.get(sub);
	const fallback = sub.slice(-8);
	return {
		color: info?.color ?? colorFromSub(sub),
		name: info?.name ?? fallback,
		shortName: info?.truncatedName ?? truncateName(fallback),
		avatarUrl: info?.avatarUrl,
		known: Boolean(info),
	};
}

function formatCount(count: number): string {
	return count > 99 ? "99+" : String(count);
}

/** Local time, re-read every `intervalMs`; `0` stops the clock. */
function useClock(intervalMs: number): number {
	const [now, setNow] = useState(() => Date.now());
	useEffect(() => {
		if (intervalMs <= 0) return;
		const id = setInterval(() => setNow(Date.now()), intervalMs);
		return () => clearInterval(id);
	}, [intervalMs]);
	return now;
}

/** Shows a join/leave notice for its TTL from `at`, fading at the end. */
function usePresenceNotice(
	event: PresenceEvent | undefined,
): { fading: boolean } | undefined {
	const [phase, setPhase] = useState<"hidden" | "shown" | "fading">("hidden");
	useEffect(() => {
		const remaining = presenceEventRemainingMs(event, Date.now());
		if (remaining <= 0) {
			setPhase("hidden");
			return;
		}
		setPhase("shown");
		const fadeId = setTimeout(
			() => setPhase("fading"),
			Math.max(0, remaining - EVENT_FADE_MS),
		);
		const hideId = setTimeout(() => setPhase("hidden"), remaining);
		return () => {
			clearTimeout(fadeId);
			clearTimeout(hideId);
		};
	}, [event]);
	return phase === "hidden" ? undefined : { fading: phase === "fading" };
}

/**
 * Who else is on the board, as a status-bar item: a facepile and a count that
 * read at a glance, and a popover that says where each person is and what they
 * are doing, with follow / jump / open-in-code / join-scope beside each row.
 */
export const FlowPresenceBar = memo(function FlowPresenceBar({
	peers,
	peerUsers,
	sub,
	followingSub,
	currentLayerPath,
	layerNames,
	modules,
	resolveNodeName,
	getLastActiveAt,
	isTypingInEditor,
	isTypingInChat,
	isAway,
	onToggleFollow,
	onStopFollowing,
	onJumpToUser,
	onJumpToLayer,
	onFocusNode,
	onOpenInCode,
	onHighlightNodes,
	onOpenChat,
	unreadCount,
	peerScopes,
	onJoinScope,
	onSummon,
	onReact,
	presenceEvent,
}: Readonly<FlowPresenceBarProps>) {
	const { t } = useTranslation("flow");

	const collaborators = useMemo(
		() => mergeCollaborators(peers, sub),
		[peers, sub],
	);
	const nameOf = useCallback(
		(userSub: string) => identityOf(userSub, peerUsers).name,
		[peerUsers],
	);
	const sorted = useMemo(
		() => sortCollaborators(collaborators, currentLayerPath, nameOf),
		[collaborators, currentLayerPath, nameOf],
	);
	const stats = useMemo(() => presenceStats(collaborators), [collaborators]);
	const fileLabels = useMemo(() => {
		const map = new Map<string, string>([[MAIN_FILE_ID, MAIN_FILE_LABEL]]);
		for (const module of modules ?? []) map.set(module.id, module.pathLabel);
		return map;
	}, [modules]);

	const unread = unreadCount ?? 0;
	const alone = sorted.length === 0;
	// The facepile's typing/away marks come from local-clock predicates, so the
	// bar re-reads them on a slow tick — only while there is a face to mark.
	const liveFaces = Boolean(isTypingInEditor || isAway);
	useClock(alone || !liveFaces ? 0 : LIVE_REFRESH_MS);

	const notice = usePresenceNotice(presenceEvent);
	const noticeName = useMemo(() => {
		if (!presenceEvent) return undefined;
		const identity = identityOf(presenceEvent.sub, peerUsers);
		return presenceEvent.sub === sub && !identity.known
			? t("you", "You")
			: identity.shortName;
	}, [presenceEvent, peerUsers, sub, t]);

	return (
		<BoardStatusItem
			icon={<UsersIcon />}
			tone={alone ? "muted" : "default"}
			title={t("showCollaborators", "Show collaborators")}
			ariaLabel={t("collaboratorsOnline", {
				defaultValue_one: "{{count}} online — show collaborators",
				defaultValue_other: "{{count}} online — show collaborators",
				count: stats.onlineCount,
			})}
			popoverAlign="start"
			popoverClassName="w-80 p-2"
			popover={
				<PresencePopover
					collaborators={sorted}
					peerUsers={peerUsers}
					onlineCount={stats.onlineCount}
					followingSub={followingSub}
					currentLayerPath={currentLayerPath}
					layerNames={layerNames}
					fileLabels={fileLabels}
					resolveNodeName={resolveNodeName}
					getLastActiveAt={getLastActiveAt}
					isTypingInEditor={isTypingInEditor}
					isTypingInChat={isTypingInChat}
					isAway={isAway}
					onToggleFollow={onToggleFollow}
					onStopFollowing={onStopFollowing}
					onJumpToUser={onJumpToUser}
					onJumpToLayer={onJumpToLayer}
					onFocusNode={onFocusNode}
					onOpenInCode={onOpenInCode}
					onHighlightNodes={onHighlightNodes}
					onOpenChat={onOpenChat}
					unread={unread}
					peerScopes={peerScopes}
					onJoinScope={onJoinScope}
					onSummon={onSummon}
					onReact={onReact}
				/>
			}
		>
			{!alone && (
				<span className="flex items-center -space-x-1" aria-hidden="true">
					{sorted.slice(0, FACEPILE_LIMIT).map((collab) => (
						<PresenceFace
							key={collab.sub}
							identity={identityOf(collab.sub, peerUsers)}
							typing={isTypingInEditor?.(collab.sub) ?? false}
							away={isAway?.(collab.sub) ?? false}
						/>
					))}
				</span>
			)}
			<span className="tabular-nums">{stats.onlineCount}</span>
			<span className="sr-only" aria-live="polite">
				{notice && presenceEvent && noticeName
					? presenceEvent.kind === "joined"
						? t("presenceJoined", "{{name}} joined", { name: noticeName })
						: t("presenceLeft", "{{name}} left", { name: noticeName })
					: ""}
			</span>
			{notice && presenceEvent && noticeName && (
				<span
					aria-hidden="true"
					className={cn(
						"animate-in fade-in text-muted-foreground transition-opacity duration-500 motion-reduce:animate-none motion-reduce:transition-none",
						notice.fading && "opacity-0",
					)}
				>
					{presenceEvent.kind === "joined"
						? t("presenceJoined", "{{name}} joined", { name: noticeName })
						: t("presenceLeft", "{{name}} left", { name: noticeName })}
				</span>
			)}
			{stats.inCodeEditor > 0 && (
				<span
					className="flex items-center gap-0.5 text-muted-foreground"
					title={t("countInCodeEditor", {
						defaultValue_one: "{{count}} in the code editor",
						defaultValue_other: "{{count}} in the code editor",
						count: stats.inCodeEditor,
					})}
				>
					<PencilLineIcon className="size-3" aria-hidden="true" />
					<span className="tabular-nums">{stats.inCodeEditor}</span>
				</span>
			)}
			{unread > 0 && (
				<span
					className="min-w-3.5 rounded-full bg-primary px-1 text-[9px] font-semibold leading-3.5 tabular-nums text-primary-foreground"
					aria-label={t("countUnreadMessages", {
						defaultValue_one: "{{count}} unread message",
						defaultValue_other: "{{count}} unread messages",
						count: unread,
					})}
				>
					{formatCount(unread)}
				</span>
			)}
		</BoardStatusItem>
	);
});

const PresenceFace = memo(function PresenceFace({
	identity,
	typing,
	away,
}: Readonly<{ identity: PresenceIdentity; typing: boolean; away: boolean }>) {
	return (
		<span
			className={cn("relative inline-flex", away && "opacity-50")}
			title={identity.name}
		>
			<Avatar
				className="size-4 rounded-full"
				style={{ boxShadow: `0 0 0 1px ${identity.color}` }}
			>
				{identity.avatarUrl && (
					<AvatarImage
						src={identity.avatarUrl}
						alt=""
						className="object-cover"
					/>
				)}
				<AvatarFallback
					className="rounded-full text-[8px] font-semibold text-white"
					style={{ background: identity.color }}
				>
					{identity.shortName.charAt(0).toUpperCase()}
				</AvatarFallback>
			</Avatar>
			{typing && (
				<span
					className="absolute -bottom-0.5 -right-0.5 flex size-2 animate-pulse items-center justify-center rounded-full bg-primary text-primary-foreground ring-1 ring-background motion-reduce:animate-none"
					aria-hidden="true"
				>
					<PencilLineIcon className="size-1.5" />
				</span>
			)}
		</span>
	);
});

const PresencePopover = memo(function PresencePopover({
	collaborators,
	peerUsers,
	onlineCount,
	followingSub,
	currentLayerPath,
	layerNames,
	fileLabels,
	resolveNodeName,
	getLastActiveAt,
	isTypingInEditor,
	isTypingInChat,
	isAway,
	onToggleFollow,
	onStopFollowing,
	onJumpToUser,
	onJumpToLayer,
	onFocusNode,
	onOpenInCode,
	onHighlightNodes,
	onOpenChat,
	unread,
	peerScopes,
	onJoinScope,
	onSummon,
	onReact,
}: Readonly<{
	collaborators: PresenceCollaborator[];
	peerUsers: Map<string, PeerUserInfo>;
	onlineCount: number;
	followingSub?: string;
	currentLayerPath: string;
	layerNames?: Map<string, string>;
	fileLabels: Map<string, string>;
	resolveNodeName?: (nodeId: string) => string | undefined;
	getLastActiveAt?: (sub: string) => number | undefined;
	isTypingInEditor?: (sub: string) => boolean;
	isTypingInChat?: (sub: string) => boolean;
	isAway?: (sub: string) => boolean;
	onToggleFollow: (sub: string) => void;
	onStopFollowing?: () => void;
	onJumpToUser: (sub: string) => void;
	onJumpToLayer: (layerPath: string) => void;
	onFocusNode?: (nodeId: string) => void;
	onOpenInCode?: (nodeId: string) => void;
	onHighlightNodes?: (nodeIds?: string[]) => void;
	onOpenChat?: () => void;
	unread: number;
	peerScopes?: Map<string, string[]>;
	onJoinScope?: (nodeIds: string[]) => void;
	onSummon?: () => void;
	onReact?: (emoji: PingEmoji) => void;
}>) {
	const { t } = useTranslation("flow");

	// The popover content only exists while it is open, so this clock — which
	// advances the idle/ago labels and re-reads the typing predicates — runs
	// for nobody else.
	const now = useClock(LIVE_REFRESH_MS);

	const following = followingSub
		? identityOf(followingSub, peerUsers)
		: undefined;
	const nobodyElse = collaborators.every((collab) => collab.self);
	const reactTitle = t("presenceReact", "React at your cursor");

	return (
		<div className="flex flex-col">
			<div className="flex items-center gap-1.5 px-1 py-1">
				<UsersIcon
					className="size-3.5 shrink-0 text-muted-foreground"
					aria-hidden="true"
				/>
				<span className="text-xs font-medium">
					{t("collaborators", "Collaborators")}
				</span>
				<span className="text-[11px] tabular-nums text-muted-foreground">
					{t("countOnline", {
						defaultValue_one: "{{count}} online",
						defaultValue_other: "{{count}} online",
						count: onlineCount,
					})}
				</span>
				<span className="flex-1" />
				{following && (
					<span className="flex max-w-36 items-center gap-1 rounded-full bg-primary/10 py-0.5 pl-1.5 pr-0.5 text-[10px] font-medium text-primary">
						<EyeIcon className="size-3 shrink-0" aria-hidden="true" />
						<span className="truncate">
							{t("followingName", "Following {{name}}", {
								name: following.shortName,
							})}
						</span>
						{onStopFollowing && (
							<button
								type="button"
								onClick={onStopFollowing}
								title={t("stopFollowing", "Stop following")}
								aria-label={t("stopFollowing", "Stop following")}
								className="rounded-full p-0.5 hover:bg-primary/20"
							>
								<XIcon className="size-3" aria-hidden="true" />
							</button>
						)}
					</span>
				)}
				{onSummon && !nobodyElse && (
					<HeaderAction
						label={t("presenceSummon", "Bring everyone here")}
						onClick={onSummon}
					>
						<MegaphoneIcon />
					</HeaderAction>
				)}
				{onOpenChat && (
					<HeaderAction
						label={t("chat", "Chat")}
						onClick={onOpenChat}
						badge={unread > 0 ? formatCount(unread) : undefined}
					>
						<MessageCircleIcon />
					</HeaderAction>
				)}
			</div>
			{onReact && (
				<div className="flex items-center gap-0.5 px-1 pb-1">
					{PING_EMOJI.map((emoji) => (
						<button
							key={emoji}
							type="button"
							onClick={() => onReact(emoji)}
							title={reactTitle}
							className="rounded-sm px-1 py-0.5 text-sm leading-none hover:bg-accent focus-visible:outline-none focus-visible:ring-1 focus-visible:ring-ring"
						>
							{emoji}
						</button>
					))}
				</div>
			)}
			{nobodyElse && (
				<p className="px-2 py-2 text-[11px] text-muted-foreground">
					{t("presenceAlone", "No one else is on this board right now.")}
				</p>
			)}
			{collaborators.length > 0 && (
				<ul className="mt-0.5 flex max-h-72 flex-col overflow-y-auto">
					{collaborators.map((collab) => (
						<CollaboratorRow
							key={collab.sub}
							collab={collab}
							identity={identityOf(collab.sub, peerUsers)}
							activity={describeActivity(collab, {
								currentLayerPath,
								layerNames,
								fileLabels,
								nodeName: resolveNodeName,
								lastActiveAt: getLastActiveAt?.(collab.sub),
								typingInEditor: isTypingInEditor?.(collab.sub),
								typingInChat: isTypingInChat?.(collab.sub),
								away: isAway?.(collab.sub),
								now,
							})}
							isFollowing={followingSub === collab.sub}
							scopeNodeIds={peerScopes?.get(collab.sub)}
							onToggleFollow={onToggleFollow}
							onJumpToUser={onJumpToUser}
							onJumpToLayer={onJumpToLayer}
							onFocusNode={onFocusNode}
							onOpenInCode={onOpenInCode}
							onHighlightNodes={onHighlightNodes}
							onJoinScope={onJoinScope}
						/>
					))}
				</ul>
			)}
		</div>
	);
});

function HeaderAction({
	label,
	onClick,
	badge,
	children,
}: Readonly<{
	label: string;
	onClick: () => void;
	badge?: string;
	children: ReactNode;
}>) {
	return (
		<button
			type="button"
			onClick={onClick}
			title={label}
			aria-label={label}
			className="relative rounded-sm p-1 text-muted-foreground hover:bg-accent hover:text-accent-foreground [&>svg]:size-3.5"
		>
			{children}
			{badge && (
				<span className="absolute -right-0.5 -top-0.5 min-w-3.5 rounded-full bg-primary px-1 text-[9px] font-semibold leading-3.5 tabular-nums text-primary-foreground">
					{badge}
				</span>
			)}
		</button>
	);
}

const CollaboratorRow = memo(function CollaboratorRow({
	collab,
	identity,
	activity,
	isFollowing,
	scopeNodeIds,
	onToggleFollow,
	onJumpToUser,
	onJumpToLayer,
	onFocusNode,
	onOpenInCode,
	onHighlightNodes,
	onJoinScope,
}: Readonly<{
	collab: PresenceCollaborator;
	identity: PresenceIdentity;
	activity: PresenceActivity;
	isFollowing: boolean;
	scopeNodeIds?: string[];
	onToggleFollow: (sub: string) => void;
	onJumpToUser: (sub: string) => void;
	onJumpToLayer: (layerPath: string) => void;
	onFocusNode?: (nodeId: string) => void;
	onOpenInCode?: (nodeId: string) => void;
	onHighlightNodes?: (nodeIds?: string[]) => void;
	onJoinScope?: (nodeIds: string[]) => void;
}>) {
	const { t } = useTranslation("flow");
	const highlightIds = useMemo(() => presenceHighlightIds(collab), [collab]);

	// Tracks whether this row currently owns the canvas highlight so it can
	// hand it back when the row (or the whole popover) goes away mid-hover.
	const highlightingRef = useRef(false);
	const highlight = useCallback(() => {
		if (!onHighlightNodes) return;
		highlightingRef.current = true;
		onHighlightNodes(highlightIds.length > 0 ? highlightIds : undefined);
	}, [onHighlightNodes, highlightIds]);
	const clearHighlight = useCallback(() => {
		if (!highlightingRef.current) return;
		highlightingRef.current = false;
		onHighlightNodes?.(undefined);
	}, [onHighlightNodes]);
	const handleBlur = useCallback(
		(event: FocusEvent<HTMLLIElement>) => {
			if (event.currentTarget.contains(event.relatedTarget as Node | null))
				return;
			clearHighlight();
		},
		[clearHighlight],
	);
	useEffect(() => () => clearHighlight(), [clearHighlight]);

	const displayName =
		collab.self && !identity.known ? t("you", "You") : identity.name;
	const showYouBadge = collab.self && identity.known;
	const layerLabel = activity.layerLabel ?? t("root", "Root");
	const { editing, firstSelectedNodeId, typingInEditor, typingInChat, away } =
		activity;

	return (
		<li
			className="group flex items-start gap-2 rounded-sm px-2 py-1.5 text-xs hover:bg-accent focus-within:bg-accent"
			onMouseEnter={highlight}
			onMouseLeave={clearHighlight}
			onFocus={highlight}
			onBlur={handleBlur}
		>
			<Avatar
				className={cn("size-7 shrink-0 rounded-md", away && "opacity-50")}
				style={{ boxShadow: `0 0 0 2px ${identity.color}` }}
			>
				{identity.avatarUrl && (
					<AvatarImage
						src={identity.avatarUrl}
						alt=""
						className="object-cover"
					/>
				)}
				<AvatarFallback
					className="rounded-md text-[10px] font-semibold text-white"
					style={{ background: identity.color }}
				>
					{displayName.charAt(0).toUpperCase()}
				</AvatarFallback>
			</Avatar>
			<div className="min-w-0 flex-1">
				<div className="flex items-center gap-1.5">
					<span className="truncate font-medium" title={identity.name}>
						{displayName}
					</span>
					{showYouBadge && (
						<Badge
							variant="outline"
							className="shrink-0 px-1 py-0 text-[9px] leading-3.5"
						>
							{t("you", "You")}
						</Badge>
					)}
					{collab.sessions > 1 && (
						<span className="shrink-0 text-[11px] text-muted-foreground">
							·{" "}
							{t("countSessions", {
								defaultValue_one: "{{count}} session",
								defaultValue_other: "{{count}} sessions",
								count: collab.sessions,
							})}
						</span>
					)}
				</div>
				<div className="mt-0.5 flex flex-wrap items-center gap-x-2 gap-y-0.5 text-[11px] leading-tight text-muted-foreground">
					<ActivityChip
						icon={<LayersIcon />}
						title={
							activity.sameLayer
								? undefined
								: t("jumpToLayer", "Go to this layer")
						}
						onClick={
							activity.sameLayer
								? undefined
								: () => onJumpToLayer(activity.layerPath)
						}
					>
						{activity.sameLayer ? t("presenceHere", "here") : layerLabel}
					</ActivityChip>
					{typingInEditor ? (
						<ActivityChip
							icon={
								<PencilLineIcon className="animate-pulse motion-reduce:animate-none" />
							}
							className="text-primary"
						>
							{t("presenceTypingIn", "typing in {{file}}", {
								file: activity.codeFileLabel ?? MAIN_FILE_LABEL,
							})}
						</ActivityChip>
					) : (
						<>
							{activity.codeFileLabel && (
								<ActivityChip icon={<FileCode2Icon />}>
									{activity.codeFileLabel}
								</ActivityChip>
							)}
							{editing && (
								<ActivityChip icon={<PencilLineIcon />}>
									{t("presenceEditing", "editing {{name}}", {
										name: editing.label,
									})}
								</ActivityChip>
							)}
						</>
					)}
					{typingInChat && (
						<ActivityChip
							icon={
								<MessageCircleIcon className="animate-pulse motion-reduce:animate-none" />
							}
							className="text-primary"
						>
							{t("presenceTypingInChat", "typing in chat")}
						</ActivityChip>
					)}
					{activity.selectedCount > 0 && (
						<ActivityChip
							icon={<MousePointerClickIcon />}
							title={
								onFocusNode ? t("revealOnBoard", "Reveal on board") : undefined
							}
							onClick={
								onFocusNode && firstSelectedNodeId
									? () => onFocusNode(firstSelectedNodeId)
									: undefined
							}
						>
							{t("selectedcountSelected", "{{selectedCount}} selected", {
								selectedCount: activity.selectedCount,
							})}
						</ActivityChip>
					)}
					{activity.running && (
						<ActivityChip
							icon={
								<PlayIcon className="animate-pulse motion-reduce:animate-none" />
							}
							className="text-primary"
						>
							{t("presenceRunning", "running")}
						</ActivityChip>
					)}
					{activity.idleMinutes !== undefined && (
						<span className="text-muted-foreground/70">
							{away
								? t("presenceAway", "away {{minutes}}m", {
										minutes: activity.idleMinutes,
									})
								: t("presenceIdleMinutes", "idle {{minutes}}m", {
										minutes: activity.idleMinutes,
									})}
						</span>
					)}
				</div>
				{(activity.lastEdit || activity.lastRun) && (
					<div className="mt-0.5 flex flex-wrap items-center gap-x-2 gap-y-0.5 text-[11px] leading-tight text-muted-foreground/70">
						{activity.lastEdit && (
							<ActivityChip icon={<HistoryIcon />}>
								<LastEditLabel edit={activity.lastEdit} />
							</ActivityChip>
						)}
						{activity.lastRun && (
							<ActivityChip
								icon={<PlayIcon />}
								className={
									activity.lastRun.status === "error"
										? "text-destructive"
										: undefined
								}
							>
								<LastRunLabel run={activity.lastRun} />
							</ActivityChip>
						)}
					</div>
				)}
			</div>
			<div
				className={cn(
					"flex shrink-0 items-center gap-0.5 opacity-0 transition-opacity group-hover:opacity-100 group-focus-within:opacity-100",
					isFollowing && "opacity-100",
				)}
			>
				{!collab.self && (
					<RowAction
						label={
							isFollowing
								? t("stopFollowing", "Stop following")
								: t("follow", "Follow")
						}
						active={isFollowing}
						onClick={() => onToggleFollow(collab.sub)}
					>
						{isFollowing ? <EyeOffIcon /> : <EyeIcon />}
					</RowAction>
				)}
				<RowAction
					label={t("jumpToUser", "Jump to user")}
					onClick={() => onJumpToUser(collab.sub)}
				>
					<NavigationIcon />
				</RowAction>
				{onOpenInCode && editing && (
					<RowAction
						label={t("presenceOpenInCode", "Open in code")}
						onClick={() => onOpenInCode(editing.anchorId)}
					>
						<FileCode2Icon />
					</RowAction>
				)}
				{onJoinScope && scopeNodeIds && scopeNodeIds.length > 0 && (
					<RowAction
						label={t("flowscriptJoinScope", "Join code scope")}
						onClick={() => onJoinScope(scopeNodeIds)}
					>
						<BracesIcon />
					</RowAction>
				)}
			</div>
		</li>
	);
});

type Translate = ReturnType<typeof useTranslation>["t"];

/** Literal keys per verb so the extractor and the typed `t` both see them. */
function editVerbLabel(t: Translate, verb: EditVerb, count: number): string {
	switch (verb) {
		case "added":
			return t("presenceEditAdded", {
				defaultValue_one: "added {{count}}",
				defaultValue_other: "added {{count}}",
				count,
			});
		case "moved":
			return t("presenceEditMoved", {
				defaultValue_one: "moved {{count}}",
				defaultValue_other: "moved {{count}}",
				count,
			});
		case "connected":
			return t("presenceEditConnected", {
				defaultValue_one: "connected {{count}}",
				defaultValue_other: "connected {{count}}",
				count,
			});
		case "disconnected":
			return t("presenceEditDisconnected", {
				defaultValue_one: "disconnected {{count}}",
				defaultValue_other: "disconnected {{count}}",
				count,
			});
		case "removed":
			return t("presenceEditRemoved", {
				defaultValue_one: "removed {{count}}",
				defaultValue_other: "removed {{count}}",
				count,
			});
		case "updated":
			return t("presenceEditUpdated", {
				defaultValue_one: "updated {{count}}",
				defaultValue_other: "updated {{count}}",
				count,
			});
		case "commented":
			return t("presenceEditCommented", {
				defaultValue_one: "commented {{count}}",
				defaultValue_other: "commented {{count}}",
				count,
			});
		case "layered":
			return t("presenceEditLayered", {
				defaultValue_one: "changed {{count}} layer",
				defaultValue_other: "changed {{count}} layers",
				count,
			});
		case "variables":
			return t("presenceEditVariables", {
				defaultValue_one: "changed {{count}} variable",
				defaultValue_other: "changed {{count}} variables",
				count,
			});
	}
}

/** "moved 3 · 2m ago": one verb per command kind in the batch, then its age. */
function LastEditLabel({ edit }: Readonly<{ edit: PresenceLastEdit }>) {
	const { t } = useTranslation("flow");
	const parts = edit.verbs.map((verb) => editVerbLabel(t, verb, edit.count));
	parts.push(agoLabel(t, edit.agoMinutes));
	return <>{parts.join(" · ")}</>;
}

/** "run ok · 12 nodes · 5m ago", in `text-destructive` when the run failed. */
function LastRunLabel({ run }: Readonly<{ run: PresenceLastRun }>) {
	const { t } = useTranslation("flow");
	const outcome =
		run.status === "ok"
			? t("presenceRunOk", {
					defaultValue_one: "run ok · {{count}} node",
					defaultValue_other: "run ok · {{count}} nodes",
					count: run.executed,
				})
			: t("presenceRunFailed", {
					defaultValue_one: "run failed · {{count}} node",
					defaultValue_other: "run failed · {{count}} nodes",
					count: run.executed,
				});
	return <>{[outcome, agoLabel(t, run.agoMinutes)].join(" · ")}</>;
}

function agoLabel(
	t: ReturnType<typeof useTranslation>["t"],
	minutes: number,
): string {
	return minutes === 0
		? t("presenceJustNow", "just now")
		: t("presenceAgoMinutes", "{{minutes}}m ago", { minutes });
}

function ActivityChip({
	icon,
	title,
	onClick,
	className,
	children,
}: Readonly<{
	icon: ReactNode;
	title?: string;
	onClick?: () => void;
	className?: string;
	children: ReactNode;
}>) {
	const classes = cn(
		"inline-flex min-w-0 items-center gap-1 [&>svg]:size-3 [&>svg]:shrink-0",
		className,
	);
	if (!onClick) {
		return (
			<span className={classes} title={title}>
				{icon}
				<span className="truncate">{children}</span>
			</span>
		);
	}
	return (
		<button
			type="button"
			onClick={onClick}
			title={title}
			className={cn(
				classes,
				"rounded-sm underline-offset-2 hover:text-foreground hover:underline focus-visible:outline-none focus-visible:ring-1 focus-visible:ring-ring",
			)}
		>
			{icon}
			<span className="truncate">{children}</span>
		</button>
	);
}

function RowAction({
	label,
	active,
	onClick,
	children,
}: Readonly<{
	label: string;
	active?: boolean;
	onClick: () => void;
	children: ReactNode;
}>) {
	return (
		<button
			type="button"
			onClick={onClick}
			title={label}
			aria-label={label}
			aria-pressed={active}
			className={cn(
				"rounded-sm p-1 text-muted-foreground hover:bg-background/70 hover:text-foreground focus-visible:outline-none focus-visible:ring-1 focus-visible:ring-ring [&>svg]:size-3.5",
				active && "text-primary",
			)}
		>
			{children}
		</button>
	);
}
