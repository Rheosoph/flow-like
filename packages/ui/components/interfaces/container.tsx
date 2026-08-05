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

	// The wrapper renders before a sidebar exists on purpose. Interfaces push
	// theirs from a mount effect, and swapping the tree shape underneath them at
	// that moment unmounts the interface that just finished rendering — the user
	// sees the app load itself a second time.
	return (
		<div className="h-full overflow-hidden flex flex-col grow max-h-full">
			{/* Desktop Resizable Layout */}
			{sidebar && !isMobile && isDesktopSidebarOpen ? (
				<ResizablePanelGroup
					direction="horizontal"
					className="flex-1 w-full flex flex-row items-stretch overflow-hidden"
				>
					<ResizablePanel
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

					<ResizableHandle withHandle />

					<ResizablePanel
						defaultSize={75}
						className="flex flex-col overflow-hidden"
					>
						{memoizedChildren}
					</ResizablePanel>
				</ResizablePanelGroup>
			) : !isMobile ? (
				<>{memoizedChildren}</>
			) : null}

			{/* Mobile Sheet */}
			{isMobile && (
				<>
					{sidebar ? (
						<Sheet open={isOpen} onOpenChange={setIsOpen}>
							<SheetContent
								side="left"
								className="w-80 p-0 border-r bg-background/95 backdrop-blur-sm supports-backdrop-filter:bg-background/60"
							>
								{sidebar}
							</SheetContent>
						</Sheet>
					) : null}

					{/* Main Content for Mobile */}
					<div className="relative h-full flex flex-col grow max-h-full overflow-hidden">
						{memoizedChildren}
					</div>
				</>
			)}
		</div>
	);
};

export const Container = memo(ContainerInner);
Container.displayName = "Container";
