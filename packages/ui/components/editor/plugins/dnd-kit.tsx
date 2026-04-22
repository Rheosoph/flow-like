"use client";

import { useContext } from "react";
import { DndProvider, DndContext as ReactDndContext } from "react-dnd";
import { HTML5Backend } from "react-dnd-html5-backend";
import { TouchBackend } from "react-dnd-touch-backend";

import { DndPlugin } from "@platejs/dnd";
import { PlaceholderPlugin } from "@platejs/media/react";

import { isTauri } from "../../../lib/platform";
import { BlockDraggable } from "../ui/block-draggable";

function DndProviderGuard({ children }: { children: React.ReactNode }) {
	const existing = useContext(ReactDndContext);

	if (existing?.dragDropManager) {
		return <>{children}</>;
	}

	const backend = isTauri() ? TouchBackend : HTML5Backend;
	const options = isTauri()
		? {
				enableMouseEvents: true,
				delayTouchStart: 0,
				delayMouseStart: 0,
				ignoreContextMenu: true,
				touchSlop: 5,
			}
		: undefined;

	return (
		<DndProvider
			backend={backend as any}
			options={options as any}
			context={window}
		>
			{children}
		</DndProvider>
	);
}

export const DndKit = [
	DndPlugin.configure({
		options: {
			enableScroller: true,
			onDropFiles: ({ dragItem, editor, target }) => {
				editor
					.getTransforms(PlaceholderPlugin)
					.insert.media(dragItem.files, { at: target, nextBlock: false });
			},
		},
		render: {
			aboveNodes: BlockDraggable,
			aboveSlate: (props) => (
				<DndProviderGuard>{props.children}</DndProviderGuard>
			),
		},
	}),
];
