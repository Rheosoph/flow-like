"use client";

import { useDraggable, useDroppable } from "@dnd-kit/core";
import { useTranslation } from "@flow-like/locales";
import {
	ClipboardPaste,
	Copy,
	GripVertical,
	Scissors,
	Sparkles,
	Trash2,
} from "lucide-react";
import {
	type ReactNode,
	type RefObject,
	createContext,
	memo,
	useCallback,
	useContext,
	useEffect,
	useLayoutEffect,
	useMemo,
	useRef,
	useState,
} from "react";
import { createPortal } from "react-dom";
import { useRuntimeTailwindStyles } from "../../lib/use-runtime-tailwind";
import { cn } from "../../lib/utils";
import { ActionProvider } from "../a2ui/ActionHandler";
import {
	type ComponentProps,
	getComponentRenderer,
} from "../a2ui/ComponentRegistry";
import { DataProvider, DataScopeProvider } from "../a2ui/DataContext";
import { WidgetRefsProvider } from "../a2ui/WidgetRefsContext";
import type {
	A2UIClientMessage,
	Children,
	DataScope,
	Surface,
	SurfaceComponent,
} from "../a2ui/types";
import { Tooltip, TooltipContent, TooltipTrigger } from "../ui/tooltip";
import { useBuilder } from "./BuilderContext";
import {
	COMPONENT_MOVE_TYPE,
	type ComponentMoveData,
	type DropData,
	useBuilderDnd,
} from "./BuilderDndContext";
import { ROOT_ID } from "./WidgetBuilder";
import {
	canAcceptComponentChildren,
	canReorderComponent,
	findComponentParent,
} from "./componentTree";
import {
	type ElementRectangle,
	getBuilderElementRectangle,
	getCanvasClip,
	getCanvasViewport,
	placeElementToolbar,
} from "./element-geometry";

interface BuilderRendererProps {
	surface: Surface;
	className?: string;
}

const CanvasElementContext =
	createContext<RefObject<HTMLDivElement | null> | null>(null);

interface SelectionToolbarProps {
	anchor: ElementRectangle;
	viewport: ElementRectangle;
	ownerId?: string;
	componentType: string;
	isRoot: boolean;
	canDrag: boolean;
	onDelete: () => void;
	onCopy: () => void;
	onCut: () => void;
	onPaste: () => void;
	onOptimize?: () => void;
	dragHandleRef: (node: HTMLElement | null) => void;
	dragAttributes: React.HTMLAttributes<HTMLButtonElement>;
	dragListeners: React.DOMAttributes<HTMLButtonElement> | undefined;
}

const SelectionToolbar = memo(function SelectionToolbar({
	anchor,
	viewport,
	ownerId,
	componentType,
	isRoot,
	canDrag,
	onDelete,
	onCopy,
	onCut,
	onPaste,
	onOptimize,
	dragHandleRef,
	dragAttributes,
	dragListeners,
}: SelectionToolbarProps) {
	const { t } = useTranslation("flow");
	const toolbarRef = useRef<HTMLDivElement>(null);
	const [size, setSize] = useState({ width: 0, height: 0 });
	useLayoutEffect(() => {
		const toolbar = toolbarRef.current;
		const view = toolbar?.ownerDocument.defaultView;
		if (!toolbar || !view) return;
		const measure = () => {
			const rect = toolbar.getBoundingClientRect();
			setSize((previous) =>
				previous.width === rect.width && previous.height === rect.height
					? previous
					: { width: rect.width, height: rect.height },
			);
		};
		measure();
		const observer = new view.ResizeObserver(measure);
		observer.observe(toolbar);
		return () => observer.disconnect();
	}, []);
	const position = placeElementToolbar(anchor, size, viewport);
	return (
		<div
			ref={toolbarRef}
			data-builder-toolbar=""
			data-builder-owner={ownerId}
			role="toolbar"
			aria-label={t("elementActions", "Element actions")}
			className="fixed z-40 flex w-max flex-wrap items-center gap-1 overflow-y-auto px-1.5 py-1 bg-primary text-primary-foreground rounded-md text-xs shadow-lg pointer-events-auto border border-primary-foreground/10"
			style={{
				left: position.left,
				top: position.top,
				maxWidth: position.maxWidth,
				maxHeight: position.maxHeight,
				visibility: position.visible && size.width > 0 ? "visible" : "hidden",
			}}
			onPointerDown={(e) => e.stopPropagation()}
		>
			{/* Drag Handle - only for non-root */}
			{!isRoot && canDrag && (
				<Tooltip>
					<TooltipTrigger asChild>
						<button
							type="button"
							aria-label={t("dragToMove", "Drag to move")}
							ref={dragHandleRef}
							{...dragAttributes}
							{...dragListeners}
							className="p-1.5 hover:bg-white/20 rounded-md cursor-grab active:cursor-grabbing touch-none transition-colors"
						>
							<GripVertical className="h-3.5 w-3.5" />
						</button>
					</TooltipTrigger>
					<TooltipContent side="top">
						{t("dragToMove", "Drag to move")}
					</TooltipContent>
				</Tooltip>
			)}

			{/* Component Type Label */}
			<span
				className="min-w-0 max-w-32 truncate px-2 py-0.5 font-medium capitalize select-none bg-white/10 rounded"
				title={componentType}
			>
				{componentType}
			</span>

			<div className="w-px h-5 bg-white/20 mx-1" />

			{/* Copy */}
			<Tooltip>
				<TooltipTrigger asChild>
					<button
						type="button"
						aria-label={t("copy", "Copy")}
						onClick={onCopy}
						className="p-1.5 hover:bg-white/20 rounded-md transition-colors"
					>
						<Copy className="h-3.5 w-3.5" />
					</button>
				</TooltipTrigger>
				<TooltipContent side="top">{t("copyC", "Copy (⌘C)")}</TooltipContent>
			</Tooltip>

			{/* Cut - only for non-root */}
			{!isRoot && (
				<Tooltip>
					<TooltipTrigger asChild>
						<button
							type="button"
							aria-label={t("cut", "Cut")}
							onClick={onCut}
							className="p-1.5 hover:bg-white/20 rounded-md transition-colors"
						>
							<Scissors className="h-3.5 w-3.5" />
						</button>
					</TooltipTrigger>
					<TooltipContent side="top">{t("cutX", "Cut (⌘X)")}</TooltipContent>
				</Tooltip>
			)}

			{/* Paste */}
			<Tooltip>
				<TooltipTrigger asChild>
					<button
						type="button"
						aria-label={t("paste", "Paste")}
						onClick={onPaste}
						className="p-1.5 hover:bg-white/20 rounded-md transition-colors"
					>
						<ClipboardPaste className="h-3.5 w-3.5" />
					</button>
				</TooltipTrigger>
				<TooltipContent side="top">{t("pasteV", "Paste (⌘V)")}</TooltipContent>
			</Tooltip>

			{/* Optimize with FlowPilot */}
			{onOptimize && (
				<>
					<div className="w-px h-5 bg-white/20 mx-1" />
					<Tooltip>
						<TooltipTrigger asChild>
							<button
								type="button"
								onClick={onOptimize}
								className="p-1.5 hover:bg-white/20 rounded-md transition-colors"
							>
								<Sparkles className="h-3.5 w-3.5" />
							</button>
						</TooltipTrigger>
						<TooltipContent side="top">
							{t("optimizeWithFlowpilot", "Optimize with FlowPilot")}
						</TooltipContent>
					</Tooltip>
				</>
			)}

			{/* Delete - only for non-root */}
			{!isRoot && (
				<>
					<div className="w-px h-5 bg-white/20 mx-1" />
					<Tooltip>
						<TooltipTrigger asChild>
							<button
								type="button"
								aria-label={t("delete", "Delete")}
								onClick={onDelete}
								className="p-1.5 hover:bg-red-500/30 rounded-md transition-colors text-red-200 hover:text-red-100"
							>
								<Trash2 className="h-3.5 w-3.5" />
							</button>
						</TooltipTrigger>
						<TooltipContent side="top">Delete (⌫)</TooltipContent>
					</Tooltip>
				</>
			)}
		</div>
	);
});

interface BuilderComponentProps {
	componentId: string;
	surfaceComponent: SurfaceComponent;
	surfaceId: string;
	renderChild: (childId: string, dataScope?: DataScope) => ReactNode;
}

function BuilderComponent({
	componentId,
	surfaceComponent,
	surfaceId,
	renderChild,
}: BuilderComponentProps) {
	const { t } = useTranslation("flow");
	const canvasRef = useContext(CanvasElementContext);
	const [element, setElement] = useState<HTMLElement | SVGElement | null>(null);
	const [isHovered, setIsHovered] = useState(false);
	const [geometry, setGeometry] = useState<{
		rect: ElementRectangle;
		clip: string;
		viewport: ElementRectangle;
	} | null>(null);
	const { component, style } = surfaceComponent;
	const {
		selection,
		selectComponent,
		deleteComponents,
		copy,
		cut,
		paste,
		isComponentHidden,
		components,
		actionContext,
	} = useBuilder();
	const { activeId } = useBuilderDnd();
	const isContainer = canAcceptComponentChildren(surfaceComponent);
	const canDrag = canReorderComponent(components, componentId);
	const isRoot = componentId === ROOT_ID;
	const isSelected = selection.componentIds.includes(componentId);
	const isPrimarySelection = selection.componentIds.at(-1) === componentId;
	const isHidden = component ? isComponentHidden(componentId) : false;
	const childData = (component as unknown as { children?: Children })?.children;
	const isEmpty =
		isContainer &&
		(!childData ||
			("explicitList" in childData && childData.explicitList.length === 0));
	const parentId = useMemo(
		() => findComponentParent(components, componentId),
		[components, componentId],
	);
	const {
		attributes,
		listeners,
		setNodeRef: setDragRef,
		setActivatorNodeRef,
		isDragging,
	} = useDraggable({
		id: `move-${componentId}`,
		disabled: isRoot || !canDrag,
		data: {
			type: COMPONENT_MOVE_TYPE,
			componentId,
			currentParentId: parentId,
		} satisfies ComponentMoveData,
	});
	const { setNodeRef: setDropRef } = useDroppable({
		id: `container-${componentId}`,
		disabled: !isContainer,
		data: {
			type: "container",
			parentId: componentId,
			isContainer: true,
		} satisfies DropData,
	});
	const elementRef = useCallback(
		(node: HTMLElement | SVGElement | null) => {
			if (node) {
				node.setAttribute("data-builder-component", componentId);
				node.setAttribute("data-component-type", component.type);
				node.toggleAttribute("data-builder-empty", isEmpty);
			}
			setElement(node);
			setDragRef(node as HTMLElement | null);
			setDropRef(node as HTMLElement | null);
		},
		[componentId, component?.type, isEmpty, setDragRef, setDropRef],
	);

	// Capture clicks on the real element before its runtime handlers run.
	useLayoutEffect(() => {
		if (!element) return;
		element.setAttribute("data-builder-component", componentId);
		element.setAttribute("data-component-type", component.type);
		element.toggleAttribute("data-builder-empty", isEmpty);
		const handleClick = (event: MouseEvent) => {
			const target = event.target as Element | null;
			if (target?.closest?.("[data-builder-component]") !== element) return;
			event.preventDefault();
			event.stopPropagation();
			selectComponent(
				componentId,
				event.shiftKey || event.metaKey || event.ctrlKey,
			);
		};
		const handleHover = (event: PointerEvent) => {
			const target = event.target as Element | null;
			setIsHovered(target?.closest?.("[data-builder-component]") === element);
		};
		const handleLeave = () => setIsHovered(false);
		element.addEventListener("click", handleClick as EventListener, true);
		element.addEventListener("pointerover", handleHover as EventListener);
		element.addEventListener("pointerleave", handleLeave);
		return () => {
			element.removeEventListener("click", handleClick as EventListener, true);
			element.removeEventListener("pointerover", handleHover as EventListener);
			element.removeEventListener("pointerleave", handleLeave);
			element.removeAttribute("data-builder-component");
			element.removeAttribute("data-component-type");
			element.removeAttribute("data-builder-empty");
		};
	}, [element, componentId, component.type, isEmpty, selectComponent]);

	const showChrome = !isHidden && (isSelected || isHovered || isEmpty);
	// biome-ignore lint/correctness/useExhaustiveDependencies: Moving another tree node can reposition an empty hint without resizing its element.
	useEffect(() => {
		if (!element || !showChrome) return;
		const view = element.ownerDocument.defaultView;
		if (!view) return;
		let frame = 0;
		const measure = () => {
			const rect = getBuilderElementRectangle(element);
			const clip = canvasRef?.current
				? getCanvasClip(canvasRef.current)
				: "inset(0)";
			const viewport = canvasRef?.current
				? getCanvasViewport(canvasRef.current)
				: { left: 0, top: 0, width: view.innerWidth, height: view.innerHeight };
			setGeometry((previous) =>
				previous &&
				previous.rect.left === rect.left &&
				previous.rect.top === rect.top &&
				previous.rect.width === rect.width &&
				previous.rect.height === rect.height &&
				previous.clip === clip &&
				previous.viewport.width === viewport.width &&
				previous.viewport.height === viewport.height
					? previous
					: { rect, clip, viewport },
			);
			if (isSelected || isHovered) frame = view.requestAnimationFrame(measure);
		};
		measure();
		if (isSelected || isHovered) return () => view.cancelAnimationFrame(frame);
		const observer = new view.ResizeObserver(measure);
		observer.observe(element);
		if (canvasRef?.current) observer.observe(canvasRef.current);
		view.addEventListener("scroll", measure, true);
		view.addEventListener("resize", measure);
		return () => {
			observer.disconnect();
			view.removeEventListener("scroll", measure, true);
			view.removeEventListener("resize", measure);
		};
	}, [element, showChrome, canvasRef, isSelected, isHovered, components]);

	const Renderer = component ? getComponentRenderer(component.type) : null;
	if (!component || !Renderer || isHidden) return null;
	const props: ComponentProps = {
		component,
		componentId,
		surfaceId,
		appId: actionContext?.appId,
		boardId: actionContext?.boardId,
		style: style ?? component.style,
		elementRef,
		onAction: () => {},
		renderChild,
	};
	const ownerId =
		element
			?.closest("[data-builder-root]")
			?.getAttribute("data-builder-root") ?? undefined;
	return (
		<>
			<Renderer {...props} />
			{element &&
				geometry &&
				showChrome &&
				createPortal(
					<div
						data-builder-chrome=""
						data-builder-owner={ownerId}
						className="fixed inset-0 pointer-events-none z-30"
						style={{ clipPath: geometry.clip }}
					>
						<div className="absolute pointer-events-none" style={geometry.rect}>
							{(isSelected || isHovered) && (
								<div
									className={cn(
										"absolute inset-0 pointer-events-none rounded",
										isDragging && "opacity-30",
										isSelected
											? "border-2 border-dotted border-foreground"
											: "border border-dotted border-foreground/40",
									)}
								/>
							)}
							{isEmpty && (
								<button
									type="button"
									className="absolute inset-0 flex items-center justify-center rounded border border-dashed border-muted-foreground/20 bg-transparent pointer-events-auto"
									onClick={(event) => {
										event.stopPropagation();
										selectComponent(
											componentId,
											event.shiftKey || event.metaKey || event.ctrlKey,
										);
									}}
								>
									<span className="text-xs text-muted-foreground/50 select-none">
										{activeId
											? t("dropHere", "Drop here")
											: t("emptyType", "Empty {{type}}", {
													type: component.type,
												})}
									</span>
								</button>
							)}
						</div>
					</div>,
					element.ownerDocument.body,
				)}
			{element &&
				geometry &&
				isPrimarySelection &&
				createPortal(
					<SelectionToolbar
						anchor={geometry.rect}
						viewport={geometry.viewport}
						ownerId={ownerId}
						componentType={component.type}
						isRoot={isRoot}
						canDrag={canDrag}
						onDelete={() => {
							if (!isRoot) deleteComponents([componentId]);
						}}
						onCopy={() => copy()}
						onCut={() => cut()}
						onPaste={() =>
							paste(isContainer ? componentId : (parentId ?? undefined))
						}
						dragHandleRef={setActivatorNodeRef}
						dragAttributes={
							attributes as React.HTMLAttributes<HTMLButtonElement>
						}
						dragListeners={
							listeners as React.DOMAttributes<HTMLButtonElement> | undefined
						}
					/>,
					element.ownerDocument.body,
				)}
		</>
	);
}

function CanvasDropIndicator() {
	const { overData } = useBuilderDnd();
	const canvasRef = useContext(CanvasElementContext);
	const canvas = canvasRef?.current;
	if (!canvas || !overData?.indicator) return null;
	return createPortal(
		<div
			data-builder-drop-indicator=""
			className="fixed inset-0 z-50 pointer-events-none"
			style={{ clipPath: getCanvasClip(canvas) }}
		>
			<div
				className="absolute bg-primary rounded-full"
				style={overData.indicator}
			/>
		</div>,
		canvas.ownerDocument.body,
	);
}

export function BuilderRenderer({ surface, className }: BuilderRendererProps) {
	const { t } = useTranslation("flow");
	const { actionContext, widgetRefs } = useBuilder();
	const canvasRef = useRef<HTMLDivElement>(null);
	useRuntimeTailwindStyles(canvasRef);
	const allComponents = useMemo(
		() => surface.components ?? {},
		[surface.components],
	);
	const handleAction = useCallback((_message: A2UIClientMessage) => {}, []);
	const renderChild = useCallback(
		(childId: string, dataScope?: DataScope): ReactNode => {
			const comp = allComponents[childId];
			if (!comp) return null;
			const node = (
				<BuilderComponent
					key={childId}
					componentId={childId}
					surfaceComponent={comp}
					surfaceId={surface.id}
					renderChild={(id, childScope) =>
						renderChild(id, childScope ?? dataScope)
					}
				/>
			);
			return dataScope ? (
				<DataScopeProvider scope={dataScope}>{node}</DataScopeProvider>
			) : (
				node
			);
		},
		[allComponents, surface.id],
	);
	if (!allComponents[surface.rootComponentId])
		return (
			<div
				className={cn(
					"flex items-center justify-center h-full text-muted-foreground",
					className,
				)}
			>
				{t("noContentToDisplay", "No content to display")}
			</div>
		);
	return (
		<DataProvider initialData={surface.dataModel ?? []}>
			<WidgetRefsProvider widgetRefs={widgetRefs}>
				<ActionProvider
					onAction={handleAction}
					surfaceId={surface.id}
					appId={actionContext?.appId}
					boardId={actionContext?.boardId}
					eventId={actionContext?.eventId}
					components={allComponents}
					isPreviewMode={false}
				>
					<CanvasElementContext.Provider value={canvasRef}>
						<div
							ref={canvasRef}
							className={cn(
								"isolate min-h-full w-full [&_iframe]:pointer-events-none",
								className,
							)}
						>
							{renderChild(surface.rootComponentId)}
						</div>
						<CanvasDropIndicator />
					</CanvasElementContext.Provider>
				</ActionProvider>
			</WidgetRefsProvider>
		</DataProvider>
	);
}

export default BuilderRenderer;
