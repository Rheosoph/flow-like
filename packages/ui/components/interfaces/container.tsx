"use client";
import {
	type ReactNode,
	memo,
	useEffect,
	useImperativeHandle,
	useMemo,
	useState,
} from "react";
import {
	ResizableHandle,
	ResizablePanel,
	ResizablePanelGroup,
	Sheet,
	SheetContent,
} from "../ui";
import type { ISidebarActions } from "./interfaces";

interface IContainerProps {
	children: ReactNode;
}

const ContainerInner = ({
	ref,
	children,
}: IContainerProps & {
	ref: React.RefObject<ISidebarActions>;
}) => {
	const [sidebar, setSidebar] = useState<ReactNode>();
	const [isOpen, setIsOpen] = useState(false);
	const [isMobile, setIsMobile] = useState(false);
	const [isDesktopSidebarOpen, setIsDesktopSidebarOpen] = useState(false);

	useImperativeHandle(ref, () => ({
		pushSidebar: (element?: ReactNode) => {
			setSidebar(element);
		},
		toggleOpen: () => {
			if (isMobile) {
				setIsOpen((prev) => !prev);
			} else {
				setIsDesktopSidebarOpen((prev) => !prev);
			}
		},
		isMobile: () => isMobile,
		isOpen: () => isOpen,
	}));

	useEffect(() => {
		const checkMobile = () => {
			setIsMobile(window.innerWidth < 768);
		};

		checkMobile();
		window.addEventListener("resize", checkMobile);
		return () => window.removeEventListener("resize", checkMobile);
	}, []);

	// Memoize children to prevent re-renders when sidebar state changes
	const memoizedChildren = useMemo(() => children, [children]);
	const showDesktopSidebar = !!sidebar && !isMobile && isDesktopSidebarOpen;

	// The interface keeps one position in the tree whatever the sidebar does. Opening it, pushing
	// one, or crossing the mobile breakpoint used to give the interface a different parent, so
	// React mounted it afresh: the app loaded itself again, and a chat lost the answer it was
	// streaming. The optional sidebar panel and handle come first and only add siblings.
	return (
		<div className="h-full overflow-hidden flex flex-col grow max-h-full">
			<ResizablePanelGroup
				direction="horizontal"
				className="flex-1 w-full flex flex-row items-stretch overflow-hidden"
			>
				{showDesktopSidebar && (
					<ResizablePanel
						id="interface-sidebar"
						order={1}
						defaultSize={25}
						minSize={20}
						maxSize={40}
						collapsible
						collapsedSize={20}
						onCollapse={() => setIsDesktopSidebarOpen(false)}
						onExpand={() => setIsDesktopSidebarOpen(true)}
						className="flex flex-col overflow-hidden"
					>
						<div className="relative flex-1 flex flex-col bg-background border-r border-border/50 shadow-lg overflow-hidden">
							{sidebar}
						</div>
					</ResizablePanel>
				)}
				{showDesktopSidebar && <ResizableHandle withHandle />}
				<ResizablePanel
					id="interface-content"
					order={2}
					defaultSize={showDesktopSidebar ? 75 : 100}
					className="flex flex-col overflow-hidden"
				>
					{memoizedChildren}
				</ResizablePanel>
			</ResizablePanelGroup>

			{isMobile && sidebar ? (
				<Sheet open={isOpen} onOpenChange={setIsOpen}>
					<SheetContent
						side="left"
						className="w-80 p-0 border-r bg-background/95 backdrop-blur-sm supports-backdrop-filter:bg-background/60"
					>
						{sidebar}
					</SheetContent>
				</Sheet>
			) : null}
		</div>
	);
};

export const Container = memo(ContainerInner);
Container.displayName = "Container";
