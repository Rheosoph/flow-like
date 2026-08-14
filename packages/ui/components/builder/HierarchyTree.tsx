"use client";

import { useTranslation } from "@flow-like/locales";
import { useDraggable, useDroppable } from "@dnd-kit/core";
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
import { cn } from "../../lib";
import type { A2UIComponent, SurfaceComponent } from "../a2ui/types";
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

type HierarchyChildren = { explicitList: string[] } | { referenceId: string };

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
		addComponent,
		updateComponent,
		copy,
		cut,
		paste,
		getComponent,
		hiddenComponents,
		toggleComponentVisibility,
	} = useBuilder();

	// Find parent ID helper
	const findParentId = useCallback(
		(childId: string): string | null => {
			for (const [id, comp] of components) {
				if (!comp.component) continue;
				const compChildrenData = (
					comp.component as unknown as Record<string, unknown>
				).children as HierarchyChildren | undefined;
				const compChildren =
					compChildrenData && "explicitList" in compChildrenData
						? compChildrenData.explicitList
						: undefined;
				if (compChildren?.includes(childId)) {
					return id;
				}
			}
			return null;
		},
		[components],
	);

	// Move component from one parent to another with optional position
	const moveComponent = useCallback(
		(
			movingId: string,
			fromParentId: string | null,
			toParentId: string,
			position?: { type: "before" | "after" | "inside"; targetId?: string },
		) => {
			// Remove from old parent
			if (fromParentId) {
				const fromParent = components.get(fromParentId);
				if (fromParent) {
					const fromChildrenData = (
						fromParent.component as unknown as Record<string, unknown>
					).children as HierarchyChildren | undefined;
					const fromChildren =
						fromChildrenData && "explicitList" in fromChildrenData
							? fromChildrenData.explicitList
							: [];
					updateComponent(fromParentId, {
						component: {
							...fromParent.component,
							children: {
								explicitList: fromChildren.filter((id) => id !== movingId),
							},
						} as A2UIComponent,
					});
				}
			}

			// Add to new parent at specific position
			const toParent = components.get(toParentId);
			if (toParent) {
				const toChildrenData = (
					toParent.component as unknown as Record<string, unknown>
				).children as HierarchyChildren | undefined;
				const toChildren =
					toChildrenData && "explicitList" in toChildrenData
						? [...toChildrenData.explicitList]
						: [];

				let newChildren: string[];
				if (position?.type === "before" && position.targetId) {
					const idx = toChildren.indexOf(position.targetId);
					if (idx >= 0) {
						newChildren = [
							...toChildren.slice(0, idx),
							movingId,
							...toChildren.slice(idx),
						];
					} else {
						newChildren = [...toChildren, movingId];
					}
				} else if (position?.type === "after" && position.targetId) {
					const idx = toChildren.indexOf(position.targetId);
					if (idx >= 0) {
						newChildren = [
							...toChildren.slice(0, idx + 1),
							movingId,
							...toChildren.slice(idx + 1),
						];
					} else {
						newChildren = [...toChildren, movingId];
					}
				} else {
					// Default: append at end (inside)
					newChildren = [...toChildren, movingId];
				}

				updateComponent(toParentId, {
					component: {
						...toParent.component,
						children: { explicitList: newChildren },
					} as A2UIComponent,
				});
			}
		},
		[components, updateComponent],
	);

	// Add new component to parent
	const handleAddComponent = useCallback(
		(newComponent: SurfaceComponent, parentId: string) => {
			addComponent(newComponent);
			const parent = components.get(parentId);
			if (parent) {
				const parentChildrenData = (
					parent.component as unknown as Record<string, unknown>
				).children as HierarchyChildren | undefined;
				const parentChildren =
					parentChildrenData && "explicitList" in parentChildrenData
						? parentChildrenData.explicitList
						: [];
				updateComponent(parentId, {
					component: {
						...parent.component,
						children: { explicitList: [...parentChildren, newComponent.id] },
					} as A2UIComponent,
				});
			}
		},
		[components, addComponent, updateComponent],
	);

	// Build tree from components
	const tree = useMemo(() => {
		const buildTree = (componentId: string): TreeNodeData | null => {
			const component = getComponent(componentId);
			if (!component || !component.component) return null;

			const props = component.component as unknown as Record<string, unknown>;
			const childIds: string[] = [];

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

			// Extract children based on component structure
			if ("children" in props && props.children) {
				const children = props.children as { explicitList?: string[] };
				if (children.explicitList) {
					childIds.push(...children.explicitList);
				}
			}
			if ("child" in props && typeof props.child === "string") {
				childIds.push(props.child);
			}
			if (
				"entryPointChild" in props &&
				typeof props.entryPointChild === "string"
			) {
				childIds.push(props.entryPointChild);
			}
			if ("contentChild" in props && typeof props.contentChild === "string") {
				childIds.push(props.contentChild);
			}

			return {
				id: componentId,
				type: component.component.type,
				children: childIds
					.map((id) => buildTree(id))
					.filter((n): n is TreeNodeData => n !== null),
				locked: lockedNodes.has(componentId),
				hidden: hiddenComponents.has(componentId),
			};
		};

		// If no root components specified, build from all top-level components
		if (rootComponents.length === 0) {
			// Find components that aren't children of any other component
			const childIds = new Set<string>();
			components.forEach((comp) => {
				if (!comp.component) return;
				const props = comp.component as unknown as Record<string, unknown>;
				if ("children" in props && props.children) {
					const children = props.children as { explicitList?: string[] };
					if (children.explicitList) {
						for (const id of children.explicitList) {
							childIds.add(id);
						}
					}
				}
				if ("child" in props && typeof props.child === "string") {
					childIds.add(props.child);
				}
				if (
					"entryPointChild" in props &&
					typeof props.entryPointChild === "string"
				) {
					childIds.add(props.entryPointChild);
				}
				if ("contentChild" in props && typeof props.contentChild === "string") {
					childIds.add(props.contentChild);
				}
			});

			const roots: TreeNodeData[] = [];
			const addedRoots = new Set<string>(); // Track added roots to prevent duplicates
			components.forEach((comp) => {
				if (!childIds.has(comp.id) && !addedRoots.has(comp.id)) {
					const node = buildTree(comp.id);
					if (node) {
						roots.push(node);
						addedRoots.add(comp.id);
					}
				}
			});
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

	// Track last expanded selection to prevent infinite loops
	const lastExpandedSelectionRef = useRef<string | null>(null);

	// Auto-expand tree when a component is selected on canvas
	useEffect(() => {
		if (selection.componentIds.length === 0) return;

		// Get the first selected component
		const selectedId = selection.componentIds[0];

		// Skip if we already expanded for this selection
		if (lastExpandedSelectionRef.current === selectedId) return;

		const path = findPathToComponent(selectedId);

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

			lastExpandedSelectionRef.current = selectedId;
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
		(nodeId: string, event: React.MouseEvent) => {
			selectComponent(nodeId, event.shiftKey || event.metaKey || event.ctrlKey);
		},
		[selectComponent],
	);

	return (
		<div
			className={cn(
				"flex flex-col h-full bg-background border-r overflow-hidden",
				className,
			)}
		>
			<div className="p-3 border-b shrink-0">
				<div className="relative">
					<Search className="absolute left-2.5 top-2.5 h-4 w-4 text-muted-foreground" />
					<Input
						placeholder={t('searchTree', 'Search tree...')}
						value={searchQuery}
						onChange={(e) => setSearchQuery(e.target.value)}
						className="pl-8 h-8"
					/>
				</div>
			</div>

			<ScrollArea className="flex-1 min-h-0">
				<div className="p-2">
					{filteredTree.length === 0 ? (
						<div className="text-sm text-muted-foreground text-center py-4">
							{t('noComponents', 'No components')}
						</div>
					) : (
						filteredTree.map((node) => (
							<TreeNode
								key={node.id}
								node={node}
								depth={0}
								isExpanded={expandedNodes.has(node.id)}
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
								onMoveComponent={moveComponent}
								onAddComponent={handleAddComponent}
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
	isExpanded: boolean;
	isSelected: boolean;
	expandedNodes: Set<string>;
	selection: string[];
	onToggleExpand: (id: string) => void;
	onToggleLock: (id: string) => void;
	onToggleHidden: (id: string) => void;
	onSelect: (id: string, event: React.MouseEvent) => void;
	onDelete: (id: string) => void;
	onCopy: () => void;
	onCut: () => void;
	onPaste: (parentId?: string) => void;
	onMoveComponent: (
		componentId: string,
		fromParentId: string | null,
		toParentId: string,
		position?: { type: "before" | "after" | "inside"; targetId?: string },
	) => void;
	onAddComponent: (component: SurfaceComponent, parentId: string) => void;
	findParentId: (childId: string) => string | null;
}

type Children = { explicitList: string[] } | { referenceId: string };

type DropPosition = "before" | "after" | "inside" | null;

function TreeNode({
	node,
	depth,
	isExpanded,
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
	onMoveComponent,
	onAddComponent,
	findParentId,
}: TreeNodeProps) {
	const { t } = useTranslation("flow");
	const { components, getWidgetRef } = useBuilder();
	const [dropPosition, setDropPosition] = useState<DropPosition>(null);
	const hasChildren = node.children.length > 0;
	const isNodeExpanded = expandedNodes.has(node.id);
	const isContainer = CONTAINER_TYPES.has(node.type);
	const isRoot = node.id === ROOT_ID;

	// Check if targetId is a descendant of sourceId
	const isDescendant = useCallback(
		(targetId: string, sourceId: string): boolean => {
			const target = components.get(targetId);
			if (!target) return false;

			const targetChildrenData = (
				target.component as unknown as Record<string, unknown>
			).children as Children | undefined;
			const targetChildren =
				targetChildrenData && "explicitList" in targetChildrenData
					? targetChildrenData.explicitList
					: [];

			if (targetChildren.includes(sourceId)) return true;
			return targetChildren.some((childId) => isDescendant(childId, sourceId));
		},
		[components],
	);

	// Draggable for moving this node
	const {
		attributes: dragAttributes,
		listeners: dragListeners,
		setNodeRef: setDragRef,
		isDragging,
	} = useDraggable({
		id: `tree-move-${node.id}`,
		disabled: isRoot || node.locked,
		data: {
			type: COMPONENT_MOVE_TYPE,
			componentId: node.id,
			currentParentId: findParentId(node.id),
		} satisfies ComponentMoveData,
	});

	// Droppable for receiving components
	const { setNodeRef: setDropRef, isOver } = useDroppable({
		id: `tree-drop-${node.id}`,
		disabled: !isContainer,
		data: {
			type: "container",
			parentId: node.id,
			isContainer: true,
		} satisfies DropData,
	});

	// For drop, we accept if isContainer and not dropping on self or descendant
	const canDrop = isContainer;

	// Reset drop position when not hovering
	const handleMouseLeave = useCallback(() => {
		setDropPosition(null);
	}, []);

	return (
		<ContextMenu>
			<ContextMenuTrigger>
				<div
					id={`tree-node-${node.id}`}
					ref={(el) => {
						setDropRef(el);
					}}
					onMouseLeave={handleMouseLeave}
					className={cn(
						"group relative flex items-center gap-1 px-2 py-1 rounded text-sm cursor-pointer hover:bg-muted transition-colors",
						isSelected && "bg-primary/10 text-primary",
						node.hidden && "opacity-50",
						isDragging && "opacity-40",
						isOver && canDrop && "bg-primary/20 ring-1 ring-primary",
					)}
					style={{ paddingLeft: `${depth * 16 + 8}px` }}
					onClick={(e) => {
						e.stopPropagation();
						onSelect(node.id, e);
					}}
				>
					{/* Drag handle */}
					{!isRoot && !node.locked && (
						<div
							ref={setDragRef}
							{...dragListeners}
							{...dragAttributes}
							className="cursor-grab hover:bg-muted-foreground/10 rounded p-0.5 touch-none"
							onClick={(e) => e.stopPropagation()}
						>
							<GripVertical className="h-3 w-3 text-muted-foreground" />
						</div>
					)}

					{/* Expand toggle */}
					<button
						type="button"
						className={cn(
							"p-0.5 hover:bg-muted-foreground/10 rounded",
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
					<span className="truncate flex-1">
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
						<span className="text-xs text-muted-foreground">{`[container]`}</span>
					)}

					{/* Visibility toggle button */}
					<button
						type="button"
						className={cn(
							"p-0.5 hover:bg-muted-foreground/10 rounded transition-opacity",
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
				<ContextMenuItem onClick={onCopy}>
					<Copy className="h-4 w-4 mr-2" />
					{t('copy', 'Copy')}
				</ContextMenuItem>
				<ContextMenuItem onClick={onCut}>
					<Scissors className="h-4 w-4 mr-2" />
					{t('cut', 'Cut')}
				</ContextMenuItem>
				<ContextMenuItem onClick={() => onPaste(node.id)}>
					<FolderPlus className="h-4 w-4 mr-2" />
					{t('pasteInto', 'Paste into')}
				</ContextMenuItem>
				<ContextMenuSeparator />
				<ContextMenuItem onClick={() => onToggleLock(node.id)}>
					{node.locked ? (
						<>
							<Unlock className="h-4 w-4 mr-2" />
							{t('unlock', 'Unlock')}
						</>
					) : (
						<>
							<Lock className="h-4 w-4 mr-2" />
							{t('lock', 'Lock')}
						</>
					)}
				</ContextMenuItem>
				<ContextMenuItem onClick={() => onToggleHidden(node.id)}>
					{node.hidden ? (
						<>
							<Eye className="h-4 w-4 mr-2" />
							{t('show', 'Show')}
						</>
					) : (
						<>
							<EyeOff className="h-4 w-4 mr-2" />
							{t('hide', 'Hide')}
						</>
					)}
				</ContextMenuItem>
				<ContextMenuSeparator />
				<ContextMenuItem
					onClick={() => onDelete(node.id)}
					className="text-destructive focus:text-destructive"
				>
					<Trash className="h-4 w-4 mr-2" />
					{t('delete', 'Delete')}
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
							isExpanded={expandedNodes.has(child.id)}
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
							onMoveComponent={onMoveComponent}
							onAddComponent={onAddComponent}
							findParentId={findParentId}
						/>
					))}
				</div>
			)}
		</ContextMenu>
	);
}
