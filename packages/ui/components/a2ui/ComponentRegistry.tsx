"use client";

import type { ComponentType } from "react";
import type { A2UIClientMessage, A2UIComponent, Style } from "./types";

// Layout components
import {
	A2UIAbsolute,
	A2UIAspectRatio,
	A2UIBox,
	A2UICenter,
	A2UIColumn,
	A2UIGrid,
	A2UIOverlay,
	A2UIRow,
	A2UIScrollArea,
	A2UISpacer,
	A2UIStack,
	A2UIWidgetInstance,
} from "./layout";

// Display components
import {
	A2UIAvatar,
	A2UIBadge,
	A2UIBoundingBoxOverlay,
	A2UIDivider,
	A2UIFilePreview,
	A2UIGeoMap,
	A2UIIcon,
	A2UIIframe,
	A2UIImage,
	A2UILottie,
	A2UIMarkdown,
	A2UINivoChart,
	A2UIPlotlyChart,
	A2UIProgress,
	A2UISkeleton,
	A2UISpinner,
	A2UITable,
	A2UITableCell,
	A2UITableRow,
	A2UIText,
	A2UIVideo,
} from "./display";

// Interactive components
import {
	A2UIButton,
	A2UICheckbox,
	A2UIDateTimeInput,
	A2UIFileInput,
	A2UIImageHotspot,
	A2UIImageInput,
	A2UIImageLabeler,
	A2UILink,
	A2UIRadioGroup,
	A2UISelect,
	A2UISlider,
	A2UISwitch,
	A2UITextField,
	A2UIVoiceInput,
} from "./interactive";

// Container components
import {
	A2UIAccordion,
	A2UICard,
	A2UIDrawer,
	A2UIModal,
	A2UIPopover,
	A2UITabs,
	A2UITooltip,
} from "./container";

// Game components
import {
	A2UICanvas2D,
	A2UICharacterPortrait,
	A2UIChoiceMenu,
	A2UIDialogue,
	A2UIHealthBar,
	A2UIInventoryGrid,
	A2UIMiniMap,
	A2UIModel3D,
	A2UIScene3D,
	A2UIShape,
	A2UISprite,
} from "./game";

export type RenderChildFn = (childId: string) => React.ReactNode;

export interface ComponentProps<T extends A2UIComponent = A2UIComponent> {
	component: T;
	componentId: string;
	surfaceId: string;
	style?: Style;
	onAction?: (message: A2UIClientMessage) => void;
	renderChild: RenderChildFn;
}

type ComponentRenderer = ComponentType<ComponentProps>;

const registry: Record<string, ComponentRenderer> = {
	// Layout
	row: A2UIRow as ComponentRenderer,
	column: A2UIColumn as ComponentRenderer,
	stack: A2UIStack as ComponentRenderer,
	grid: A2UIGrid as ComponentRenderer,
	scrollArea: A2UIScrollArea as ComponentRenderer,
	aspectRatio: A2UIAspectRatio as ComponentRenderer,
	overlay: A2UIOverlay as ComponentRenderer,
	absolute: A2UIAbsolute as ComponentRenderer,
	widgetInstance: A2UIWidgetInstance as ComponentRenderer,
	box: A2UIBox as ComponentRenderer,
	center: A2UICenter as ComponentRenderer,
	spacer: A2UISpacer as ComponentRenderer,

	// Display
	text: A2UIText as ComponentRenderer,
	image: A2UIImage as ComponentRenderer,
	icon: A2UIIcon as ComponentRenderer,
	video: A2UIVideo as ComponentRenderer,
	markdown: A2UIMarkdown as ComponentRenderer,
	divider: A2UIDivider as ComponentRenderer,
	badge: A2UIBadge as ComponentRenderer,
	avatar: A2UIAvatar as ComponentRenderer,
	progress: A2UIProgress as ComponentRenderer,
	spinner: A2UISpinner as ComponentRenderer,
	skeleton: A2UISkeleton as ComponentRenderer,
	lottie: A2UILottie as ComponentRenderer,
	iframe: A2UIIframe as ComponentRenderer,
	plotlyChart: A2UIPlotlyChart as ComponentRenderer,
	table: A2UITable as ComponentRenderer,
	tableRow: A2UITableRow as ComponentRenderer,
	tableCell: A2UITableCell as ComponentRenderer,
	filePreview: A2UIFilePreview as ComponentRenderer,
	nivoChart: A2UINivoChart as ComponentRenderer,
	boundingBoxOverlay: A2UIBoundingBoxOverlay as ComponentRenderer,
	geoMap: A2UIGeoMap as ComponentRenderer,

	// Interactive
	button: A2UIButton as ComponentRenderer,
	textField: A2UITextField as ComponentRenderer,
	select: A2UISelect as ComponentRenderer,
	slider: A2UISlider as ComponentRenderer,
	checkbox: A2UICheckbox as ComponentRenderer,
	switch: A2UISwitch as ComponentRenderer,
	radioGroup: A2UIRadioGroup as ComponentRenderer,
	dateTimeInput: A2UIDateTimeInput as ComponentRenderer,
	fileInput: A2UIFileInput as ComponentRenderer,
	imageLabeler: A2UIImageLabeler as ComponentRenderer,
	imageHotspot: A2UIImageHotspot as ComponentRenderer,
	imageInput: A2UIImageInput as ComponentRenderer,
	voiceInput: A2UIVoiceInput as ComponentRenderer,
	link: A2UILink as ComponentRenderer,

	// Container
	card: A2UICard as ComponentRenderer,
	modal: A2UIModal as ComponentRenderer,
	tabs: A2UITabs as ComponentRenderer,
	accordion: A2UIAccordion as ComponentRenderer,
	drawer: A2UIDrawer as ComponentRenderer,
	tooltip: A2UITooltip as ComponentRenderer,
	popover: A2UIPopover as ComponentRenderer,

	// Game
	canvas2d: A2UICanvas2D as ComponentRenderer,
	sprite: A2UISprite as ComponentRenderer,
	shape: A2UIShape as ComponentRenderer,
	scene3d: A2UIScene3D as ComponentRenderer,
	model3d: A2UIModel3D as ComponentRenderer,
	dialogue: A2UIDialogue as ComponentRenderer,
	characterPortrait: A2UICharacterPortrait as ComponentRenderer,
	choiceMenu: A2UIChoiceMenu as ComponentRenderer,
	inventoryGrid: A2UIInventoryGrid as ComponentRenderer,
	healthBar: A2UIHealthBar as ComponentRenderer,
	miniMap: A2UIMiniMap as ComponentRenderer,
};

export function getComponentRenderer(type: string): ComponentRenderer | null {
	return registry[type] ?? null;
}

export function registerComponent(
	type: string,
	renderer: ComponentRenderer,
): void {
	registry[type] = renderer;
}

export function getRegisteredTypes(): string[] {
	return Object.keys(registry);
}
