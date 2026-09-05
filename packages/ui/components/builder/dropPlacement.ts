export interface DropRect {
	left: number;
	top: number;
	width: number;
	height: number;
}

export interface IndexedDropRect extends DropRect {
	index: number;
}

export interface DropLayout {
	orientation: "horizontal" | "vertical";
	reverse?: boolean;
	wrapped?: boolean;
}

/** Uses viewport coordinates so the same result works at every canvas zoom. */
export function getInsertionPlacement(
	container: DropRect,
	children: IndexedDropRect[],
	pointer: { x: number; y: number },
	layout: DropLayout,
): { index: number; indicator: DropRect } {
	const horizontal = layout.orientation === "horizontal";
	if (!children.length) {
		return {
			index: 0,
			indicator: horizontal
				? {
						left: container.left,
						top: container.top,
						width: 2,
						height: container.height,
					}
				: {
						left: container.left,
						top: container.top,
						width: container.width,
						height: 2,
					},
		};
	}
	let nearest = children[0];
	let distance = Number.POSITIVE_INFINITY;
	for (const child of children) {
		const dx = Math.max(
			child.left - pointer.x,
			0,
			pointer.x - child.left - child.width,
		);
		const dy = Math.max(
			child.top - pointer.y,
			0,
			pointer.y - child.top - child.height,
		);
		const nextDistance = dx * dx + dy * dy;
		if (nextDistance < distance) {
			nearest = child;
			distance = nextDistance;
		}
	}
	const start = horizontal ? nearest.left : nearest.top;
	const size = horizontal ? nearest.width : nearest.height;
	const coordinate = horizontal ? pointer.x : pointer.y;
	const after = layout.reverse
		? coordinate < start + size / 2
		: coordinate >= start + size / 2;
	const edge = start + (after !== !!layout.reverse ? size : 0);
	const cross = layout.wrapped ? nearest : container;
	return {
		index: nearest.index + (after ? 1 : 0),
		indicator: horizontal
			? { left: edge - 1, top: cross.top, width: 2, height: cross.height }
			: { left: cross.left, top: edge - 1, width: cross.width, height: 2 },
	};
}
