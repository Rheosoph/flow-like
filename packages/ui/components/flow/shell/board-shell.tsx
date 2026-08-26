"use client";

import type { ReactNode } from "react";
import { memo, useEffect, useRef } from "react";
import type { ImperativePanelHandle } from "react-resizable-panels";
import {
	ResizableHandle,
	ResizablePanel,
	ResizablePanelGroup,
} from "../../ui/resizable";

/**
 * The board's layout frame.
 *
 * Two rules it exists to enforce. Every region is a box in normal flow — the
 * rail, the tab strip and the status bar can no longer overlap the canvas or be
 * painted over by a panel, which the floating dock and the viewport-fixed
 * status cluster both did. And optional regions are *unmounted* rather than
 * collapsed to zero width, so closing one hands its space back to the editor
 * instead of to whichever sibling happened to be next to it.
 *
 * `order` is mandatory on conditionally rendered panels, or react-resizable-panels
 * mis-attributes persisted sizes when a region comes back.
 */
export const BoardShell = memo(function BoardShell({
	rail,
	sidebar,
	tabs,
	breadcrumb,
	canvas,
	script,
	panel,
	secondary,
	secondaryWide,
	statusBar,
	overlays,
}: Readonly<{
	rail: ReactNode;
	sidebar?: ReactNode;
	tabs?: ReactNode;
	breadcrumb?: ReactNode;
	canvas: ReactNode;
	script?: ReactNode;
	panel?: ReactNode;
	secondary?: ReactNode;
	/**
	 * The secondary view needs room for a second column — FlowPilot opening its
	 * FlowScript workspace. Sizes are persisted, so the panel is resized
	 * imperatively; changing `defaultSize` would not move a mounted panel.
	 */
	secondaryWide?: boolean;
	statusBar: ReactNode;
	overlays?: ReactNode;
}>) {
	const secondaryPanel = useRef<ImperativePanelHandle>(null);

	useEffect(() => {
		if (!secondary) return;
		secondaryPanel.current?.resize(secondaryWide ? 48 : 22);
	}, [secondaryWide, secondary]);

	return (
		<div className="relative flex min-h-0 w-full flex-1 grow flex-col overflow-hidden">
			<div className="flex min-h-0 flex-1">
				{rail}
				<ResizablePanelGroup
					direction="horizontal"
					autoSaveId="flow-board-shell"
					className="min-h-0 flex-1"
					style={{ touchAction: "none", overflow: "hidden" }}
				>
					{sidebar && (
						<>
							<ResizablePanel
								id="board-sidebar"
								order={1}
								defaultSize={18}
								minSize={12}
								maxSize={36}
								className="hidden md:block"
							>
								{sidebar}
							</ResizablePanel>
							<ResizableHandle withHandle />
						</>
					)}

					<ResizablePanel id="board-editor" order={2} minSize={25}>
						<ResizablePanelGroup
							direction="vertical"
							autoSaveId="flow-board-editor"
							className="h-full"
						>
							<ResizablePanel id="board-editors" order={1} minSize={20}>
								<div className="flex h-full min-h-0 flex-col">
									{tabs}
									{breadcrumb}
									<div className="min-h-0 flex-1">
										{script ? (
											<ResizablePanelGroup
												direction="horizontal"
												autoSaveId="flow-board-split"
												className="h-full"
											>
												<ResizablePanel
													id="board-canvas"
													order={1}
													minSize={20}
													className="flex min-h-0 flex-col"
												>
													{canvas}
												</ResizablePanel>
												<ResizableHandle withHandle />
												<ResizablePanel
													id="board-script"
													order={2}
													defaultSize={45}
													minSize={20}
												>
													{script}
												</ResizablePanel>
											</ResizablePanelGroup>
										) : (
											<div className="flex h-full min-h-0 flex-col">
												{canvas}
											</div>
										)}
									</div>
								</div>
							</ResizablePanel>
							{panel && (
								<>
									<ResizableHandle withHandle />
									<ResizablePanel
										id="board-panel"
										order={2}
										defaultSize={30}
										minSize={10}
										className="hidden md:block"
									>
										{panel}
									</ResizablePanel>
								</>
							)}
						</ResizablePanelGroup>
					</ResizablePanel>

					{secondary && (
						<>
							<ResizableHandle withHandle />
							<ResizablePanel
								id="board-secondary"
								order={3}
								defaultSize={22}
								minSize={14}
								maxSize={45}
								className="hidden md:block"
							>
								{secondary}
							</ResizablePanel>
						</>
					)}
				</ResizablePanelGroup>
			</div>
			{statusBar}
			{overlays}
		</div>
	);
});
