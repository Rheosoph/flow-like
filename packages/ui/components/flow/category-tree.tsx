"use client";

/**
 * Sidebar folders for anything that carries a "/"-separated `category`
 * (variables, function layers, …). One tree, one drag-and-drop contract.
 */

import { useDroppable } from "@dnd-kit/core";
import { ChevronDown, ChevronRight, FolderIcon } from "lucide-react";
import type { ReactNode } from "react";
import { Fragment, useCallback, useMemo, useState } from "react";

export interface ICategorized {
	id: string;
	name: string;
	category?: string | null;
}

export interface ICategoryNode<T> {
	name: string;
	path: string;
	items: T[];
	children: Record<string, ICategoryNode<T>>;
}

/** Dispatched on `document` when something is dropped onto a folder or onto a tree root. */
export const FOLDER_DROP_EVENT = "flow-folder-drop";

export interface IFolderDropDetail<T = unknown> {
	/** Identifies the tree that owns the drop target — see {@link folderDroppableId}. */
	kind: string;
	/** Target category, `""` for the top level. */
	path: string;
	item: T;
	/**
	 * Set by the first listener that acts on the drop. Several trees of the same kind can be
	 * mounted at once (desktop panel + mobile sheet); only one of them may apply the move.
	 */
	consumed?: boolean;
}

export const folderDroppableId = (kind: string, path: string) =>
	`folder:${kind}:${path}`;

export function parseFolderDroppableId(
	droppableId: string,
): { kind: string; path: string } | null {
	if (!droppableId.startsWith("folder:")) return null;
	const rest = droppableId.slice("folder:".length);
	const separator = rest.indexOf(":");
	if (separator < 0) return null;
	return { kind: rest.slice(0, separator), path: rest.slice(separator + 1) };
}

export const categorySegments = (category?: string | null): string[] =>
	(category ?? "")
		.split("/")
		.map((segment) => segment.trim())
		.filter(Boolean);

/** Normalized category, or `undefined` for the top level. */
export const normalizeCategory = (
	category?: string | null,
): string | undefined => {
	const segments = categorySegments(category);
	return segments.length > 0 ? segments.join("/") : undefined;
};

/**
 * Total order that survives a reload — object iteration order does not, so ties on name
 * fall back to the id.
 */
export const compareByNameThenId = (a: ICategorized, b: ICategorized): number =>
	a.name.localeCompare(b.name) || a.id.localeCompare(b.id);

export function buildCategoryTree<T extends ICategorized>(
	items: T[],
): ICategoryNode<T> {
	const root: ICategoryNode<T> = {
		name: "",
		path: "",
		items: [],
		children: {},
	};

	for (const item of items) {
		let node = root;
		let path = "";
		for (const segment of categorySegments(item.category)) {
			path = path ? `${path}/${segment}` : segment;
			if (!node.children[segment])
				node.children[segment] = {
					name: segment,
					path,
					items: [],
					children: {},
				};
			node = node.children[segment];
		}
		node.items.push(item);
	}

	return root;
}

const countRecursive = <T,>(node: ICategoryNode<T>): number =>
	node.items.length +
	Object.values(node.children).reduce((sum, c) => sum + countRecursive(c), 0);

/** Every folder path in the tree, depth-first and sorted. */
export function collectFolderPaths<T>(node: ICategoryNode<T>): string[] {
	return Object.keys(node.children)
		.sort((a, b) => a.localeCompare(b))
		.flatMap((key) => [
			node.children[key].path,
			...collectFolderPaths(node.children[key]),
		]);
}

const sortedChildKeys = <T,>(node: ICategoryNode<T>) =>
	Object.keys(node.children).sort((a, b) => a.localeCompare(b));

export function CategoryTree<T extends ICategorized>({
	root,
	kind,
	renderItem,
}: Readonly<{
	root: ICategoryNode<T>;
	kind: string;
	renderItem: (item: T) => ReactNode;
}>) {
	const [open, setOpen] = useState<Record<string, boolean>>({});
	const isOpen = useCallback((path: string) => open[path] ?? true, [open]);
	const toggle = useCallback((path: string) => {
		setOpen((prev) => ({ ...prev, [path]: !(prev[path] ?? true) }));
	}, []);

	const { setNodeRef, isOver } = useDroppable({
		id: folderDroppableId(kind, ""),
	});

	const childKeys = useMemo(() => sortedChildKeys(root), [root]);
	const items = useMemo(
		() => [...root.items].sort(compareByNameThenId),
		[root.items],
	);

	return (
		<div
			ref={setNodeRef}
			className={`space-y-2 rounded-md ${isOver ? "ring-1 ring-primary/40" : ""}`}
		>
			{items.length > 0 && (
				<div className="flex flex-col gap-2">
					{items.map((item) => (
						<Fragment key={item.id}>{renderItem(item)}</Fragment>
					))}
				</div>
			)}
			{childKeys.length > 0 && (
				<div className="space-y-2">
					{childKeys.map((key) => (
						<FolderNode
							key={root.children[key].path}
							node={root.children[key]}
							kind={kind}
							isOpen={isOpen}
							toggle={toggle}
							renderItem={renderItem}
						/>
					))}
				</div>
			)}
		</div>
	);
}

function FolderNode<T extends ICategorized>({
	node,
	kind,
	isOpen,
	toggle,
	renderItem,
}: Readonly<{
	node: ICategoryNode<T>;
	kind: string;
	isOpen: (path: string) => boolean;
	toggle: (path: string) => void;
	renderItem: (item: T) => ReactNode;
}>) {
	const { setNodeRef, isOver } = useDroppable({
		id: folderDroppableId(kind, node.path),
	});
	const childKeys = useMemo(() => sortedChildKeys(node), [node]);
	const items = useMemo(
		() => [...node.items].sort(compareByNameThenId),
		[node.items],
	);
	const total = countRecursive(node);

	return (
		<div className="rounded-md border">
			<button
				ref={setNodeRef}
				type="button"
				className={`w-full flex items-center gap-2 px-2 py-2 hover:bg-accent/50 ${isOver ? "bg-primary/5" : ""}`}
				onClick={() => toggle(node.path)}
			>
				{isOpen(node.path) ? (
					<ChevronDown className="h-4 w-4 text-muted-foreground" />
				) : (
					<ChevronRight className="h-4 w-4 text-muted-foreground" />
				)}
				<FolderIcon className="h-4 w-4 text-muted-foreground" />
				<span className="text-sm font-medium">{node.name}</span>
				<span className="ml-auto text-xs text-muted-foreground">{total}</span>
			</button>

			{isOpen(node.path) && (
				<div className="p-2 pt-1 space-y-2">
					{items.map((item) => (
						<Fragment key={item.id}>{renderItem(item)}</Fragment>
					))}
					{childKeys.length > 0 && (
						<div className="mt-2 space-y-2">
							{childKeys.map((key) => (
								<FolderNode
									key={node.children[key].path}
									node={node.children[key]}
									kind={kind}
									isOpen={isOpen}
									toggle={toggle}
									renderItem={renderItem}
								/>
							))}
						</div>
					)}
				</div>
			)}
		</div>
	);
}
