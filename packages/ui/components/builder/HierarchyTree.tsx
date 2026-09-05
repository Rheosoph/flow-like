"use client";

import { useDraggable, useDroppable } from "@dnd-kit/core";
import { useTranslation } from "@flow-like/locales";
import {
	ChevronDown,
	ChevronRight,
	Copy,
	Eye,
	EyeOff,
	FolderPlus,
	GripVertical,
	Lock,
	Scissors,
	Search,
	Trash,
	Unlock,
} from "lucide-react";
import { useCallback, useEffect, useMemo, useRef, useState } from "react";
import { cn } from "../../lib/utils";
import {
	ContextMenu,
	ContextMenuContent,
	ContextMenuItem,
	ContextMenuSeparator,
	ContextMenuTrigger,
} from "../ui/context-menu";
import { Input } from "../ui/input";
import { ScrollArea } from "../ui/scroll-area";
import { useBuilder } from "./BuilderContext";
import {
	COMPONENT_MOVE_TYPE,
	type ComponentMoveData,
	type DropData,
} from "./BuilderDndContext";
import { CONTAINER_TYPES, ROOT_ID } from "./WidgetBuilder";
import {
	canAcceptComponentChildren,
	canReorderComponent,
	findComponentParent,
	getComponentChildren,
	getExplicitChildren,
} from "./componentTree";

interface TreeNodeData {
	id: string;
	type: string;
	children: TreeNodeData[];
	locked?: boolean;
	hidden?: boolean;
}

export interface HierarchyTreeProps {
	className?: string;
	rootComponents?: string[];
}

export function HierarchyTree({
	className,
	rootComponents = [],
}: HierarchyTreeProps) {
	const { t } = useTranslation("flow");
	const [searchQuery, setSearchQuery] = useState("");
	const [expandedNodes, setExpandedNodes] = useState<Set<string>>(new Set());
	const [lockedNodes, setLockedNodes] = useState<Set<string>>(new Set());

	const {
		components,
		selection,
		selectComponent,
		deleteComponents,
		copy,
		cut,
		paste,
		getComponent,
		hiddenComponents,
		toggleComponentVisibility,
	} = useBuilder();

	const findParentId = useCallback(
		(childId: string) => findComponentParent(components, childId),
		[components],
	);

	// Build tree from components
	const tree = useMemo(() => {
		const buildTree = (
			componentId: string,
			ancestors = new Set<string>(),
		): TreeNodeData | null => {
			if (ancestors.has(componentId)) return null;
			const component = getComponent(componentId);
			if (!component || !component.component) return null;

			const childIds = getComponentChildren(component);

			// Widget instances are leaf nodes - they reference widget definitions from widgetRefs
			// We don't traverse into their internal structure
			if (component.component.type === "widgetInstance") {
				return {
					id: componentId,
					type: component.component.type,
					children: [],
					locked: lockedNodes.has(componentId),
					hidden: hiddenComponents.has(componentId),
				};
			}

			return {
				id: componentId,
				type: component.component.type,
				children: childIds
					.map((id) => buildTree(id, new Set(ancestors).add(componentId)))
					.filter((n): n is TreeNodeData => n !== null),
				locked: lockedNodes.has(componentId),
				hidden: hiddenComponents.has(componentId),
			};
		};

		// If no root components specified, build from all top-level components
		if (rootComponents.length === 0) {
			// Find components that aren't children of any other component
			const childIds = new Set<string>();
			for (const comp of components.values()) {
				for (const id of getComponentChildren(comp)) childIds.add(id);
			}

			const roots: TreeNodeData[] = [];
			const addedRoots = new Set<string>(); // Track added roots to prevent duplicates
			for (const comp of components.values()) {
				if (!childIds.has(comp.id) && !addedRoots.has(comp.id)) {
					const node = buildTree(comp.id);
					if (node) {
						roots.push(node);
						addedRoots.add(comp.id);
					}
				}
			}
			return roots;
		}

		return rootComponents
			.map((id) => buildTree(id))
			.filter((n): n is TreeNodeData => n !== null);
	}, [components, rootComponents, getComponent, lockedNodes, hiddenComponents]);

	// Filter tree based on search
	const filteredTree = useMemo(() => {
		if (!searchQuery.trim()) return tree;

		const query = searchQuery.toLowerCase();
		const filterNode = (node: TreeNodeData): TreeNodeData | null => {
			const matchesSearch =
				node.id.toLowerCase().includes(query) ||
				node.type.toLowerCase().includes(query);

			const filteredChildren = node.children
				.map((child) => filterNode(child))
				.filter((n): n is TreeNodeData => n !== null);

			if (matchesSearch || filteredChildren.length > 0) {
				return { ...node, children: filteredChildren };
			}
			return null;
		};

		return tree
			.map((node) => filterNode(node))
			.filter((n): n is TreeNodeData => n !== null);
	}, [tree, searchQuery]);

	// Find path from root to a component (returns all ancestor IDs including the component)
	const findPathToComponent = useCallback(
		(targetId: string): string[] => {
			const findInTree = (
				nodes: TreeNodeData[],
				path: string[],
			): string[] | null => {
				for (const node of nodes) {
					if (node.id === targetId) {
						return [...path, node.id];
					}
					const found = findInTree(node.children, [...path, node.id]);
					if (found) return found;
				}
				return null;
			};
			return findInTree(tree, []) ?? [];
		},
		[tree],
	);

	// Reparenting changes the selected component's path without changing its ID.
	const lastExpandedPathRef = useRef<string | null>(null);

	// Auto-expand tree when a component is selected on canvas
	useEffect(() => {
		if (selection.componentIds.length === 0) {
			lastExpandedPathRef.current = null;
			return;
		}

		// Get the first selected component
		const selectedId = selection.componentIds[0];

		const path = findPathToComponent(selectedId);
		const pathKey = JSON.stringify(path);
		if (lastExpandedPathRef.current === pathKey) return;
		lastExpandedPathRef.current = pathKey;

		if (path.length > 1) {
			// Check if we actually need to expand any nodes
			const nodesToExpand = path.slice(0, -1); // All except the selected component

			setExpandedNodes((prev) => {
				const hasNewNodes = nodesToExpand.some((id) => !prev.has(id));
				if (!hasNewNodes) return prev; // Return same reference if no changes

				const next = new Set(prev);
				for (const id of nodesToExpand) {
					next.add(id);
				}
				return next;
			});
		}
	}, [selection.componentIds, findPathToComponent]);

	const toggleExpand = useCallback((nodeId: string) => {
		setExpandedNodes((prev) => {
			const next = new Set(prev);
			if (next.has(nodeId)) {
				next.delete(nodeId);
			} else {
				next.add(nodeId);
			}
			return next;
		});
	}, []);

	const toggleLock = useCallback((nodeId: string) => {
		setLockedNodes((prev) => {
			const next = new Set(prev);
			if (next.has(nodeId)) {
				next.delete(nodeId);
			} else {
				next.add(nodeId);
			}
			return next;
		});
	}, []);

	const handleSelect = useCallback(
		(nodeId: string, event: React.MouseEvent | React.KeyboardEvent) => {
			selectComponent(nodeId, event.shiftKey || event.metaKey || event.ctrlKey);
		},
		[selectComponent],
	);

	return (
		<div
			className={cn(
				"flex min-w-0 flex-col h-full bg-background border-r overflow-hidden",
				className,
			)}
		>
			<div className="p-3 border-b shrink-0">
				<div className="relative">
					<Search className="absolute left-2.5 top-2.5 h-4 w-4 text-muted-foreground" />
					<Input
						placeholder={t("searchTree", "Search tree...")}
						value={searchQuery}
						onChange={(e) => setSearchQuery(e.target.value)}
						className="pl-8 h-8"
					/>
				</div>
			</div>

			<ScrollArea className="flex-1 min-h-0 min-w-0 [&_[data-radix-scroll-area-viewport]>div]:block!">
				<div
					className="p-2"
					role="tree"
					aria-label={t("hierarchy", "Hierarchy")}
				>
					{filteredTree.length === 0 ? (
						<div className="text-sm text-muted-foreground text-center py-4">
							{t("noComponents", "No components")}
						</div>
					) : (
						filteredTree.map((node) => (
							<TreeNode
								key={node.id}
								node={node}
								depth={0}
								isSelected={selection.componentIds.includes(node.id)}
								onToggleExpand={toggleExpand}
								onToggleLock={toggleLock}
								onToggleHidden={toggleComponentVisibility}
								onSelect={handleSelect}
								onDelete={(id) => deleteComponents([id])}
								onCopy={copy}
								onCut={cut}
								onPaste={paste}
								expandedNodes={expandedNodes}
								selection={selection.componentIds}
								findParentId={findParentId}
							/>
						))
					)}
				</div>
			</ScrollArea>
		</div>
	);
}

interface TreeNodeProps {
	node: TreeNodeData;
	depth: number;
	isSelected: boolean;
	expandedNodes: Set<string>;
	selection: string[];
	onToggleExpand: (id: string) => void;
	onToggleLock: (id: string) => void;
	onToggleHidden: (id: string) => void;
	onSelect: (id: string, event: React.MouseEvent | React.KeyboardEvent) => void;
	onDelete: (id: string) => void;
	onCopy: (ids?: string[]) => void;
	onCut: (ids?: string[]) => void;
	onPaste: (parentId?: string) => void;
	findParentId: (childId: string) => string | null;
}

function TreeNode({
	node,
	depth,
	isSelected,
	expandedNodes,
	selection,
	onToggleExpand,
	onToggleLock,
	onToggleHidden,
	onSelect,
	onDelete,
	onCopy,
	onCut,
	onPaste,
	findParentId,
}: TreeNodeProps) {
	const { t } = useTranslation("flow");
	const { components, getWidgetRef } = useBuilder();
	const hasChildren = node.children.length > 0;
	const isNodeExpanded = expandedNodes.has(node.id);
	const isContainer = CONTAINER_TYPES.has(node.type);
	const isRoot = node.id === ROOT_ID;
	const canReceiveChildren = canAcceptComponentChildren(
		components.get(node.id),
	);
	const canDrag = !node.locked && canReorderComponent(components, node.id);

	const parentId = findParentId(node.id);
	const siblingIndex = parentId
		? getExplicitChildren(components.get(parentId)).indexOf(node.id)
		: -1;

	// Draggable for moving this node
	const {
		attributes: dragAttributes,
		listeners: dragListeners,
		setNodeRef: setDragRef,
		isDragging,
	} = useDraggable({
		id: `tree-move-${node.id}`,
		disabled: isRoot || !canDrag,
		data: {
			type: COMPONENT_MOVE_TYPE,
			componentId: node.id,
			currentParentId: parentId,
		} satisfies ComponentMoveData,
	});

	// Droppable for receiving components
	const { setNodeRef: setDropRef, isOver } = useDroppable({
		id: `tree-drop-${node.id}`,
		disabled: !canReceiveChildren,
		data: {
			type: "container",
			parentId: node.id,
			isContainer: true,
		} satisfies DropData,
	});

	const { setNodeRef: setBeforeRef, isOver: isOverBefore } = useDroppable({
		id: `tree-before-${node.id}`,
		disabled: isRoot || siblingIndex < 0,
		data: {
			type: "drop-zone",
			parentId: parentId ?? "",
			index: siblingIndex,
		} satisfies DropData,
	});
	const { setNodeRef: setAfterRef, isOver: isOverAfter } = useDroppable({
		id: `tree-after-${node.id}`,
		disabled: isRoot || siblingIndex < 0,
		data: {
			type: "drop-zone",
			parentId: parentId ?? "",
			index: siblingIndex + 1,
		} satisfies DropData,
	});

	return (
		<ContextMenu>
			<ContextMenuTrigger asChild>
				<div
					id={`tree-node-${node.id}`}
					role="treeitem"
					// biome-ignore lint/a11y/noNoninteractiveTabindex: Tree items are keyboard-focusable selection controls.
					tabIndex={0}
					aria-selected={isSelected}
					aria-expanded={hasChildren ? isNodeExpanded : undefined}
					onKeyDown={(event) => {
						if (event.key === "Enter" || event.key === " ") {
							event.preventDefault();
							event.stopPropagation();
							onSelect(node.id, event);
						}
					}}
					ref={(el) => {
						setDropRef(el);
					}}
					className={cn(
						"group relative flex min-w-0 max-w-full items-center gap-1 px-2 py-1 rounded text-sm cursor-pointer hover:bg-muted transition-colors",
						isSelected && "bg-primary/10 text-primary",
						node.hidden && "opacity-50",
						isDragging && "opacity-40",
						isOver && canReceiveChildren && "bg-primary/20 ring-1 ring-primary",
					)}
					style={{ paddingLeft: `min(${depth * 16 + 8}px, 40%)` }}
					title={node.id}
					onClick={(e) => {
						e.stopPropagation();
						onSelect(node.id, e);
					}}
				>
					{!isRoot && siblingIndex >= 0 && (
						<>
							<span
								ref={setBeforeRef}
								className={cn(
									"pointer-events-none absolute inset-x-0 top-0",
									canReceiveChildren ? "h-1/4" : "h-1/2",
								)}
							>
								{isOverBefore && (
									<span className="absolute inset-x-0 top-0 h-0.5 bg-primary" />
								)}
							</span>
							<span
								ref={setAfterRef}
								className={cn(
									"pointer-events-none absolute inset-x-0 bottom-0",
									canReceiveChildren ? "h-1/4" : "h-1/2",
								)}
							>
								{isOverAfter && (
									<span className="absolute inset-x-0 bottom-0 h-0.5 bg-primary" />
								)}
							</span>
						</>
					)}
					{/* Drag handle */}
					{!isRoot && canDrag && (
						<div
							ref={setDragRef}
							{...dragListeners}
							{...dragAttributes}
							aria-label={t("reorderComponent", "Reorder {{id}}", {
								id: node.id,
							})}
							className="shrink-0 cursor-grab hover:bg-muted-foreground/10 rounded p-0.5 touch-none"
							onClick={(e) => e.stopPropagation()}
						>
							<GripVertical className="h-3 w-3 text-muted-foreground" />
						</div>
					)}

					{/* Expand toggle */}
					<button
						type="button"
						className={cn(
							"shrink-0 p-0.5 hover:bg-muted-foreground/10 rounded",
							!hasChildren && "invisible",
						)}
						onClick={(e) => {
							e.stopPropagation();
							onToggleExpand(node.id);
						}}
					>
						{isNodeExpanded ? (
							<ChevronDown className="h-3 w-3" />
						) : (
							<ChevronRight className="h-3 w-3" />
						)}
					</button>

					{/* Component type/icon */}
					<span className="min-w-0 truncate flex-1">
						{node.type === "widgetInstance"
							? (() => {
									const comp = components.get(node.id);
									const instanceId = comp
										? (comp.component as unknown as { instanceId?: string })
												.instanceId
										: undefined;
									const widgetDef = instanceId
										? getWidgetRef(instanceId)
										: undefined;
									return widgetDef?.name ?? "Widget";
								})()
							: node.type === "microWidgetInstance"
								? (() => {
										const comp = components.get(node.id);
										const widgetId = comp
											? (comp.component as unknown as { widgetId?: string })
													.widgetId
											: undefined;
										return widgetId ?? "Package Widget";
									})()
								: node.type}
					</span>

					{/* Container indicator */}
					{isContainer && (
						<span className="shrink-0 text-xs text-muted-foreground">
							[container]
						</span>
					)}

					{/* Visibility toggle button */}
					<button
						type="button"
						className={cn(
							"shrink-0 p-0.5 hover:bg-muted-foreground/10 rounded transition-opacity",
							node.hidden ? "opacity-100" : "opacity-0 group-hover:opacity-100",
						)}
						onClick={(e) => {
							e.stopPropagation();
							onToggleHidden(node.id);
						}}
						title={node.hidden ? "Show" : "Hide"}
					>
						{node.hidden ? (
							<EyeOff className="h-3 w-3 text-muted-foreground" />
						) : (
							<Eye className="h-3 w-3 text-muted-foreground" />
						)}
					</button>

					{/* Lock indicator */}
					{node.locked && <Lock className="h-3 w-3 text-muted-foreground" />}
				</div>
			</ContextMenuTrigger>

			<ContextMenuContent>
				<ContextMenuItem
					onClick={() => onCopy(isSelected ? selection : [node.id])}
				>
					<Copy className="h-4 w-4 mr-2" />
					{t("copy", "Copy")}
				</ContextMenuItem>
				<ContextMenuItem
					onClick={() => onCut(isSelected ? selection : [node.id])}
				>
					<Scissors className="h-4 w-4 mr-2" />
					{t("cut", "Cut")}
				</ContextMenuItem>
				<ContextMenuItem onClick={() => onPaste(node.id)}>
					<FolderPlus className="h-4 w-4 mr-2" />
					{canReceiveChildren
						? t("pasteInto", "Paste into")
						: t("paste", "Paste")}
				</ContextMenuItem>
				<ContextMenuSeparator />
				<ContextMenuItem onClick={() => onToggleLock(node.id)}>
					{node.locked ? (
						<>
							<Unlock className="h-4 w-4 mr-2" />
							{t("unlock", "Unlock")}
						</>
					) : (
						<>
							<Lock className="h-4 w-4 mr-2" />
							{t("lock", "Lock")}
						</>
					)}
				</ContextMenuItem>
				<ContextMenuItem onClick={() => onToggleHidden(node.id)}>
					{node.hidden ? (
						<>
							<Eye className="h-4 w-4 mr-2" />
							{t("show", "Show")}
						</>
					) : (
						<>
							<EyeOff className="h-4 w-4 mr-2" />
							{t("hide", "Hide")}
						</>
					)}
				</ContextMenuItem>
				<ContextMenuSeparator />
				<ContextMenuItem
					onClick={() => onDelete(node.id)}
					className="text-destructive focus:text-destructive"
				>
					<Trash className="h-4 w-4 mr-2" />
					{t("delete", "Delete")}
				</ContextMenuItem>
			</ContextMenuContent>

			{/* Children */}
			{hasChildren && isNodeExpanded && (
				<div>
					{node.children.map((child) => (
						<TreeNode
							key={child.id}
							node={child}
							depth={depth + 1}
							isSelected={selection.includes(child.id)}
							expandedNodes={expandedNodes}
							selection={selection}
							onToggleExpand={onToggleExpand}
							onToggleLock={onToggleLock}
							onToggleHidden={onToggleHidden}
							onSelect={onSelect}
							onDelete={onDelete}
							onCopy={onCopy}
							onCut={onCut}
							onPaste={onPaste}
							findParentId={findParentId}
						/>
					))}
				</div>
			)}
		</ContextMenu>
	);
}
