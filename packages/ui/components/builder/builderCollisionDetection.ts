import { type CollisionDetection, pointerWithin } from "@dnd-kit/core";
import type { SurfaceComponent } from "../a2ui/types";
import type { DragData, DropData } from "./BuilderDndContext";
import {
	canAcceptComponentChildren,
	canMoveComponent,
	findComponentParent,
	getExplicitChildren,
} from "./componentTree";
import { getInsertionPlacement } from "./dropPlacement";
import {
	getBuilderElementRectangle,
	getElementRectangle,
} from "./element-geometry";

export function measureBuilderDroppable(element: HTMLElement) {
	const rect = getBuilderElementRectangle(element);
	return {
		...rect,
		right: rect.left + rect.width,
		bottom: rect.top + rect.height,
	};
}

export function createBuilderCollisionDetection(
	components: Map<string, SurfaceComponent>,
): CollisionDetection {
	return (args) => {
		// A dragged handle can overlap the canvas after the pointer has left it.
		// Only the pointer determines whether releasing will place an element.
		const pointer = args.pointerCoordinates;
		if (!pointer) return [];
		const drag = args.active.data.current as DragData | undefined;
		const collisions = pointerWithin(args)
			.flatMap((collision) => {
				const container = args.droppableContainers.find(
					(target) => target.id === collision.id,
				);
				return container
					? [
							{
								...collision,
								data: { ...collision.data, droppableContainer: container },
							},
						]
					: [];
			})
			.filter((collision) => {
				const container = collision.data.droppableContainer;
				const data = container.data.current as DropData | undefined;
				if (!data || !canAcceptComponentChildren(components.get(data.parentId)))
					return false;
				if (
					drag?.type === "a2ui-component-move" &&
					!canMoveComponent(components, drag.componentId, data.parentId)
				) {
					return false;
				}
				// Ignore the clipped part of a target inside a scrolling panel.
				let ancestor = container.node.current?.parentElement;
				while (ancestor) {
					const style =
						ancestor.ownerDocument.defaultView?.getComputedStyle(ancestor);
					const rect = ancestor.getBoundingClientRect();
					if (style && rect.width > 0 && rect.height > 0) {
						const { x, y } = pointer;
						if (
							/(auto|scroll|hidden|clip)/.test(style.overflowX) &&
							(x < rect.left || x > rect.right)
						)
							return false;
						if (
							/(auto|scroll|hidden|clip)/.test(style.overflowY) &&
							(y < rect.top || y > rect.bottom)
						)
							return false;
					}
					ancestor = ancestor.parentElement;
				}
				return true;
			});

		collisions.sort((a, b) => {
			const aContainer = a.data.droppableContainer;
			const bContainer = b.data.droppableContainer;
			const aNode = aContainer.node.current;
			const bNode = bContainer.node.current;
			if (aNode && bNode && aNode !== bNode) {
				if (aNode.contains(bNode)) return 1;
				if (bNode.contains(aNode)) return -1;
			}
			// The measured rect is a ref. Comparing the ref itself yields NaN.
			const aRect = args.droppableRects.get(a.id);
			const bRect = args.droppableRects.get(b.id);
			const areaDifference =
				(aRect ? aRect.width * aRect.height : Number.POSITIVE_INFINITY) -
				(bRect ? bRect.width * bRect.height : Number.POSITIVE_INFINITY);
			if (areaDifference) return areaDifference;
			const aData = aContainer.data.current as DropData;
			const bData = bContainer.data.current as DropData;
			return (
				Number(bData.type === "drop-zone") - Number(aData.type === "drop-zone")
			);
		});

		let collision = collisions[0];
		if (!collision) return [];
		const bestNode = collision.data.droppableContainer.node.current;
		const bestData = collision.data.droppableContainer.data.current as DropData;
		if (
			bestData.type === "container" &&
			bestNode?.dataset.builderComponent === bestData.parentId
		) {
			const parentId = findComponentParent(components, bestData.parentId);
			const parentCollision = collisions.find((candidate) => {
				const target = candidate.data.droppableContainer;
				return (
					target.data.current?.parentId === parentId &&
					target.node.current?.dataset.builderComponent === parentId
				);
			});
			const parentNode = parentCollision?.data.droppableContainer.node.current;
			if (parentCollision && parentNode) {
				const parentStyle =
					parentNode.ownerDocument.defaultView?.getComputedStyle(parentNode);
				const horizontal =
					parentStyle?.display.includes("grid") ||
					(parentStyle?.display.includes("flex") &&
						parentStyle.flexDirection.startsWith("row"));
				const rect = args.droppableRects.get(collision.id);
				if (rect) {
					const size = horizontal ? rect.width : rect.height;
					const position = horizontal
						? pointer.x - rect.left
						: pointer.y - rect.top;
					const edge = Math.min(8, size * 0.2);
					if (position <= edge || position >= size - edge)
						collision = parentCollision;
				}
			}
		}
		const container = collision.data.droppableContainer;
		let dropData = container.data.current as DropData;
		const node = container.node.current;
		if (
			dropData.type === "container" &&
			node?.dataset.builderComponent === dropData.parentId
		) {
			const childIds = getExplicitChildren(components.get(dropData.parentId));
			const nodes = new Map(
				Array.from(
					node.querySelectorAll<HTMLElement>("[data-builder-component]"),
				).map((child) => [child.dataset.builderComponent, child]),
			);
			const childRects = childIds.flatMap((id, index) => {
				const child = nodes.get(id);
				if (!child) return [];
				const rect = getElementRectangle(child);
				return rect.width || rect.height
					? [
							{
								index,
								left: rect.left,
								top: rect.top,
								width: rect.width,
								height: rect.height,
							},
						]
					: [];
			});
			const style = node.ownerDocument.defaultView?.getComputedStyle(node);
			const grid = style?.display.includes("grid") ?? false;
			const flex = style?.display.includes("flex") ?? false;
			const horizontal =
				grid || (flex && style?.flexDirection.startsWith("row"));
			const reverse = horizontal
				? (style?.flexDirection === "row-reverse") !==
					(style?.direction === "rtl")
				: style?.flexDirection === "column-reverse";
			dropData = {
				...dropData,
				...getInsertionPlacement(
					getBuilderElementRectangle(node),
					childRects,
					pointer,
					{
						orientation: horizontal ? "horizontal" : "vertical",
						reverse,
						wrapped: grid || (!!style?.flexWrap && style.flexWrap !== "nowrap"),
					},
				),
			};
		}
		return [{ ...collision, data: { ...collision.data, dropData } }];
	};
}
