// A2UI runtime JSON definitions. Deprecated fields keep older Rust payloads readable.

import type { WidgetContract } from "@flow-like/widget-sdk";

export interface SelectOption {
	value: string;
	label: string;
}

export type BoundValue =
	| { literalString: string }
	| { literalNumber: number }
	| { literalBool: boolean }
	| { literalOptions: SelectOption[] }
	| { literalJson: string }
	| { path: string; defaultValue?: unknown };

export interface Action {
	name: string;
	context: Record<string, unknown>;
}

export type EventHandlers = Record<string, Action[]>;

export type Children =
	| { explicitList: string[] }
	| { template: ChildrenTemplate };

export interface ChildrenTemplate {
	dataPath: string;
	itemIdPath?: string;
	templateComponentId: string;
}

export interface DataScope {
	dataPath?: string;
	index?: number;
	item?: unknown;
	itemId?: string;
}

export interface Style {
	className?: string;
	background?: Background;
	border?: Border;
	shadow?: Shadow;
	position?: Position;
	transform?: Transform;
	overflow?: Overflow;
	responsiveOverrides?: ResponsiveOverrides;
	/** @deprecated Rust emitted this name before the JSON contract was aligned. */
	responsive?: ResponsiveOverrides;
	// Spacing
	margin?: Spacing;
	padding?: Spacing;
	gap?: string;
	// Sizing
	width?: StyleValue;
	height?: StyleValue;
	minWidth?: StyleValue;
	minHeight?: StyleValue;
	maxWidth?: StyleValue;
	maxHeight?: StyleValue;
	// Flex item properties
	flex?: string;
	flexGrow?: number;
	flexShrink?: number;
	flexBasis?: string;
	alignSelf?: "auto" | "start" | "end" | "center" | "stretch" | "baseline";
	// Grid item properties
	gridColumn?: string;
	gridRow?: string;
	gridArea?: string;
	justifySelf?: "auto" | "start" | "end" | "center" | "stretch";
	// Typography
	color?: string;
	fontSize?: string;
	fontWeight?: string;
	fontFamily?: string;
	lineHeight?: string;
	letterSpacing?: string;
	textAlign?: "left" | "center" | "right" | "justify";
	textDecoration?: string;
	textTransform?: "none" | "uppercase" | "lowercase" | "capitalize";
	whiteSpace?: "normal" | "nowrap" | "pre" | "pre-wrap" | "pre-line";
	wordBreak?: "normal" | "break-all" | "keep-all" | "break-word";
	// Visibility & interaction
	opacity?: number;
	visibility?: "visible" | "hidden" | "collapse";
	cursor?: string;
	userSelect?: "none" | "auto" | "text" | "all";
	pointerEvents?: "auto" | "none";
	// Stacking
	zIndex?: number;
	// Transitions & animations
	transition?: string;
	animation?: string;
	// Display
	display?:
		| "block"
		| "inline"
		| "inline-block"
		| "flex"
		| "inline-flex"
		| "grid"
		| "inline-grid"
		| "none"
		| "contents";
	// Outline (for focus states)
	outline?: string;
	outlineOffset?: string;
	// Filters
	filter?: string;
	backdropFilter?: string;
	// Aspect ratio
	aspectRatio?: string;
}

/** CSS length/value. The object form is accepted for legacy Rust payloads. */
export type StyleValue = string | { value: string };

export type Spacing =
	| {
			top?: string;
			right?: string;
			bottom?: string;
			left?: string;
			value?: undefined;
	  }
	| {
			/** @deprecated Rust emitted CSS shorthand in this wrapper. */
			value: string;
			top?: undefined;
			right?: undefined;
			bottom?: undefined;
			left?: undefined;
	  };

export type Background =
	| { color: string }
	| { gradient: Gradient }
	| { image: BackgroundImage }
	| { blur: string };

export type GradientType = "linear" | "radial" | "conic";

export type Gradient =
	| {
			type: GradientType;
			angle?: number;
			direction?: string;
			stops: GradientStop[];
			/** @deprecated Rust emitted this name before the JSON contract was aligned. */
			gradientType?: GradientType;
	  }
	| {
			/** @deprecated Rust compatibility shape. Use `type`. */
			gradientType: GradientType;
			type?: undefined;
			angle?: number;
			direction?: string;
			stops: GradientStop[];
	  };

export interface GradientStop {
	color: string;
	/** Percentage (0-100). Legacy Rust payloads may contain a 0-1 fraction. */
	position?: number;
}

export interface BackgroundImage {
	url: BoundValue;
	size?: string;
	position?: string;
	repeat?: string;
}

export interface Border {
	width?: string;
	style?: string;
	color?: string;
	radius?: string;
}

export interface Shadow {
	x?: string;
	y?: string;
	blur?: string;
	spread?: string;
	color?: string;
	inset?: boolean;
	/** @deprecated Rust compatibility shape. */
	boxShadows?: string[];
	/** Supported as an optional richer shadow effect. */
	textShadow?: string;
}

export type PositionType = "absolute" | "relative" | "fixed" | "sticky";

interface PositionOffsets {
	top?: string;
	right?: string;
	bottom?: string;
	left?: string;
}

export type Position =
	| (PositionOffsets & {
			type: PositionType;
			/** @deprecated Rust compatibility field. */
			positionType?: PositionType;
	  })
	| (PositionOffsets & {
			/** @deprecated Rust compatibility shape. Use `type`. */
			positionType: PositionType;
			type?: undefined;
	  });

export interface Transform {
	translate?: string;
	rotate?: number;
	scale?: string;
	transformOrigin?: string;
	skew?: string;
}

export type Overflow = "visible" | "hidden" | "scroll" | "auto";

export interface ResponsiveOverrides {
	sm?: BreakpointStyle;
	md?: BreakpointStyle;
	lg?: BreakpointStyle;
	xl?: BreakpointStyle;
	xxl?: BreakpointStyle;
}

export interface BreakpointStyle {
	className?: string;
	display?: string;
	flexDirection?: string;
	justifyContent?: string;
	alignItems?: string;
	gap?: string;
	gridCols?: number;
	width?: StyleValue;
	height?: StyleValue;
	padding?: Spacing;
	margin?: Spacing;
	hidden?: boolean;
	fontSize?: string;
	textAlign?: string;
	order?: number;
}

export interface Size {
	width?: string;
	height?: string;
	minWidth?: string;
	maxWidth?: string;
	minHeight?: string;
	maxHeight?: string;
}

// Component definitions
export interface ComponentBase {
	id: string;
	style?: Style;
	children?: Children;
	actions?: Action[];
	/** Ordered actions bound to named component events. Legacy `actions` remains the fallback. */
	eventHandlers?: EventHandlers;
	hidden?: BoundValue | boolean;
}

// Layout components
export interface RowComponent extends ComponentBase {
	type: "row";
	gap?: BoundValue;
	align?: BoundValue; // "start" | "center" | "end" | "stretch" | "baseline"
	justify?: BoundValue; // "start" | "center" | "end" | "between" | "around" | "evenly"
	wrap?: BoundValue;
	reverse?: BoundValue;
}

export interface ColumnComponent extends ComponentBase {
	type: "column";
	gap?: BoundValue;
	align?: BoundValue; // "start" | "center" | "end" | "stretch" | "baseline"
	justify?: BoundValue; // "start" | "center" | "end" | "between" | "around" | "evenly"
	reverse?: BoundValue;
	wrap?: BoundValue;
}

export interface StackComponent extends ComponentBase {
	type: "stack";
	align?: BoundValue; // "start" | "center" | "end" | "stretch"
	width?: BoundValue;
	height?: BoundValue;
}

export interface GridComponent extends ComponentBase {
	type: "grid";
	columns?: BoundValue;
	rows?: BoundValue;
	gap?: BoundValue;
	columnGap?: BoundValue;
	rowGap?: BoundValue;
	autoFlow?: BoundValue; // "row" | "column" | "dense" | "rowDense" | "columnDense"
}

export interface ScrollAreaComponent extends ComponentBase {
	type: "scrollArea";
	direction?: BoundValue; // "vertical" | "horizontal" | "both"
}

export interface AspectRatioComponent extends ComponentBase {
	type: "aspectRatio";
	ratio: BoundValue;
}

export interface OverlayItem {
	componentId: string;
	anchor?: BoundValue; // "topLeft" | "topCenter" | "topRight" | "centerLeft" | "center" | "centerRight" | "bottomLeft" | "bottomCenter" | "bottomRight"
	offsetX?: BoundValue;
	offsetY?: BoundValue;
	zIndex?: BoundValue;
}

export interface OverlayComponent extends ComponentBase {
	type: "overlay";
	baseComponentId: string;
	overlays: OverlayItem[];
}

export interface AbsoluteComponent extends ComponentBase {
	type: "absolute";
	width?: BoundValue;
	height?: BoundValue;
}

export interface BoxComponent extends ComponentBase {
	type: "box";
	/** Resolves to one of the allowlisted semantic elements; invalid values render as div. */
	as?: BoundValue;
}

export interface CenterComponent extends ComponentBase {
	type: "center";
	inline?: BoundValue;
}

export interface SpacerComponent extends ComponentBase {
	type: "spacer";
	size?: BoundValue; // Fixed size (e.g., "20px")
	flex?: BoundValue; // Flex grow value
}

// Display components
export interface TextComponent extends ComponentBase {
	type: "text";
	content: BoundValue;
	variant?: BoundValue; // "body" | "heading" | "label" | "caption" | "code"
	size?: BoundValue; // "xs" | "sm" | "md" | "lg" | "xl" | "2xl" | "3xl" | "4xl"
	weight?: BoundValue; // "light" | "normal" | "medium" | "semibold" | "bold"
	color?: BoundValue;
	align?: BoundValue; // "left" | "center" | "right" | "justify"
	truncate?: BoundValue;
	maxLines?: BoundValue;
}

export interface ImageComponent extends ComponentBase {
	type: "image";
	src: BoundValue;
	alt?: BoundValue;
	fit?: BoundValue; // "contain" | "cover" | "fill" | "none" | "scaleDown"
	fallback?: BoundValue;
	loading?: BoundValue; // "lazy" | "eager"
	aspectRatio?: BoundValue;
}

export interface IconComponent extends ComponentBase {
	type: "icon";
	name: BoundValue;
	size?: BoundValue;
	color?: BoundValue;
	strokeWidth?: BoundValue;
}

export interface VideoComponent extends ComponentBase {
	type: "video";
	src: BoundValue;
	poster?: BoundValue;
	autoplay?: BoundValue;
	loop?: BoundValue;
	muted?: BoundValue;
	controls?: BoundValue;
	width?: BoundValue;
	height?: BoundValue;
}

export interface LottieComponent extends ComponentBase {
	type: "lottie";
	src: BoundValue;
	autoplay?: BoundValue;
	loop?: BoundValue;
	speed?: BoundValue;
	width?: BoundValue;
	height?: BoundValue;
}

export interface MarkdownComponent extends ComponentBase {
	type: "markdown";
	content: BoundValue;
	allowHtml?: BoundValue;
}

export interface DividerComponent extends ComponentBase {
	type: "divider";
	orientation?: BoundValue; // "horizontal" | "vertical"
	thickness?: BoundValue;
	color?: BoundValue;
}

export interface BadgeComponent extends ComponentBase {
	type: "badge";
	content: BoundValue;
	variant?: BoundValue; // "default" | "secondary" | "destructive" | "outline"
	color?: BoundValue;
}

export interface AvatarComponent extends ComponentBase {
	type: "avatar";
	src?: BoundValue;
	fallback?: BoundValue;
	size?: BoundValue; // "sm" | "md" | "lg" | "xl"
}

export interface UserProfileComponent extends ComponentBase {
	type: "userProfile";
	/**
	 * User subject/sub ID. Compatible with Set Element Value via component.value.
	 * The "local" sub of an unauthenticated execution renders the current user.
	 */
	value: BoundValue;
	/** "avatar" | "chip" | "row" | "detailed" | "card" */
	variant?: BoundValue;
	/** "xs" | "sm" | "md" | "lg" | "xl" | "2xl" */
	avatarSize?: BoundValue;
	showHover?: BoundValue;
	showEmail?: BoundValue;
	showDescription?: BoundValue;
	showUserId?: BoundValue;
	showProfileLink?: BoundValue;
	fallbackLabel?: BoundValue;
	muted?: BoundValue;
}

export interface ProgressComponent extends ComponentBase {
	type: "progress";
	value: BoundValue;
	max?: BoundValue;
	showLabel?: BoundValue;
	variant?: BoundValue; // "default" | "success" | "warning" | "error"
	color?: BoundValue;
}

export interface SpinnerComponent extends ComponentBase {
	type: "spinner";
	size?: BoundValue; // "sm" | "md" | "lg"
	color?: BoundValue;
}

export interface SkeletonComponent extends ComponentBase {
	type: "skeleton";
	width?: BoundValue;
	height?: BoundValue;
	rounded?: BoundValue;
}

// Table components
export interface TableColumn {
	id: string;
	header: BoundValue;
	accessor?: BoundValue; // Path to data in row object
	width?: BoundValue;
	align?: BoundValue; // "left" | "center" | "right"
	sortable?: BoundValue;
	hidden?: BoundValue;
}

export interface TableComponent extends ComponentBase {
	type: "table";
	columns: BoundValue; // Array of TableColumn
	data: BoundValue; // Array of row objects
	caption?: BoundValue;
	striped?: BoundValue;
	bordered?: BoundValue;
	hoverable?: BoundValue;
	compact?: BoundValue;
	stickyHeader?: BoundValue;
	sortable?: BoundValue;
	searchable?: BoundValue;
	paginated?: BoundValue;
	pageSize?: BoundValue;
	selectable?: BoundValue;
	onRowClick?: BoundValue;
}

export interface TableRowComponent extends ComponentBase {
	type: "tableRow";
	cells: BoundValue; // Array of cell values
	selected?: BoundValue;
	disabled?: BoundValue;
}

export interface TableCellComponent extends ComponentBase {
	type: "tableCell";
	content: BoundValue;
	isHeader?: BoundValue;
	colSpan?: BoundValue;
	rowSpan?: BoundValue;
	align?: BoundValue;
}

// Interactive components
export interface ButtonComponent extends ComponentBase {
	type: "button";
	label: BoundValue;
	variant?: BoundValue; // "default" | "secondary" | "outline" | "ghost" | "destructive" | "link"
	size?: BoundValue; // "sm" | "md" | "lg" | "icon"
	disabled?: BoundValue;
	loading?: BoundValue;
	icon?: BoundValue;
	iconPosition?: BoundValue; // "left" | "right"
	tooltip?: BoundValue;
}

export interface FeedbackComponent extends ComponentBase {
	type: "feedback";
	mode?: BoundValue; // "icon" | "compact" | "segmented" | "rating" | "extended"
	size?: BoundValue; // "sm" | "md" | "lg"
	title?: BoundValue;
	description?: BoundValue;
	positiveLabel?: BoundValue;
	negativeLabel?: BoundValue;
	positiveRating?: BoundValue;
	negativeRating?: BoundValue;
	showComment?: BoundValue;
	commentMode?: BoundValue; // "none" | "inline" | "modal"
	commentLabel?: BoundValue;
	commentPlaceholder?: BoundValue;
	commentTitle?: BoundValue;
	commentDescription?: BoundValue;
	commentSubmitLabel?: BoundValue;
	commentCancelLabel?: BoundValue;
	feedbackId?: BoundValue;
	includeState?: BoundValue;
	pageContextMode?: BoundValue; // "none" | "path" | "query"
	pageContextQueryParamAllowlist?: BoundValue; // comma-separated query param names
	pageContextQueryParamDenylist?: BoundValue; // comma-separated query param names
	includePageHash?: BoundValue;
	successMessage?: BoundValue;
	disabled?: BoundValue;
}

export interface AppLinkComponent extends ComponentBase {
	type: "appLink";
	target?: BoundValue; // "config" | "settings" | "overview"
	label?: BoundValue;
	variant?: BoundValue; // "default" | "secondary" | "outline" | "ghost" | "destructive" | "link"
	size?: BoundValue; // "sm" | "md" | "lg" | "icon"
	icon?: BoundValue;
	iconPosition?: BoundValue; // "left" | "right"
	appId?: BoundValue;
	eventId?: BoundValue;
	disabled?: BoundValue;
}

export interface TextFieldComponent extends ComponentBase {
	type: "textField";
	value: BoundValue;
	placeholder?: BoundValue;
	label?: BoundValue;
	helperText?: BoundValue;
	error?: BoundValue;
	disabled?: BoundValue;
	inputType?: BoundValue; // "text" | "email" | "password" | "number" | "tel" | "url" | "search"
	multiline?: BoundValue;
	rows?: BoundValue;
	maxLength?: BoundValue;
	required?: BoundValue;
	debounceMs?: BoundValue; // pause before the "input" event fires (default 400, min 100)
}

export interface RichTextComponent extends ComponentBase {
	type: "richText";
	/** The editor document, a `plate_json::`-prefixed string. */
	value: BoundValue;
	label?: BoundValue;
	helperText?: BoundValue;
	placeholder?: BoundValue;
	error?: BoundValue;
	disabled?: BoundValue;
	readOnly?: BoundValue;
	/** Storage folder that pasted or dropped images are uploaded into. */
	uploadPrefix?: BoundValue;
	/** "app" (shared storage, default) or "user" (the viewer's private area). */
	uploadScope?: BoundValue;
	minHeight?: BoundValue;
	maxHeight?: BoundValue;
	/** Pause before the "change" event fires (default 600, min 100). */
	debounceMs?: BoundValue;
}

export interface SelectComponent extends ComponentBase {
	type: "select";
	value: BoundValue;
	options: BoundValue;
	placeholder?: BoundValue;
	label?: BoundValue;
	disabled?: BoundValue;
	multiple?: BoundValue;
	searchable?: BoundValue;
}

export interface SliderComponent extends ComponentBase {
	type: "slider";
	value: BoundValue;
	min?: BoundValue;
	max?: BoundValue;
	step?: BoundValue;
	disabled?: BoundValue;
	showValue?: BoundValue;
	label?: BoundValue;
	debounceMs?: BoundValue; // pause before the "input" event fires (default 400, min 100)
}

export interface CheckboxComponent extends ComponentBase {
	type: "checkbox";
	checked: BoundValue;
	label?: BoundValue;
	disabled?: BoundValue;
	indeterminate?: BoundValue;
}

export interface SwitchComponent extends ComponentBase {
	type: "switch";
	checked: BoundValue;
	label?: BoundValue;
	disabled?: BoundValue;
}

export interface RadioGroupComponent extends ComponentBase {
	type: "radioGroup";
	value: BoundValue;
	options: BoundValue;
	disabled?: BoundValue;
	orientation?: BoundValue; // "horizontal" | "vertical"
	label?: BoundValue;
}

export interface DateTimeInputComponent extends ComponentBase {
	type: "dateTimeInput";
	value: BoundValue;
	mode?: BoundValue; // "date" | "time" | "datetime"
	min?: BoundValue;
	max?: BoundValue;
	disabled?: BoundValue;
	label?: BoundValue;
}

export interface FileInputComponent extends ComponentBase {
	type: "fileInput";
	value: BoundValue;
	label?: BoundValue;
	helperText?: BoundValue;
	accept?: BoundValue;
	multiple?: BoundValue;
	maxSize?: BoundValue;
	maxFiles?: BoundValue;
	disabled?: BoundValue;
	error?: BoundValue;
}

export interface ImageInputComponent extends ComponentBase {
	type: "imageInput";
	value: BoundValue;
	label?: BoundValue;
	helperText?: BoundValue;
	accept?: BoundValue;
	multiple?: BoundValue;
	maxSize?: BoundValue;
	maxFiles?: BoundValue;
	disabled?: BoundValue;
	error?: BoundValue;
	aspectRatio?: BoundValue;
	showPreview?: BoundValue;
}

export interface VoiceInputComponent extends ComponentBase {
	type: "voiceInput";
	value: BoundValue;
	label?: BoundValue;
	helperText?: BoundValue;
	maxDuration?: BoundValue;
	autoStop?: BoundValue;
	silenceThreshold?: BoundValue;
	silenceDuration?: BoundValue;
	disabled?: BoundValue;
	error?: BoundValue;
	/** Deprecated alias for `variant`. */
	visualizer?: BoundValue;
	/** "conservative" | "waveform" | "orb" | "vortex" | "shader" | "aurora" | "pulse" */
	variant?: BoundValue;
	/** "sm" | "md" | "lg" */
	size?: BoundValue;
	/** "record" (send audio) | "stt" (send transcript text) */
	mode?: BoundValue;
	/** "manual" | "hold" | "auto" */
	invoke?: BoundValue;
	color?: BoundValue;
	recordingColor?: BoundValue;
	/** Post-input look: "player" (animated playback) | "autoplay" (play the backend-set response media immediately, for conversations) | "summary" (compact info + delete). Default "player". */
	resultMode?: BoundValue;
	/** Backend-set response media URL (e.g. via Set Media Source). In "autoplay" mode this is what plays — until it arrives the element shows a loading state. */
	src?: BoundValue;
	/** Alias for `src` (Set Media Source writes both). */
	url?: BoundValue;
}

export interface LinkComponent extends ComponentBase {
	type: "link";
	href: BoundValue;
	label?: BoundValue;
	route?: BoundValue;
	queryParams?: BoundValue;
	external?: boolean;
	target?: "_blank" | "_self" | "_parent" | "_top";
	variant?: "default" | "muted" | "primary" | "destructive";
	underline?: "always" | "hover" | "none";
	disabled?: BoundValue;
}

// Container components
export interface CardComponent extends ComponentBase {
	type: "card";
	title?: BoundValue;
	description?: BoundValue;
	footer?: BoundValue;
	hoverable?: BoundValue;
	clickable?: BoundValue;
	variant?: BoundValue; // "default" | "bordered" | "elevated"
	padding?: BoundValue;
	headerImage?: BoundValue;
	headerIcon?: BoundValue;
}

export interface ModalComponent extends ComponentBase {
	type: "modal";
	open: BoundValue;
	title?: BoundValue;
	description?: BoundValue;
	closeOnOverlay?: BoundValue;
	closeOnEscape?: BoundValue;
	showCloseButton?: BoundValue;
	size?: BoundValue; // "sm" | "md" | "lg" | "xl" | "full"
	centered?: BoundValue;
}

export interface TabsComponent extends ComponentBase {
	type: "tabs";
	value: BoundValue;
	tabs: TabDefinition[];
	orientation?: BoundValue; // "horizontal" | "vertical"
	variant?: BoundValue; // "default" | "pills" | "underline"
	listStyle?: Style;
	triggerStyle?: Style;
	contentStyle?: Style;
}

export interface TabDefinition {
	id: string;
	label: BoundValue;
	icon?: BoundValue;
	disabled?: BoundValue;
	contentComponentId: string;
}

export interface AccordionComponent extends ComponentBase {
	type: "accordion";
	items: AccordionItem[];
	multiple?: BoundValue;
	defaultExpanded?: BoundValue;
	collapsible?: BoundValue;
}

export interface AccordionItem {
	id: string;
	title: BoundValue;
	contentComponentId: string;
}

export interface DrawerComponent extends ComponentBase {
	type: "drawer";
	open: BoundValue;
	side?: BoundValue; // "left" | "right" | "top" | "bottom"
	title?: BoundValue;
	size?: BoundValue;
	overlay?: BoundValue;
	closable?: BoundValue;
}

export interface TooltipComponent extends ComponentBase {
	type: "tooltip";
	content: BoundValue;
	side?: BoundValue; // "top" | "right" | "bottom" | "left"
	delayMs?: BoundValue;
	maxWidth?: BoundValue;
}

export interface PopoverComponent extends ComponentBase {
	type: "popover";
	open?: BoundValue;
	contentComponentId: string;
	side?: BoundValue; // "top" | "right" | "bottom" | "left"
	trigger?: BoundValue; // "click" | "hover"
	closeOnClickOutside?: BoundValue;
}

// Game components
export interface Canvas2DComponent extends ComponentBase {
	type: "canvas2d";
	width: BoundValue;
	height: BoundValue;
	backgroundColor?: BoundValue;
	pixelPerfect?: BoundValue;
}

export interface SpriteComponent extends ComponentBase {
	type: "sprite";
	src: BoundValue;
	x: BoundValue;
	y: BoundValue;
	width?: BoundValue;
	height?: BoundValue;
	rotation?: BoundValue;
	scale?: BoundValue;
	opacity?: BoundValue;
	flipX?: BoundValue;
	flipY?: BoundValue;
	zIndex?: BoundValue;
}

export interface ShapeComponent extends ComponentBase {
	type: "shape";
	shapeType: BoundValue; // "rectangle" | "circle" | "ellipse" | "polygon" | "line" | "path"
	x: BoundValue;
	y: BoundValue;
	width?: BoundValue;
	height?: BoundValue;
	radius?: BoundValue;
	points?: BoundValue;
	fill?: BoundValue;
	stroke?: BoundValue;
	strokeWidth?: BoundValue;
}

export interface Scene3DComponent extends ComponentBase {
	type: "scene3d";
	width: BoundValue;
	height: BoundValue;
	cameraType?: BoundValue; // "perspective" | "orthographic"
	cameraPosition?: BoundValue;
	backgroundColor?: BoundValue;
	/** Camera control mode: "orbit" (rotate around), "fly" (free movement), "fixed" (static view), "auto-rotate" */
	controlMode?: BoundValue;
	/** For fixed mode: "front" | "back" | "left" | "right" | "top" | "bottom" | "isometric" */
	fixedView?: BoundValue;
	/** Auto-rotation speed (degrees per second, default: 30) */
	autoRotateSpeed?: BoundValue;
	/** Enable/disable user controls (default: true for orbit/fly, false for fixed) */
	enableControls?: BoundValue;
	/** Enable zoom controls (default: true) */
	enableZoom?: BoundValue;
	/** Enable pan controls (default: true for orbit) */
	enablePan?: BoundValue;
	/** Camera field of view in degrees (default: 75) */
	fov?: BoundValue;
	/** Camera near clipping plane (default: 0.1) */
	near?: BoundValue;
	/** Camera far clipping plane (default: 1000) */
	far?: BoundValue;
	/** Target point to look at [x, y, z] (default: [0, 0, 0]) */
	target?: BoundValue;
	/** Ambient light intensity (default: 0.5) */
	ambientLight?: BoundValue;
	/** Directional light intensity (default: 1) */
	directionalLight?: BoundValue;
	/** Show grid helper (default: false) */
	showGrid?: BoundValue;
	/** Show axes helper (default: false) */
	showAxes?: BoundValue;
}

export interface Model3DComponent extends ComponentBase {
	type: "model3d";
	/** URL or path to the 3D model file (GLB, GLTF supported) */
	src: BoundValue;
	/** Position in 3D space [x, y, z] */
	position?: BoundValue;
	/** Rotation in radians [x, y, z] or Euler angles */
	rotation?: BoundValue;
	/** Uniform scale (number) or per-axis scale [x, y, z] */
	scale?: BoundValue;
	/** Whether to cast shadows (default: true) */
	castShadow?: BoundValue;
	/** Whether to receive shadows (default: true) */
	receiveShadow?: BoundValue;
	/** Animation name to play (if model has animations) */
	animation?: BoundValue;
	/** Whether the model should auto-rotate independently (default: false) */
	autoRotate?: BoundValue;
	/** Auto-rotation speed for this specific model */
	rotateSpeed?: BoundValue;

	// ============ STANDALONE VIEWER OPTIONS ============
	// These apply when Model3D is used outside of a Scene3D

	/** Viewer height (default: "256px") */
	viewerHeight?: BoundValue;
	/** Background color (default: "transparent") */
	backgroundColor?: BoundValue;
	/** Camera distance from model (default: 3) */
	cameraDistance?: BoundValue;
	/** Camera field of view in degrees (default: 50) */
	fov?: BoundValue;
	/** Camera angle preset: "front" | "side" | "top" | "isometric" (default: "front") */
	cameraAngle?: BoundValue;
	/** Explicit camera position [x, y, z] (overrides angle/distance) */
	cameraPosition?: BoundValue;
	/** Camera target [x, y, z] (default: [0,0,0]) */
	cameraTarget?: BoundValue;

	// Control options
	/** Enable orbit controls (default: true) */
	enableControls?: BoundValue;
	/** Enable zoom (default: true) */
	enableZoom?: BoundValue;
	/** Enable panning (default: false) */
	enablePan?: BoundValue;
	/** Enable auto-rotate on the camera/scene (default: false) */
	autoRotateCamera?: BoundValue;
	/** Camera auto-rotation speed (default: 2) */
	cameraRotateSpeed?: BoundValue;

	// Lighting options
	/** Ambient light intensity (default: 0.6) */
	ambientLight?: BoundValue;
	/** Main directional light intensity (default: 1.0) */
	directionalLight?: BoundValue;
	/** Fill light intensity - secondary light from opposite side (default: 0.4) */
	fillLight?: BoundValue;
	/** Rim/back light intensity - for edge highlights (default: 0.3) */
	rimLight?: BoundValue;
	/** Light color (default: "#ffffff") */
	lightColor?: BoundValue;
	/** Warm/cool lighting preset: "neutral" | "warm" | "cool" | "studio" | "dramatic" (default: "studio") */
	lightingPreset?: BoundValue;

	// Environment options
	/** Show ground plane/shadow catcher (default: false) */
	showGround?: BoundValue;
	/** Ground color (default: "#1a1a2e") */
	groundColor?: BoundValue;
	/** Enable environment reflections (default: true) */
	enableReflections?: BoundValue;
	/** Environment preset: "studio" | "sunset" | "dawn" | "night" | "warehouse" | "forest" | "apartment" | "city" | "park" | "lobby" (default: "studio") */
	environment?: BoundValue;
	/** Environment source: "local" | "preset" | "polyhaven" | "custom" (default: "local") */
	environmentSource?: BoundValue;
	/** Use HDRI as background (default: false) */
	useHdrBackground?: BoundValue;
	/** Poly Haven HDRI id (when environmentSource = "polyhaven") */
	polyhavenHdri?: BoundValue;
	/** Poly Haven resolution: "1k" | "2k" | "4k" | "8k" (default: "1k") */
	polyhavenResolution?: BoundValue;
	/** Custom HDRI URL or storage path (when environmentSource = "custom") */
	hdriUrl?: BoundValue;
	/** Ground plane size (default: 200) */
	groundSize?: BoundValue;
	/** Ground offset Y (default: -0.5) */
	groundOffsetY?: BoundValue;
	/** Keep ground centered under camera (default: true) */
	groundFollowCamera?: BoundValue;
}

export interface DialogueComponent extends ComponentBase {
	type: "dialogue";
	text: BoundValue;
	speakerName?: BoundValue;
	speakerPortraitId?: BoundValue;
	typewriter?: BoundValue;
	typewriterSpeed?: BoundValue;
}

export interface CharacterPortraitComponent extends ComponentBase {
	type: "characterPortrait";
	image: BoundValue;
	expression?: BoundValue;
	position?: BoundValue; // "left" | "right" | "center"
	size?: BoundValue; // "small" | "medium" | "large"
	dimmed?: BoundValue;
}

export interface ChoiceComponent {
	id: string;
	text: BoundValue;
	disabled?: BoundValue;
}

export interface ChoiceMenuComponent extends ComponentBase {
	type: "choiceMenu";
	choices: BoundValue;
	title?: BoundValue;
	layout?: BoundValue; // "vertical" | "horizontal" | "grid"
}

export interface InventoryItemDef {
	id: string;
	icon: BoundValue;
	name: BoundValue;
	quantity?: BoundValue;
}

export interface InventoryGridComponent extends ComponentBase {
	type: "inventoryGrid";
	items: BoundValue;
	columns?: BoundValue;
	rows?: BoundValue;
	cellSize?: BoundValue;
}

export interface HealthBarComponent extends ComponentBase {
	type: "healthBar";
	value: BoundValue;
	maxValue: BoundValue;
	label?: BoundValue;
	showValue?: BoundValue;
	fillColor?: BoundValue;
	backgroundColor?: BoundValue;
	variant?: BoundValue; // "bar" | "segmented" | "circular"
}

export interface MapMarkerDef {
	id: string;
	x: BoundValue;
	y: BoundValue;
	icon?: BoundValue;
	color?: BoundValue;
	label?: BoundValue;
}

export interface MiniMapComponent extends ComponentBase {
	type: "miniMap";
	mapImage?: BoundValue;
	width: BoundValue;
	height: BoundValue;
	markers?: BoundValue;
	playerX?: BoundValue;
	playerY?: BoundValue;
	playerRotation?: BoundValue;
}

export interface IframeComponent extends ComponentBase {
	type: "iframe";
	src?: BoundValue;
	srcdoc?: BoundValue;
	width?: BoundValue;
	height?: BoundValue;
	sandbox?: BoundValue;
	allow?: BoundValue;
	title?: BoundValue;
	loading?: BoundValue; // "lazy" | "eager"
	referrerPolicy?: BoundValue;
	border?: BoundValue;
}

// Chart types for PlotlyChart
export type ChartType =
	| "line"
	| "bar"
	| "scatter"
	| "pie"
	| "area"
	| "histogram";

export type ChartDataSource =
	| { csv: string } // Inline CSV: "label,value\nJan,20\nFeb,14\nMar,25"
	| { xPath: string; yPath: string }; // Data binding paths

export interface ChartSeries {
	name: string;
	type: ChartType;
	dataSource: ChartDataSource;
	color?: string;
	mode?: "lines" | "markers" | "lines+markers"; // For line/scatter
}

export interface ChartAxis {
	title?: string;
	type?: "linear" | "log" | "date" | "category";
	min?: number;
	max?: number;
	showGrid?: boolean;
	tickFormat?: string;
}

export interface PlotlyChartComponent extends ComponentBase {
	type: "plotlyChart";
	// New structured approach
	chartType?: BoundValue; // Default chart type for quick setup
	title?: BoundValue;
	series?: ChartSeries[]; // Structured series data
	xAxis?: ChartAxis;
	yAxis?: ChartAxis;
	// Legacy/advanced raw data (overrides series if provided)
	data?: BoundValue;
	layout?: BoundValue;
	config?: BoundValue;
	// Display
	width?: BoundValue;
	height?: BoundValue;
	responsive?: BoundValue;
	showLegend?: BoundValue;
	legendPosition?: BoundValue; // "top" | "bottom" | "left" | "right"
}

// FilePreview - Generic file preview component
export interface FilePreviewComponent extends ComponentBase {
	type: "filePreview";
	src?: BoundValue;
	url?: BoundValue;
	filename?: BoundValue;
	mimeType?: BoundValue;
	fileType?: BoundValue; // "pdf" | "image" | "video" | "audio" | "code" | "text"
	showControls?: BoundValue;
	fit?: BoundValue; // "contain" | "cover" | "fill" | "none" | "scaleDown"
	fallbackText?: BoundValue;
	height?: BoundValue;
	showDownload?: BoundValue;
	loading?: BoundValue; // "lazy" | "eager"
	/** Audio only: when set, render an animated visualizer player instead of the default controls. "conservative" | "waveform" | "orb" | "vortex" | "shader" | "aurora" | "pulse" */
	variant?: BoundValue;
	/** Audio (animated `variant`) only: auto-play when the source is set, e.g. for a conversation reply. Default false. */
	autoPlay?: BoundValue;
}

export interface DiffViewComponent extends ComponentBase {
	type: "diffView";
	original: BoundValue; // left / old content (text or document URL)
	modified: BoundValue; // right / new content (text or document URL)
	mode?: BoundValue; // "split" | "unified" | "inline"
	kind?: BoundValue; // "auto" | "text" | "code" | "markdown" | "json" | "document"
	language?: BoundValue; // syntax language for code/json
	markdownMode?: BoundValue; // "source" | "rendered"
	showLineNumbers?: BoundValue;
	wordWrap?: BoundValue;
	wordLevel?: BoundValue; // intra-line word-level highlighting
	collapseUnchanged?: BoundValue;
	contextLines?: BoundValue; // context lines kept around changes when collapsing
	showStats?: BoundValue;
	originalLabel?: BoundValue;
	modifiedLabel?: BoundValue;
	ignoreWhitespace?: BoundValue;
	ignoreCase?: BoundValue;
	trimTrailingWhitespace?: BoundValue;
	swapSides?: BoundValue;
}

// NivoChart - Nivo chart library component
// Install: bun add @nivo/core @nivo/bar @nivo/line @nivo/pie @nivo/radar @nivo/heatmap @nivo/scatterplot @nivo/funnel @nivo/treemap @nivo/sunburst @nivo/calendar @nivo/bump @nivo/circle-packing @nivo/network @nivo/sankey @nivo/stream @nivo/swarmplot @nivo/voronoi @nivo/waffle @nivo/marimekko @nivo/parallel-coordinates @nivo/radial-bar @nivo/boxplot @nivo/bullet @nivo/chord
export type NivoChartType =
	| "bar"
	| "line"
	| "pie"
	| "radar"
	| "heatmap"
	| "scatter"
	| "funnel"
	| "treemap"
	| "sunburst"
	| "calendar"
	| "bump"
	| "areaBump"
	| "circlePacking"
	| "network"
	| "sankey"
	| "stream"
	| "swarmplot"
	| "voronoi"
	| "waffle"
	| "marimekko"
	| "parallelCoordinates"
	| "radialBar"
	| "boxplot"
	| "bullet"
	| "chord";

// Chart-specific style configurations
export interface BarChartStyle {
	layout?: "vertical" | "horizontal";
	groupMode?: "grouped" | "stacked";
	padding?: number;
	innerPadding?: number;
	borderRadius?: number;
	borderWidth?: number;
	enableLabel?: boolean;
	labelSkipWidth?: number;
	labelSkipHeight?: number;
	enableGridX?: boolean;
	enableGridY?: boolean;
}

export interface LineChartStyle {
	curve?:
		| "linear"
		| "monotoneX"
		| "natural"
		| "step"
		| "stepBefore"
		| "stepAfter"
		| "basis"
		| "cardinal"
		| "catmullRom";
	lineWidth?: number;
	enableArea?: boolean;
	areaOpacity?: number;
	enablePoints?: boolean;
	pointSize?: number;
	pointBorderWidth?: number;
	enableSlices?: "x" | "y" | false;
	enableCrosshair?: boolean;
	enableGridX?: boolean;
	enableGridY?: boolean;
}

export interface PieChartStyle {
	innerRadius?: number; // 0 = pie, > 0 = donut
	padAngle?: number;
	cornerRadius?: number;
	startAngle?: number;
	endAngle?: number;
	sortByValue?: boolean;
	enableArcLabels?: boolean;
	enableArcLinkLabels?: boolean;
	arcLabelsSkipAngle?: number;
	arcLinkLabelsSkipAngle?: number;
	activeOuterRadiusOffset?: number;
}

export interface RadarChartStyle {
	gridShape?: "circular" | "linear";
	gridLevels?: number;
	gridLabelOffset?: number;
	dotSize?: number;
	dotBorderWidth?: number;
	enableDots?: boolean;
	enableDotLabel?: boolean;
	fillOpacity?: number;
	borderWidth?: number;
}

export interface HeatmapChartStyle {
	forceSquare?: boolean;
	sizeVariation?: number;
	cellOpacity?: number;
	cellBorderWidth?: number;
	enableLabels?: boolean;
	labelTextColor?: string;
}

export interface ScatterChartStyle {
	nodeSize?:
		| number
		| { key: string; values: [number, number]; sizes: [number, number] };
	enableGridX?: boolean;
	enableGridY?: boolean;
	useMesh?: boolean;
	debugMesh?: boolean;
}

export interface FunnelChartStyle {
	direction?: "horizontal" | "vertical";
	interpolation?: "smooth" | "linear";
	spacing?: number;
	shapeBlending?: number;
	enableLabel?: boolean;
	labelColor?: string;
	borderWidth?: number;
	borderOpacity?: number;
	beforeSeparatorLength?: number;
	afterSeparatorLength?: number;
	currentPartSizeExtension?: number;
}

export interface TreemapChartStyle {
	tile?: "binary" | "dice" | "slice" | "sliceDice" | "squarify" | "resquarify";
	leavesOnly?: boolean;
	innerPadding?: number;
	outerPadding?: number;
	enableLabel?: boolean;
	enableParentLabel?: boolean;
	labelSkipSize?: number;
}

export interface SankeyChartStyle {
	layout?: "horizontal" | "vertical";
	align?: "center" | "justify" | "start" | "end";
	nodeOpacity?: number;
	nodeThickness?: number;
	nodeSpacing?: number;
	nodeInnerPadding?: number;
	linkOpacity?: number;
	linkBlendMode?: string;
	enableLinkGradient?: boolean;
	enableLabels?: boolean;
	labelPosition?: "inside" | "outside";
}

export interface CalendarChartStyle {
	direction?: "horizontal" | "vertical";
	emptyColor?: string;
	yearSpacing?: number;
	yearLegendOffset?: number;
	monthSpacing?: number;
	monthBorderWidth?: number;
	daySpacing?: number;
	dayBorderWidth?: number;
}

export interface ChordChartStyle {
	padAngle?: number;
	innerRadiusRatio?: number;
	innerRadiusOffset?: number;
	arcOpacity?: number;
	arcBorderWidth?: number;
	ribbonOpacity?: number;
	ribbonBorderWidth?: number;
	enableLabel?: boolean;
	labelOffset?: number;
	labelRotation?: number;
}

export interface NivoChartComponent extends ComponentBase {
	type: "nivoChart";
	chartType: BoundValue; // NivoChartType
	title?: BoundValue;
	data?: BoundValue; // Chart-specific data format (JSON or array)
	height?: BoundValue;
	colors?: BoundValue; // color scheme name (e.g. "nivo", "paired") or array of colors
	animate?: BoundValue;
	showLegend?: BoundValue;
	legendPosition?: BoundValue; // "top" | "bottom" | "left" | "right"
	indexBy?: BoundValue; // Key for indexing data (bar, radar)
	keys?: BoundValue; // Data keys to display (bar, radar, stream)
	margin?: BoundValue; // { top, right, bottom, left }
	axisBottom?: BoundValue; // Bottom axis config
	axisLeft?: BoundValue; // Left axis config
	axisTop?: BoundValue; // Top axis config
	axisRight?: BoundValue; // Right axis config
	config?: BoundValue; // Full Nivo config override (advanced)
	// Chart-type specific styling
	barStyle?: BoundValue; // BarChartStyle
	lineStyle?: BoundValue; // LineChartStyle
	pieStyle?: BoundValue; // PieChartStyle
	radarStyle?: BoundValue; // RadarChartStyle
	heatmapStyle?: BoundValue; // HeatmapChartStyle
	scatterStyle?: BoundValue; // ScatterChartStyle
	funnelStyle?: BoundValue; // FunnelChartStyle
	treemapStyle?: BoundValue; // TreemapChartStyle
	sankeyStyle?: BoundValue; // SankeyChartStyle
	calendarStyle?: BoundValue; // CalendarChartStyle
	chordStyle?: BoundValue; // ChordChartStyle
}

// BoundingBoxOverlay - Display bounding boxes on an image
export interface BoundingBox {
	id?: string;
	x: number;
	y: number;
	width: number;
	height: number;
	label?: string;
	confidence?: number;
	color?: string;
}

export interface BoundingBoxOverlayComponent extends ComponentBase {
	type: "boundingBoxOverlay";
	src: BoundValue;
	alt?: BoundValue;
	boxes: BoundValue; // BoundingBox[]
	showLabels?: BoundValue;
	showConfidence?: BoundValue;
	strokeWidth?: BoundValue;
	fontSize?: BoundValue;
	fit?: BoundValue; // "contain" | "cover" | "fill"
	normalized?: BoundValue; // If true, coordinates are 0-1 normalized
	interactive?: BoundValue; // Enable click events on boxes
}

// ImageLabeler - Draw bounding boxes on an image for labeling tasks
export interface LabelBox {
	id: string;
	x: number;
	y: number;
	width: number;
	height: number;
	label: string;
}

export interface ImageLabelerComponent extends ComponentBase {
	type: "imageLabeler";
	src: BoundValue;
	alt?: BoundValue;
	boxes?: BoundValue; // LabelBox[] - initial boxes
	labels: BoundValue; // string[] - available labels to choose from
	disabled?: BoundValue;
	showLabels?: BoundValue;
	minBoxSize?: BoundValue; // Minimum box size in pixels
}

// ImageHotspot - Point and click adventure / interactive image
export interface Hotspot {
	id: string;
	x: number;
	y: number;
	size?: number;
	color?: string;
	icon?: string;
	label?: string;
	description?: string;
	action?: string;
	disabled?: boolean;
}

export interface ImageHotspotComponent extends ComponentBase {
	type: "imageHotspot";
	src: BoundValue;
	alt?: BoundValue;
	hotspots: BoundValue; // Hotspot[]
	showMarkers?: BoundValue;
	markerStyle?: BoundValue; // "pulse" | "dot" | "ring" | "square" | "diamond" | "none"
	fit?: BoundValue; // "contain" | "cover" | "fill"
	normalized?: BoundValue; // If true, coordinates are 0-1 normalized
	showTooltips?: BoundValue;
}

// ── Geo types (mirrors Rust structs from packages/catalog/geo) ─────
// All fields use snake_case matching default serde serialization.

export interface GeoCoordinate {
	latitude: number;
	longitude: number;
}

export interface GeoBoundingBox {
	min_lat: number;
	min_lon: number;
	max_lat: number;
	max_lon: number;
}

export interface GeoRouteGeometry {
	points: GeoCoordinate[];
}

export interface GeoRouteStep {
	instruction: string;
	distance: number;
	duration: number;
	name: string;
	maneuver_type: string;
	coordinate: GeoCoordinate;
}

export interface GeoRouteLeg {
	distance: number;
	duration: number;
	summary: string;
	steps: GeoRouteStep[];
}

export interface GeoRouteResult {
	distance: number;
	duration: number;
	geometry: GeoRouteGeometry;
	legs: GeoRouteLeg[];
	weight_name: string;
}

export interface GeoTripWaypoint {
	name: string;
	distance: number;
	coordinate: GeoCoordinate;
	hint?: string;
	waypoint_index?: number;
}

export interface GeoSearchResult {
	display_name: string;
	coordinate: GeoCoordinate;
	place_type: string;
	importance: number;
	bounding_box?: GeoBoundingBox;
	osm_id?: number;
	osm_type?: string;
}

// ── Geo Map component types ────────────────────────────────────────

export interface GeoMapMarkerDef {
	id: string;
	coordinate: GeoCoordinate;
	color?: string;
	label?: string;
	icon?: string;
	popup?: string;
	draggable?: boolean;
}

export interface GeoMapRouteDef {
	id: string;
	coordinates: GeoCoordinate[];
	color?: string;
	width?: number;
	opacity?: number;
	dashArray?: [number, number];
	label?: string;
}

export interface GeoMapViewport {
	center: GeoCoordinate;
	zoom: number;
	bearing?: number;
	pitch?: number;
}

export interface GeoMapComponent extends ComponentBase {
	type: "geoMap";
	viewport?: BoundValue;
	markers?: BoundValue;
	routes?: BoundValue;
	showControls?: BoundValue;
	showZoom?: BoundValue;
	showCompass?: BoundValue;
	showLocate?: BoundValue;
	showFullscreen?: BoundValue;
	interactive?: BoundValue;
	controlPosition?: BoundValue;
	clusterMarkers?: BoundValue;
	clusterRadius?: BoundValue;
	clusterMaxZoom?: BoundValue;
}

// ── Graph component types ──────────────────────────────────────────

/** Style applied to every node or edge carrying a label. */
export interface GraphLabelStyleDef {
	color?: string;
	icon?: string;
	size?: number;
}

/** Mirrors SubgraphNode, so graph query output binds without mapping. */
export interface GraphNodeDef {
	id: string;
	label: string;
	caption?: string;
	props?: Record<string, unknown>;
}

/** Mirrors SubgraphEdge, so graph query output binds without mapping. */
export interface GraphEdgeDef {
	id: string;
	source: string;
	target: string;
	label: string;
	props?: Record<string, unknown>;
}

export interface GraphComponent extends ComponentBase {
	type: "graph";
	nodes: BoundValue;
	edges?: BoundValue;
	/** `{ "<label>": { color, icon, size } }` or `[{ label, color, icon, size }]` */
	labelStyles?: BoundValue;
	showToolbar?: BoundValue;
	showSearch?: BoundValue;
	showLegend?: BoundValue;
	showInspector?: BoundValue;
	height?: BoundValue;
}

export interface OntologyGraphComponent extends ComponentBase {
	type: "ontologyGraph";
	ontologyId: BoundValue;
	appId?: BoundValue;
	limit?: BoundValue;
	allowExpand?: BoundValue;
	allowSearch?: BoundValue;
	allowPaths?: BoundValue;
	allowActions?: BoundValue;
	allowCypher?: BoundValue;
	allowStyleEdit?: BoundValue;
	allowLimitChange?: BoundValue;
	showToolbar?: BoundValue;
	showLegend?: BoundValue;
	height?: BoundValue;
}

// ── Planning: Calendar & Gantt (mirrors Rust structs) ──────────────

// A single calendar event (mirrors CalendarEvent in update_schemas.rs)
export interface CalendarEvent {
	id: string;
	title: string;
	start: string; // ISO 8601
	end?: string; // ISO 8601
	allDay?: boolean;
	color?: string;
	description?: string;
	location?: string;
	calendarId?: string;
	editable?: boolean;
	/** Opened from the detail dialog: relative = in-app route, absolute = external. */
	link?: string;
	/** Key-value metadata shown in the detail dialog (e.g. ticket number). */
	metadata?: Record<string, unknown>;
}

export type CalendarView = "month" | "week" | "day" | "agenda";

/** Visual density preset for planning components. */
export type PlanningDensity = "compact" | "default" | "comfortable";

export interface CalendarComponent extends ComponentBase {
	type: "calendar";
	events: BoundValue; // CalendarEvent[]
	view?: BoundValue; // CalendarView
	date?: BoundValue; // focused date (ISO 8601)
	title?: BoundValue; // optional header title
	density?: BoundValue; // PlanningDensity
	editable?: BoundValue;
	selectable?: BoundValue;
	firstDayOfWeek?: BoundValue; // 0 = Sunday
	minTime?: BoundValue; // "06:00"
	maxTime?: BoundValue; // "22:00"
	slotDuration?: BoundValue; // minutes
	showWeekends?: BoundValue;
	showNowIndicator?: BoundValue;
	showAllDay?: BoundValue;
	showViewSwitcher?: BoundValue;
	locale?: BoundValue;
	height?: BoundValue;
	responsive?: BoundValue;
	compactBreakpoint?: BoundValue; // px
}

// A single gantt task (mirrors GanttTask in update_schemas.rs)
export interface GanttTask {
	id: string;
	name: string;
	start: string; // ISO 8601
	end: string; // ISO 8601
	progress?: number; // 0-100
	dependencies?: string[]; // predecessor task ids
	parent?: string;
	color?: string;
	assignee?: string;
	milestone?: boolean;
	collapsed?: boolean;
	/** Opened from the detail dialog: relative = in-app route, absolute = external. */
	link?: string;
	/** Key-value metadata shown in the detail dialog (e.g. ticket number). */
	metadata?: Record<string, unknown>;
}

export type GanttView = "day" | "week" | "month" | "quarter" | "compact";

export interface GanttComponent extends ComponentBase {
	type: "gantt";
	tasks: BoundValue; // GanttTask[]
	view?: BoundValue; // GanttView
	title?: BoundValue; // header title (default "Timeline")
	density?: BoundValue; // PlanningDensity
	editable?: BoundValue;
	draggable?: BoundValue;
	resizable?: BoundValue;
	showDependencies?: BoundValue;
	showProgress?: BoundValue;
	showToday?: BoundValue;
	showViewSwitcher?: BoundValue;
	showTaskList?: BoundValue; // show/hide the left task panel
	taskListWidth?: BoundValue; // px width of the left task panel
	shadeWeekends?: BoundValue; // shade Sat/Sun columns in day/week views
	rowHeight?: BoundValue;
	columns?: BoundValue; // string[] extra left-panel columns
	height?: BoundValue;
	responsive?: BoundValue;
	compactBreakpoint?: BoundValue; // px
}

// Widget Instance Component - references a widget definition stored in page.widgetRefs
// The widget definition is looked up by instanceId from the page's widgetRefs
export interface WidgetInstanceComponent extends ComponentBase {
	type: "widgetInstance";
	/** The instance ID - used to look up the widget definition from page.widgetRefs */
	instanceId: string;
	/** Original widget ID for reference/updates */
	widgetId: string;
	/** Original app ID for fetching updates */
	appId?: string;
	/** Values for exposed props */
	exposedPropValues?: Record<string, unknown>;
	/** Bindings from widget actions to page workflows */
	actionBindings?: Record<string, unknown>;
	/** Style overrides for the widget instance */
	styleOverride?: Style;
	style?: Style;
}

// Micro Widget Instance Component - a package-shipped widget rendered inside a
// sandboxed iframe (flw/1 host bridge). Self-contained: contract + props are
// embedded, so instances replay from component data alone (chat parity).
export interface MicroWidgetInstanceComponent extends ComponentBase {
	type: "microWidgetInstance";
	/** Unique instance id — also the elements-payload key prefix for `{instanceId}/values`. */
	instanceId: string;
	/** Package that ships the widget bundle. */
	packageId: string;
	/** Widget id inside the bundle. */
	widgetId: string;
	/** Package version, used for web (CDN/registry) serving. */
	packageVersion: string;
	/** sha256 hex of the widget bundle, used for desktop (flow-widget://) serving. */
	bundleHash?: string | null;
	/** Embedded typed contract (inputs/events/queries/sizing). */
	contract?: WidgetContract | null;
	/** Current input props, patched via typed element updates. */
	props?: Record<string, unknown>;
	/** Preview mode: events are ignored and the widget's emit is a no-op. */
	preview?: boolean;
	/** Bindings from contract event names to page workflows (same as widgetInstance). */
	actionBindings?: Record<string, unknown>;
	style?: Style;
}

// All component types union
export type A2UIComponent =
	| RowComponent
	| ColumnComponent
	| StackComponent
	| GridComponent
	| ScrollAreaComponent
	| AspectRatioComponent
	| OverlayComponent
	| AbsoluteComponent
	| BoxComponent
	| CenterComponent
	| SpacerComponent
	| TextComponent
	| ImageComponent
	| IconComponent
	| VideoComponent
	| LottieComponent
	| MarkdownComponent
	| DividerComponent
	| BadgeComponent
	| AvatarComponent
	| UserProfileComponent
	| ProgressComponent
	| SpinnerComponent
	| SkeletonComponent
	| TableComponent
	| TableRowComponent
	| TableCellComponent
	| ButtonComponent
	| FeedbackComponent
	| AppLinkComponent
	| TextFieldComponent
	| RichTextComponent
	| SelectComponent
	| SliderComponent
	| CheckboxComponent
	| SwitchComponent
	| RadioGroupComponent
	| DateTimeInputComponent
	| FileInputComponent
	| ImageInputComponent
	| VoiceInputComponent
	| LinkComponent
	| CardComponent
	| ModalComponent
	| TabsComponent
	| AccordionComponent
	| DrawerComponent
	| TooltipComponent
	| PopoverComponent
	| Canvas2DComponent
	| SpriteComponent
	| ShapeComponent
	| Scene3DComponent
	| Model3DComponent
	| DialogueComponent
	| CharacterPortraitComponent
	| ChoiceMenuComponent
	| InventoryGridComponent
	| HealthBarComponent
	| MiniMapComponent
	| IframeComponent
	| PlotlyChartComponent
	| FilePreviewComponent
	| DiffViewComponent
	| NivoChartComponent
	| BoundingBoxOverlayComponent
	| ImageLabelerComponent
	| ImageHotspotComponent
	| GeoMapComponent
	| GraphComponent
	| OntologyGraphComponent
	| CalendarComponent
	| GanttComponent
	| WidgetInstanceComponent
	| MicroWidgetInstanceComponent;

// Surface and data model
export interface DataEntry {
	path: string;
	value: unknown;
}

export interface SurfaceComponent {
	id: string;
	style?: Style;
	component: A2UIComponent;
	eventRelevant?: boolean;
}

export interface Surface {
	id: string;
	rootComponentId: string;
	components: Record<string, SurfaceComponent>;
	dataModel?: DataEntry[];
	canvasSettings?: CanvasSettings;
	catalogId?: string;
}

// Messages
export type A2UIServerMessage =
	| {
			type: "beginRendering";
			surfaceId: string;
			rootComponentId: string;
			components: SurfaceComponent[];
			dataModel: DataEntry[];
			catalogId?: string;
	  }
	| {
			type: "surfaceUpdate";
			surfaceId: string;
			components: SurfaceComponent[];
			parentId?: string;
	  }
	| {
			type: "setCanvasSettings";
			surfaceId: string;
			canvasSettings: CanvasSettings;
	  }
	| {
			type: "dataModelUpdate";
			surfaceId: string;
			path?: string;
			contents: DataEntry[];
	  }
	| {
			type: "deleteSurface";
			surfaceId: string;
	  }
	| {
			type: "requestElements";
			elementIds: string[];
	  }
	| {
			type: "widgetQuery";
			requestId: string;
			instanceId: string;
			query: string;
			args?: unknown;
			timeoutMs: number;
	  }
	| {
			type: "showScreen";
	  }
	| {
			type: "upsertElement";
			element_id: string;
			value: unknown;
	  }
	| {
			type: "navigateTo";
			route: string;
			replace: boolean;
			queryParams?: Record<string, string>;
	  }
	| {
			type: "createElement";
			surfaceId: string;
			parentId: string;
			component: SurfaceComponent;
			index?: number;
	  }
	| {
			type: "removeElement";
			surfaceId: string;
			elementId: string;
	  }
	| {
			type: "setGlobalState";
			key: string;
			value: unknown;
	  }
	| {
			type: "setPageState";
			pageId: string;
			key: string;
			value: unknown;
	  }
	| {
			type: "clearPageState";
			pageId: string;
	  }
	| {
			type: "clearFileInput";
			surfaceId: string;
			componentId: string;
	  }
	| {
			type: "setQueryParam";
			key: string;
			value?: string;
			replace: boolean;
	  }
	| {
			type: "openDialog";
			route: string;
			title?: string;
			queryParams?: Record<string, string>;
			dialogId?: string;
	  }
	| {
			type: "closeDialog";
			dialogId?: string;
	  };

export interface A2UIClientMessage {
	type: "userAction";
	name: string;
	surfaceId: string;
	sourceComponentId: string;
	timestamp: number;
	context: Record<string, unknown>;
}

// Widget and Page definitions
export interface Widget {
	id: string;
	name: string;
	description?: string;
	rootComponentId: string;
	components: SurfaceComponent[];
	dataModel: DataEntry[];
	customizationOptions: CustomizationOption[];
	catalogId?: string;
	thumbnail?: string;
	tags: string[];
	actions: WidgetAction[];
}

export interface WidgetAction {
	id: string;
	label: string;
	description?: string;
	icon?: string;
	contextSchema: WidgetActionContextField[];
}

export interface WidgetActionContextField {
	name: string;
	label: string;
	fieldType: string;
	description?: string;
	defaultPath?: string;
}

export type ActionBinding =
	| { workflow: WorkflowBinding }
	| { command: CommandBinding };

export interface WorkflowBinding {
	flowId: string;
	inputMappings: Record<string, BoundValue>;
}

export interface CommandBinding {
	commandName: string;
	args: Record<string, BoundValue>;
}

export interface WidgetRef {
	appId: string;
	widgetId: string;
	version?: string;
}

export interface CustomizationOption {
	id: string;
	label: string;
	description?: string;
	type: CustomizationType;
	defaultValue?: unknown;
	validations: ValidationRule[];
	group?: string;
}

export type CustomizationType =
	| "string"
	| "number"
	| "boolean"
	| "color"
	| "imageUrl"
	| "icon"
	| "enum"
	| "json";

export interface ValidationRule {
	ruleType: string;
	value?: unknown;
	message?: string;
}

export interface CanvasSettings {
	backgroundColor?: string;
	backgroundImage?: string;
	padding?: string;
	customCss?: string;
}

export interface Page {
	id: string;
	name: string;
	route: string;
	title?: string;
	canvasSettings?: CanvasSettings;
	content: PageContent[];
	layoutType: PageLayoutType;
	attachedElementId?: string;
	meta?: PageMeta;
	components: SurfaceComponent[];
}

export type PageContent =
	| { widget: WidgetInstance }
	| { component: A2UIComponent }
	| { componentId: string };

export interface WidgetInstance {
	widgetId: string;
	instanceId: string;
	position?: Position;
	customizationValues: Record<string, unknown>;
	actionBindings: Record<string, ActionBinding>;
	widgetRef?: WidgetRef;
}

export type PageLayoutType = "single" | "sidebar" | "grid" | "custom";

export interface PageMeta {
	description?: string;
	keywords: string[];
	ogImage?: string;
}
