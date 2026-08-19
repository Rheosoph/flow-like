"use client";

import { useDroppable } from "@dnd-kit/core";
import { useTranslation } from "@flow-like/locales";
import { ChevronDownIcon } from "lucide-react";
import type { ReactNode } from "react";
import { useCallback, useMemo, useRef, useState } from "react";
import { cn } from "../../../lib/utils";
import { compareByNameThenId, folderDroppableId } from "../category-tree";
import {
	type IFolderNode,
	type IGroupMode,
	type ITokenGroup,
	type ITokenItem,
	type ITokenQuery,
	buildFolderTree,
	groupFlat,
	isQueryEmpty,
} from "./model";

/** Header height, in px — the stride the nested sticky headers stack on. */
const HEADER_H = 24;
/** One indent step per folder level. Cheap horizontally, which is the point. */
const INDENT = 10;

interface IGroupHeaderProps {
	label: string;
	count: number;
	depth: number;
	open: boolean;
	dropPath: string | null;
	kind: string;
	tone?: "warn";
	onToggle: () => void;
}

function GroupHeader({
	label,
	count,
	depth,
	open,
	dropPath,
	kind,
	tone,
	onToggle,
}: Readonly<IGroupHeaderProps>) {
	// Only folders accept drops; a "by type" bucket is not a place to file into.
	// A non-folder header still needs a unique id — dnd-kit keys its registry on
	// it, so every flat group sharing one id collides even while disabled.
	const { setNodeRef, isOver } = useDroppable({
		id:
			dropPath === null
				? `not-a-folder:${kind}:${label}`
				: folderDroppableId(kind, dropPath),
		disabled: dropPath === null,
	});

	return (
		<button
			ref={dropPath === null ? undefined : setNodeRef}
			type="button"
			onClick={onToggle}
			aria-expanded={open}
			className={cn(
				"sticky flex w-full items-center gap-1.5 border-b border-border/60 bg-card pr-2 text-left transition-colors",
				"hover:bg-muted/60",
				isOver && "bg-primary/15 ring-1 ring-primary/50 ring-inset",
			)}
			style={{
				top: depth * HEADER_H,
				zIndex: 20 - depth,
				height: HEADER_H,
				paddingLeft: 8 + depth * INDENT,
			}}
		>
			<ChevronDownIcon
				className={cn(
					"h-3 w-3 shrink-0 text-muted-foreground transition-transform",
					!open && "-rotate-90",
				)}
			/>
			<span
				className={cn(
					"min-w-0 flex-1 truncate font-mono text-[9.5px] uppercase tracking-[0.14em]",
					tone === "warn" ? "text-destructive" : "text-muted-foreground",
				)}
			>
				{label}
			</span>
			<span
				className={cn(
					"rounded-full bg-muted px-1.5 font-mono text-[9.5px] leading-[14px] tabular-nums",
					tone === "warn"
						? "bg-destructive/15 text-destructive"
						: "text-muted-foreground",
				)}
			>
				{count}
			</span>
		</button>
	);
}

function TokenRow({
	items,
	depth,
	renderToken,
	focusedId,
}: Readonly<{
	items: ITokenItem[];
	depth: number;
	renderToken: (item: ITokenItem, focused: boolean) => ReactNode;
	focusedId: string | null;
}>) {
	if (items.length === 0) return null;
	return (
		<div
			className="flex flex-wrap gap-x-2 gap-y-2.5 border-l border-border/50 py-2.5 pr-3"
			style={{ marginLeft: 8 + depth * INDENT, paddingLeft: INDENT }}
		>
			{items.map((item) => renderToken(item, focusedId === item.id))}
		</div>
	);
}

function FolderSection({
	node,
	kind,
	collapsed,
	forceOpen,
	onToggle,
	renderToken,
	focusedId,
}: Readonly<{
	node: IFolderNode;
	kind: string;
	collapsed: Record<string, boolean>;
	forceOpen: boolean;
	onToggle: (key: string) => void;
	renderToken: (item: ITokenItem, focused: boolean) => ReactNode;
	focusedId: string | null;
}>) {
	const key = `folder:${node.path}`;
	const open = forceOpen || !collapsed[key];

	return (
		<section>
			<GroupHeader
				label={node.name}
				count={node.total}
				depth={node.depth}
				open={open}
				dropPath={node.path}
				kind={kind}
				onToggle={() => onToggle(key)}
			/>
			{open && (
				<>
					<TokenRow
						items={node.items}
						depth={node.depth}
						renderToken={renderToken}
						focusedId={focusedId}
					/>
					{node.children.map((child) => (
						<FolderSection
							key={child.path}
							node={child}
							kind={kind}
							collapsed={collapsed}
							forceOpen={forceOpen}
							onToggle={onToggle}
							renderToken={renderToken}
							focusedId={focusedId}
						/>
					))}
				</>
			)}
		</section>
	);
}

export interface ITokenBoardProps {
	items: ITokenItem[];
	/** Droppable namespace — `variables`, `local-variables` or `functions`. */
	kind: string;
	group: IGroupMode;
	query: ITokenQuery;
	renderToken: (item: ITokenItem, focused: boolean) => ReactNode;
	/** Shown when nothing matches. */
	empty: ReactNode;
	/**
	 * A section pinned above the groups — the local scope of the function you are
	 * standing in, which must not be filed into the board's folder tree.
	 */
	lead?: { label: string; items: ITokenItem[] };
	/** Rendered above the groups, inside the same scroller. */
	children?: ReactNode;
}

/**
 * The wrapping board of typed tokens.
 *
 * Folders nest properly: each level is an independently sticky header stacked at
 * `depth * HEADER_H`, so scrolling deep into `Feedback/State/Filters` keeps every
 * ancestor pinned above you, and the body indents by a rail instead of a box.
 * While a filter is active every group force-opens — a collapsed folder must
 * never hide a match.
 */
export function TokenBoard({
	items,
	kind,
	group,
	query,
	renderToken,
	empty,
	lead,
	children,
}: Readonly<ITokenBoardProps>) {
	const { t } = useTranslation("flow");
	const [collapsed, setCollapsed] = useState<Record<string, boolean>>({});
	const [focusedId, setFocusedId] = useState<string | null>(null);
	const boardRef = useRef<HTMLDivElement | null>(null);

	const filtering = !isQueryEmpty(query);
	const { setNodeRef: setRootRef, isOver: isOverRoot } = useDroppable({
		id: folderDroppableId(kind, ""),
	});

	const toggle = useCallback((key: string) => {
		setCollapsed((prev) => ({ ...prev, [key]: !prev[key] }));
	}, []);

	const tree = useMemo(
		() => (group === "folder" ? buildFolderTree(items) : null),
		[group, items],
	);

	const flatGroups: ITokenGroup[] = useMemo(() => {
		if (group === "folder") return [];
		return groupFlat(items, group, {
			usage: {
				unused: t("unusedSafeToDelete", "Unused — safe to delete"),
				hot: t("heavilyUsed8", "Heavily used · 8+"),
				warm: t("used37", "Used · 3–7"),
				cold: t("rarelyUsed12", "Rarely used · 1–2"),
			},
			local: t("localToThisFunction", "Local to this function"),
			board: t("boardGlobals", "Board globals"),
			function: t("functions", "Functions"),
		});
	}, [group, items, t]);

	const rootItems = useMemo(
		() => (tree ? [...tree.items].sort(compareByNameThenId) : []),
		[tree],
	);

	/** Arrow keys roam the board; the wrap means up/down has to be geometric. */
	const onKeyDown = useCallback((event: React.KeyboardEvent) => {
		if (
			event.key !== "ArrowLeft" &&
			event.key !== "ArrowRight" &&
			event.key !== "ArrowUp" &&
			event.key !== "ArrowDown"
		)
			return;
		const host = boardRef.current;
		if (!host) return;
		const tokens = Array.from(
			host.querySelectorAll<HTMLElement>("[data-token-id]"),
		);
		if (tokens.length === 0) return;

		const active = document.activeElement as HTMLElement | null;
		const current = tokens.find((token) => token.contains(active)) ?? tokens[0];
		event.preventDefault();

		let next: HTMLElement | undefined;
		if (event.key === "ArrowLeft" || event.key === "ArrowRight") {
			const index = tokens.indexOf(current);
			next = tokens[index + (event.key === "ArrowRight" ? 1 : -1)];
		} else {
			const rect = current.getBoundingClientRect();
			const centre = rect.left + rect.width / 2;
			const down = event.key === "ArrowDown";
			let best = Number.POSITIVE_INFINITY;
			for (const token of tokens) {
				const box = token.getBoundingClientRect();
				if (down ? box.top <= rect.top + 2 : box.top >= rect.top - 2) continue;
				const distance =
					Math.abs(box.top - rect.top) * 3 +
					Math.abs(box.left + box.width / 2 - centre);
				if (distance < best) {
					best = distance;
					next = token;
				}
			}
		}

		if (!next) return;
		const id = next.dataset.tokenId;
		if (id) setFocusedId(id);
		next.querySelector<HTMLElement>("button")?.focus();
	}, []);

	const isEmpty =
		items.length === 0 &&
		!lead?.items.length &&
		(group !== "folder" || (rootItems.length === 0 && !tree?.children.length));

	return (
		<div
			ref={(element) => {
				boardRef.current = element;
				setRootRef(element);
			}}
			className={cn(
				"min-h-0 flex-1 overflow-y-auto overflow-x-hidden bg-card",
				isOverRoot && "ring-1 ring-primary/40 ring-inset",
			)}
			onKeyDown={onKeyDown}
		>
			{children}
			{lead && lead.items.length > 0 && (
				<section>
					<GroupHeader
						label={lead.label}
						count={lead.items.length}
						depth={0}
						open={filtering || !collapsed.lead}
						dropPath={null}
						kind={kind}
						onToggle={() => toggle("lead")}
					/>
					{(filtering || !collapsed.lead) && (
						<TokenRow
							items={lead.items}
							depth={0}
							renderToken={renderToken}
							focusedId={focusedId}
						/>
					)}
				</section>
			)}
			{isEmpty ? (
				empty
			) : group === "folder" && tree ? (
				<>
					<TokenRow
						items={rootItems}
						depth={0}
						renderToken={renderToken}
						focusedId={focusedId}
					/>
					{tree.children.map((child) => (
						<FolderSection
							key={child.path}
							node={child}
							kind={kind}
							collapsed={collapsed}
							forceOpen={filtering}
							onToggle={toggle}
							renderToken={renderToken}
							focusedId={focusedId}
						/>
					))}
				</>
			) : (
				flatGroups.map((bucket) => {
					const open = filtering || !collapsed[bucket.key];
					return (
						<section key={bucket.key}>
							<GroupHeader
								label={bucket.label}
								count={bucket.items.length}
								depth={0}
								open={open}
								dropPath={bucket.dropPath}
								kind={kind}
								tone={bucket.tone}
								onToggle={() => toggle(bucket.key)}
							/>
							{open && (
								<TokenRow
									items={bucket.items}
									depth={0}
									renderToken={renderToken}
									focusedId={focusedId}
								/>
							)}
						</section>
					);
				})
			)}
			<div className="h-16" />
		</div>
	);
}
