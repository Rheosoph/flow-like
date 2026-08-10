"use client";

import { type ComponentType, Suspense, lazy } from "react";
import type {
	A2UIClientMessage,
	A2UIComponent,
	DataScope,
	Style,
} from "./types";

import { A2UIMicroWidget } from "./layout/A2UIMicroWidget";
import { A2UIWidgetInstance } from "./layout/A2UIWidgetInstance";
// Imported by module path rather than through the folder barrels: a barrel re-export makes
// every component in the folder reachable from one specifier, which is exactly what put the
// 3D renderer and the mapping stack into the first load of pages that use neither.
import { A2UIAbsolute } from "./layout/Absolute";
import { A2UIAspectRatio } from "./layout/AspectRatio";
import { A2UIBox } from "./layout/Box";
import { A2UICenter } from "./layout/Center";
import { A2UIColumn } from "./layout/Column";
import { A2UIGrid } from "./layout/Grid";
import { A2UIOverlay } from "./layout/Overlay";
import { A2UIRow } from "./layout/Row";
import { A2UIScrollArea } from "./layout/ScrollArea";
import { A2UISpacer } from "./layout/Spacer";
import { A2UIStack } from "./layout/Stack";

import { A2UIAvatar } from "./display/Avatar";
import { A2UIBadge } from "./display/Badge";
import { A2UIBoundingBoxOverlay } from "./display/BoundingBoxOverlay";
import { A2UIDivider } from "./display/Divider";
import { A2UIFilePreview } from "./display/FilePreview";
import { A2UIIcon } from "./display/Icon";
import { A2UIIframe } from "./display/Iframe";
import { A2UIImage } from "./display/Image";
import { A2UIMarkdown } from "./display/Markdown";
import { A2UIOntologyGraph } from "./display/OntologyGraph";
import { A2UIProgress } from "./display/Progress";
import { A2UISkeleton } from "./display/Skeleton";
import { A2UISpinner } from "./display/Spinner";
import { A2UITable, A2UITableCell, A2UITableRow } from "./display/Table";
import { A2UIText } from "./display/Text";
import { A2UIUserProfile } from "./display/UserProfile";
import { A2UIVideo } from "./display/Video";

import { A2UIAppLink } from "./interactive/AppLink";
import { A2UIButton } from "./interactive/Button";
import { A2UICheckbox } from "./interactive/Checkbox";
import { A2UIDateTimeInput } from "./interactive/DateTimeInput";
import { A2UIFeedback } from "./interactive/Feedback";
import { A2UIFileInput } from "./interactive/FileInput";
import { A2UIImageHotspot } from "./interactive/ImageHotspot";
import { A2UIImageInput } from "./interactive/ImageInput";
import { A2UIImageLabeler } from "./interactive/ImageLabeler";
import { A2UILink } from "./interactive/Link";
import { A2UIRadioGroup } from "./interactive/RadioGroup";
import { A2UISelect } from "./interactive/Select";
import { A2UISlider } from "./interactive/Slider";
import { A2UISwitch } from "./interactive/Switch";
import { A2UITextField } from "./interactive/TextField";
import { A2UIVoiceInput } from "./interactive/VoiceInput";

import { A2UIAccordion } from "./container/Accordion";
import { A2UICard } from "./container/Card";
import { A2UIDrawer } from "./container/Drawer";
import { A2UIModal } from "./container/Modal";
import { A2UIPopover } from "./container/Popover";
import { A2UITabs } from "./container/Tabs";
import { A2UITooltip } from "./container/Tooltip";

export type RenderChildFn = (
	childId: string,
	dataScope?: DataScope,
) => React.ReactNode;

export interface ComponentProps<T extends A2UIComponent = A2UIComponent> {
	component: T;
	componentId: string;
	surfaceId: string;
	appId?: string;
	boardId?: string;
	style?: Style;
	onAction?: (message: A2UIClientMessage) => void;
	renderChild: RenderChildFn;
}

type ComponentRenderer = ComponentType<ComponentProps>;

/**
 * Registers a component that is fetched the first time a surface actually contains one.
 *
 * The heavy renderers — charts, maps, graphs, the 3D scene — are rare on any given page but
 * expensive in every bundle that can reach them. Each already renders nothing until its own
 * library finishes loading, so an empty frame while the module arrives is the behaviour they
 * had anyway; the Suspense boundary lives here so `A2UIRenderer` stays unaware of the split.
 */
function lazyRenderer(
	loader: () => Promise<{ default: ComponentRenderer }>,
): ComponentRenderer {
	const Loaded = lazy(loader);
	return function LazyRenderer(props: ComponentProps) {
		return (
			<Suspense fallback={null}>
				<Loaded {...props} />
			</Suspense>
		);
	};
}

/**
 * Each component declares its own narrower prop type, exactly as the eagerly registered ones
 * do, so the cast that the registry applies to them is applied here too.
 */
function named(name: string) {
	return (module: Record<string, unknown>) => ({
		default: module[name] as ComponentRenderer,
	});
}

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
	microWidgetInstance: A2UIMicroWidget as ComponentRenderer,
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
	userProfile: A2UIUserProfile as ComponentRenderer,
	progress: A2UIProgress as ComponentRenderer,
	spinner: A2UISpinner as ComponentRenderer,
	skeleton: A2UISkeleton as ComponentRenderer,
	lottie: lazyRenderer(() =>
		import("./display/Lottie").then(named("A2UILottie")),
	),
	iframe: A2UIIframe as ComponentRenderer,
	plotlyChart: lazyRenderer(() =>
		import("./display/PlotlyChart").then(named("A2UIPlotlyChart")),
	),
	table: A2UITable as ComponentRenderer,
	tableRow: A2UITableRow as ComponentRenderer,
	tableCell: A2UITableCell as ComponentRenderer,
	filePreview: A2UIFilePreview as ComponentRenderer,
	diffView: lazyRenderer(() =>
		import("./display/DiffView").then(named("A2UIDiffView")),
	),
	nivoChart: lazyRenderer(() =>
		import("./display/NivoChart").then(named("A2UINivoChart")),
	),
	boundingBoxOverlay: A2UIBoundingBoxOverlay as ComponentRenderer,
	geoMap: lazyRenderer(() =>
		import("./display/GeoMap").then(named("A2UIGeoMap")),
	),
	graph: lazyRenderer(() => import("./display/Graph").then(named("A2UIGraph"))),
	// Registered eagerly on purpose: the module is a thin wrapper that defers the
	// sigma chunk itself, behind a fallback that keeps the element's own height.
	// Loading it lazily instead would collapse the element to nothing first.
	ontologyGraph: A2UIOntologyGraph as ComponentRenderer,
	calendar: lazyRenderer(() =>
		import("./display/Calendar").then(named("A2UICalendar")),
	),
	gantt: lazyRenderer(() =>
		import("./display/GanttChart").then(named("A2UIGantt")),
	),

	// Interactive
	button: A2UIButton as ComponentRenderer,
	feedback: A2UIFeedback as ComponentRenderer,
	appLink: A2UIAppLink as ComponentRenderer,
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

	// Game — a niche group that reaches three.js through the 3D scene, so the whole set loads
	// on demand rather than riding along in every page's bundle.
	canvas2d: lazyRenderer(() =>
		import("./game/Canvas2D").then(named("A2UICanvas2D")),
	),
	sprite: lazyRenderer(() => import("./game/Sprite").then(named("A2UISprite"))),
	shape: lazyRenderer(() => import("./game/Shape").then(named("A2UIShape"))),
	scene3d: lazyRenderer(() =>
		import("./game/Scene3D").then(named("A2UIScene3D")),
	),
	model3d: lazyRenderer(() =>
		import("./game/Model3D").then(named("A2UIModel3D")),
	),
	dialogue: lazyRenderer(() =>
		import("./game/Dialogue").then(named("A2UIDialogue")),
	),
	characterPortrait: lazyRenderer(() =>
		import("./game/CharacterPortrait").then(named("A2UICharacterPortrait")),
	),
	choiceMenu: lazyRenderer(() =>
		import("./game/ChoiceMenu").then(named("A2UIChoiceMenu")),
	),
	inventoryGrid: lazyRenderer(() =>
		import("./game/InventoryGrid").then(named("A2UIInventoryGrid")),
	),
	healthBar: lazyRenderer(() =>
		import("./game/HealthBar").then(named("A2UIHealthBar")),
	),
	miniMap: lazyRenderer(() =>
		import("./game/MiniMap").then(named("A2UIMiniMap")),
	),
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
