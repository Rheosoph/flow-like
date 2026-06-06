"use client";

import { cn } from "../../../lib/utils";
import { useComponentActionTrigger } from "../ActionHandler";
import type { ComponentProps } from "../ComponentRegistry";
import { useData } from "../DataContext";
import { resolveInlineStyle, resolveStyle } from "../StyleResolver";
import type {
	BoundValue,
	InventoryGridComponent,
	InventoryItemDef,
} from "../types";

function useResolved<T>(boundValue: BoundValue | undefined): T | undefined {
	const { resolve } = useData();
	if (!boundValue) return undefined;
	return resolve(boundValue) as T;
}

export function A2UIInventoryGrid({
	component,
	style,
	componentId,
}: ComponentProps<InventoryGridComponent>) {
	const { resolve } = useData();
	const itemsRaw = useResolved<InventoryItemDef[]>(component.items);
	const triggerAction = useComponentActionTrigger(componentId);

	const items = itemsRaw ?? [];
	const columns = useResolved<number>(component.columns) ?? 4;
	const rows = useResolved<number>(component.rows) ?? 4;
	const cellSize = useResolved<string>(component.cellSize) ?? "64px";
	const totalSlots = columns * rows;

	const handleItemClick = (item: InventoryItemDef, index: number) => {
		void triggerAction(component.actions, {
			itemId: item.id,
			itemIndex: index,
		});
	};

	return (
		<div
			className={cn("inline-block", resolveStyle(style))}
			style={{
				display: "grid",
				gridTemplateColumns: `repeat(${columns}, ${cellSize})`,
				gap: "4px",
				...resolveInlineStyle(style),
			}}
		>
			{Array.from({ length: totalSlots }).map((_, index) => {
				const item = items[index];
				const icon = item?.icon ? (resolve(item.icon) as string) : undefined;
				const name = item?.name ? (resolve(item.name) as string) : undefined;
				const quantity = item?.quantity
					? (resolve(item.quantity) as number)
					: undefined;
				return (
					<button
						key={index}
						type="button"
						className={cn(
							"relative flex items-center justify-center border rounded bg-muted/50 hover:bg-muted transition-colors",
							item && "cursor-pointer",
						)}
						style={{ width: cellSize, height: cellSize }}
						onClick={() => item && handleItemClick(item, index)}
						disabled={!item}
					>
						{item && icon && (
							<>
								<img
									src={icon}
									alt={name ?? ""}
									className="w-3/4 h-3/4 object-contain"
								/>
								{quantity && quantity > 1 && (
									<span className="absolute bottom-0.5 right-1 text-xs font-bold">
										{quantity}
									</span>
								)}
							</>
						)}
					</button>
				);
			})}
		</div>
	);
}
