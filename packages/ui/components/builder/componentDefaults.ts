import type {
	A2UIComponent,
	BoundValue,
	SelectOption,
	Style,
} from "../a2ui/types";
import { isValidComponentType, isValidProperty } from "./componentSchema";

// Helper to create BoundValue literals
export const str = (value: string): BoundValue => ({ literalString: value });
export const num = (value: number): BoundValue => ({ literalNumber: value });
export const bool = (value: boolean): BoundValue => ({ literalBool: value });
export const options = (value: SelectOption[]): BoundValue => ({
	literalOptions: value,
});

// Default styles for components that need specific dimensions
export const COMPONENT_STYLE_DEFAULTS: Record<string, Partial<Style>> = {
	stack: { className: "min-h-[200px] min-w-[200px]" },
	scrollArea: { className: "h-[300px]" },
	modal: { className: "" },
	drawer: { className: "" },
};

// Default props for each component type
export const COMPONENT_DEFAULT_PROPS: Record<
	string,
	Record<string, unknown>
> = {
	// Layout
	row: {
		gap: str("8px"),
		align: str("start"),
		justify: str("start"),
		wrap: bool(false),
		reverse: bool(false),
		children: { explicitList: [] },
	},
	column: {
		gap: str("8px"),
		align: str("stretch"),
		justify: str("start"),
		wrap: bool(false),
		reverse: bool(false),
		children: { explicitList: [] },
	},
	stack: {
		align: str("stretch"),
		width: str("100%"),
		height: str("300px"),
		children: { explicitList: [] },
	},
	grid: {
		columns: num(2),
		rows: num(2),
		gap: str("8px"),
		autoFlow: str("row"),
		children: { explicitList: [] },
	},
	scrollArea: { direction: str("vertical"), children: { explicitList: [] } },
	overlay: { children: { explicitList: [] } },
	absolute: {
		width: str("100%"),
		height: str("100%"),
		children: { explicitList: [] },
	},
	box: {
		as: str("div"),
		children: { explicitList: [] },
	},
	center: {
		inline: bool(false),
		children: { explicitList: [] },
	},
	spacer: {
		flex: num(1),
	},

	// Display
	text: {
		content: str("Text"),
		variant: str("body"),
		size: str("md"),
		weight: str("normal"),
		align: str("left"),
		truncate: bool(false),
	},
	image: {
		src: str("https://placehold.co/400x300"),
		alt: str("Image"),
		fallback: str(""),
		fit: str("cover"),
		loading: str("lazy"),
	},
	video: {
		src: str(""),
		autoplay: bool(false),
		loop: bool(false),
		muted: bool(false),
		controls: bool(true),
		poster: str(""),
	},
	icon: { name: str("star"), size: num(24), strokeWidth: num(2) },
	markdown: { content: str("**Hello** _world_"), allowHtml: bool(false) },
	divider: { orientation: str("horizontal"), thickness: str("1px") },
	badge: { content: str("Badge"), variant: str("default") },
	avatar: { src: str(""), fallback: str("AB"), size: str("md") },
	userProfile: {
		value: str(""),
		variant: str("row"),
		avatarSize: str("md"),
		showHover: bool(true),
		showEmail: bool(true),
		showDescription: bool(true),
		showUserId: bool(true),
		showProfileLink: bool(true),
		fallbackLabel: str("Unknown user"),
		muted: bool(false),
	},
	progress: {
		value: num(50),
		max: num(100),
		showLabel: bool(false),
		variant: str("default"),
	},
	spinner: { size: str("md") },
	skeleton: { width: str("100%"), height: str("20px"), rounded: bool(true) },
	iframe: {
		src: str("https://example.com"),
		width: str("100%"),
		height: str("400px"),
		title: str("Embedded content"),
		loading: str("lazy"),
		border: bool(false),
	},
	plotlyChart: {
		chartType: str("line"),
		title: str("Chart Title"),
		series: [
			{
				name: "Series 1",
				type: "line" as const,
				dataSource: { csv: "Jan,20\nFeb,14\nMar,25\nApr,16\nMay,18\nJun,22" },
				color: "#6366f1",
				mode: "lines+markers" as const,
			},
		],
		xAxis: { title: "X Axis", showGrid: true },
		yAxis: { title: "Y Axis", showGrid: true },
		width: str("100%"),
		height: str("400px"),
		responsive: bool(true),
		showLegend: bool(true),
		legendPosition: str("bottom"),
	},
	table: {
		columns: {
			literalJson: JSON.stringify([
				{
					id: "col-1",
					header: { literalString: "Name" },
					accessor: { literalString: "name" },
					sortable: { literalBool: true },
				},
				{
					id: "col-2",
					header: { literalString: "Age" },
					accessor: { literalString: "age" },
					sortable: { literalBool: true },
				},
				{
					id: "col-3",
					header: { literalString: "Email" },
					accessor: { literalString: "email" },
					sortable: { literalBool: true },
				},
			]),
		},
		data: {
			literalJson: JSON.stringify([
				{ name: "John Doe", age: "28", email: "john@example.com" },
				{ name: "Jane Smith", age: "34", email: "jane@example.com" },
				{ name: "Bob Johnson", age: "45", email: "bob@example.com" },
			]),
		},
		striped: bool(false),
		bordered: bool(false),
		hoverable: bool(true),
		compact: bool(false),
		stickyHeader: bool(false),
		sortable: bool(true),
		searchable: bool(false),
		paginated: bool(false),
		pageSize: num(10),
	},

	// Interactive
	button: {
		label: str("Button"),
		variant: str("default"),
		size: str("md"),
		disabled: bool(false),
		loading: bool(false),
		icon: str(""),
		iconPosition: str("left"),
	},
	feedback: {
		mode: str("compact"),
		size: str("md"),
		title: str("Was this helpful?"),
		description: str(""),
		positiveLabel: str("Yes"),
		negativeLabel: str("No"),
		positiveRating: num(5),
		negativeRating: num(1),
		showComment: bool(false),
		commentMode: str("modal"),
		commentLabel: str("Comment"),
		commentPlaceholder: str("Optional comment"),
		commentTitle: str("Add feedback"),
		commentDescription: str("Share a little more context before submitting."),
		commentSubmitLabel: str("Submit feedback"),
		commentCancelLabel: str("Cancel"),
		feedbackId: str(""),
		includeState: bool(true),
		pageContextMode: str("path"),
		pageContextQueryParamAllowlist: str(""),
		pageContextQueryParamDenylist: str(""),
		includePageHash: bool(false),
		successMessage: str("Thanks for the feedback."),
		disabled: bool(false),
	},
	appLink: {
		target: str("config"),
		label: str(""),
		variant: str("outline"),
		size: str("sm"),
		icon: str("settings"),
		iconPosition: str("left"),
		appId: str(""),
		eventId: str(""),
		disabled: bool(false),
	},
	textField: {
		value: str(""),
		placeholder: str("Enter text..."),
		label: str("Label"),
		inputType: str("text"),
		multiline: bool(false),
		disabled: bool(false),
		required: bool(false),
	},
	select: {
		value: str(""),
		options: options([
			{ value: "option1", label: "Option 1" },
			{ value: "option2", label: "Option 2" },
			{ value: "option3", label: "Option 3" },
		]),
		placeholder: str("Select..."),
		label: str("Label"),
		disabled: bool(false),
		multiple: bool(false),
		searchable: bool(false),
	},
	slider: {
		value: num(50),
		min: num(0),
		max: num(100),
		step: num(1),
		disabled: bool(false),
		showValue: bool(true),
		label: str("Slider"),
	},
	checkbox: {
		checked: bool(false),
		label: str("Checkbox"),
		disabled: bool(false),
		indeterminate: bool(false),
	},
	switch: { checked: bool(false), label: str("Switch"), disabled: bool(false) },
	radioGroup: {
		value: str(""),
		options: options([
			{ value: "option1", label: "Option 1" },
			{ value: "option2", label: "Option 2" },
			{ value: "option3", label: "Option 3" },
		]),
		disabled: bool(false),
		orientation: str("vertical"),
		label: str("Radio Group"),
	},
	dateTimeInput: {
		value: str(""),
		mode: str("date"),
		disabled: bool(false),
		label: str("Date"),
	},
	fileInput: {
		value: str(""),
		label: str("Upload File"),
		accept: str("*/*"),
		multiple: bool(false),
		disabled: bool(false),
	},
	imageInput: {
		value: str(""),
		label: str("Upload Image"),
		accept: str("image/*"),
		multiple: bool(false),
		disabled: bool(false),
		showPreview: bool(true),
	},
	link: {
		label: str("Click here"),
		variant: str("default"),
		underline: str("hover"),
		disabled: bool(false),
		external: bool(false),
	},

	// Container
	card: {
		title: str("Card Title"),
		description: str("Card description"),
		hoverable: bool(false),
		clickable: bool(false),
		children: { explicitList: [] },
	},
	modal: {
		open: bool(false),
		title: str("Modal Title"),
		description: str("Modal description"),
		closeOnOverlay: bool(true),
		closeOnEscape: bool(true),
		showCloseButton: bool(true),
		size: str("md"),
		children: { explicitList: [] },
	},
	tabs: {
		value: str("tab1"),
		orientation: str("horizontal"),
		variant: str("default"),
	},
	accordion: { multiple: bool(false), collapsible: bool(true) },
	drawer: {
		open: bool(false),
		side: str("right"),
		title: str("Drawer"),
		size: str("300px"),
		overlay: bool(true),
		closable: bool(true),
		children: { explicitList: [] },
	},
	tooltip: {
		content: str("Tooltip text"),
		side: str("top"),
		delayMs: num(200),
		children: { explicitList: [] },
	},
	popover: {
		open: bool(false),
		side: str("bottom"),
		trigger: str("click"),
		closeOnClickOutside: bool(true),
		children: { explicitList: [] },
	},

	// Game
	canvas2d: {
		width: num(800),
		height: num(600),
		backgroundColor: str("#000000"),
		pixelPerfect: bool(false),
		children: { explicitList: [] },
	},
	sprite: {
		src: str(""),
		x: num(0),
		y: num(0),
		rotation: num(0),
		scale: num(1),
		opacity: num(1),
		flipX: bool(false),
		flipY: bool(false),
		zIndex: num(0),
	},
	shape: {
		shapeType: str("rectangle"),
		x: num(0),
		y: num(0),
		width: num(100),
		height: num(100),
		fill: str("#3b82f6"),
		stroke: str("#1d4ed8"),
		strokeWidth: num(2),
	},
	scene3d: {
		width: num(800),
		height: num(600),
		cameraType: str("perspective"),
		backgroundColor: str("#111827"),
		children: { explicitList: [] },
	},
	model3d: {
		src: str(""),
		position: { literalJson: JSON.stringify([0, 0, 0]) },
		rotation: { literalJson: JSON.stringify([0, 0, 0]) },
		scale: num(1),
		castShadow: bool(true),
		receiveShadow: bool(true),
		autoRotate: bool(false),
		rotateSpeed: num(1),
		viewerHeight: str("100%"),
		backgroundColor: str("transparent"),
		cameraDistance: num(3),
		fov: num(50),
		cameraAngle: str("front"),
		enableControls: bool(true),
		enableZoom: bool(true),
		enablePan: bool(false),
		autoRotateCamera: bool(false),
		cameraRotateSpeed: num(2),
		lightingPreset: str("studio"),
		ambientLight: num(0.6),
		directionalLight: num(1.2),
		fillLight: num(0.5),
		rimLight: num(0.4),
		lightColor: str("#ffffff"),
		showGround: bool(false),
		groundColor: str("#1a1a2e"),
		groundSize: num(200),
		groundOffsetY: num(-0.5),
		groundFollowCamera: bool(true),
		enableReflections: bool(true),
		environment: str("studio"),
		environmentSource: str("local"),
		useHdrBackground: bool(false),
		polyhavenHdri: str("studio_small_03"),
		polyhavenResolution: str("1k"),
		hdriUrl: str(""),
	},
	dialogue: {
		text: str("Hello, traveler!"),
		speakerName: str("NPC"),
		typewriter: bool(true),
		typewriterSpeed: num(50),
	},
	characterPortrait: {
		image: str(""),
		position: str("left"),
		size: str("medium"),
		dimmed: bool(false),
	},
	choiceMenu: {
		choices: { path: "choices" },
		title: str("Choose an option"),
		layout: str("vertical"),
	},
	inventoryGrid: {
		items: { path: "inventory" },
		columns: num(5),
		rows: num(4),
		cellSize: str("64px"),
	},
	healthBar: {
		value: num(80),
		maxValue: num(100),
		label: str("HP"),
		showValue: bool(true),
		fillColor: str("#ef4444"),
		backgroundColor: str("#1f2937"),
		variant: str("bar"),
	},
	miniMap: {
		mapImage: str(""),
		width: num(200),
		height: num(200),
		playerX: num(100),
		playerY: num(100),
		playerRotation: num(0),
	},
	geoMap: {
		viewport: {
			literalJson: JSON.stringify({
				center: { latitude: 48.137, longitude: 11.576 },
				zoom: 10,
			}),
		},
		showControls: bool(true),
		showZoom: bool(true),
		showCompass: bool(false),
		showLocate: bool(false),
		showFullscreen: bool(false),
		interactive: bool(true),
		controlPosition: str("bottom-right"),
	},
	lottie: {
		src: str(""),
		autoplay: bool(true),
		loop: bool(true),
		speed: num(1),
	},
	aspectRatio: { ratio: num(16 / 9), children: { explicitList: [] } },
	filePreview: {
		src: str(""),
		filename: str("document"),
		mimeType: str(""),
		fileType: str("unknown"),
		showControls: bool(true),
		fit: str("contain"),
		fallbackText: str("Cannot preview this file"),
		showDownload: bool(false),
		loading: str("lazy"),
	},
	nivoChart: {
		chartType: str("bar"),
		data: {
			literalJson: JSON.stringify([
				{ country: "USA", burgers: 120, fries: 80, sandwiches: 60 },
				{ country: "UK", burgers: 90, fries: 110, sandwiches: 70 },
				{ country: "France", burgers: 60, fries: 70, sandwiches: 100 },
				{ country: "Germany", burgers: 85, fries: 95, sandwiches: 55 },
			]),
		},
		width: str("100%"),
		height: str("400px"),
		indexBy: str("country"),
		keys: { literalJson: JSON.stringify(["burgers", "fries", "sandwiches"]) },
		colors: str("nivo"),
		animate: bool(true),
		showLegend: bool(true),
		legendPosition: str("bottom"),
		margin: {
			literalJson: JSON.stringify({
				top: 50,
				right: 130,
				bottom: 50,
				left: 60,
			}),
		},
	},
	boundingBoxOverlay: {
		src: str("https://placehold.co/800x600"),
		boxes: { literalJson: "[]" },
		editable: bool(false),
		showLabels: bool(true),
	},
	imageLabeler: {
		src: str("https://placehold.co/800x600"),
		regions: { literalJson: "[]" },
		tools: { literalJson: JSON.stringify(["rectangle", "polygon"]) },
		showLabels: bool(true),
	},
	imageHotspot: {
		src: str("https://placehold.co/800x600"),
		hotspots: { literalJson: "[]" },
	},
};

export function createDefaultComponent(type: string): A2UIComponent {
	const defaults = COMPONENT_DEFAULT_PROPS[type] ?? {};
	return { type, ...defaults } as unknown as A2UIComponent;
}

export function getDefaultProps(type: string): Record<string, unknown> {
	return COMPONENT_DEFAULT_PROPS[type] ?? {};
}

export function getDefaultStyle(type: string): Partial<Style> | undefined {
	return COMPONENT_STYLE_DEFAULTS[type];
}

/**
 * Validates and normalizes a component by:
 * 1. Removing unknown properties not in the schema
 * 2. Ensuring all required props are present with defaults
 *
 * @param component The component to normalize
 * @param strict If true, unknown props are removed. If false, they're kept.
 */
export function normalizeComponent(
	component: A2UIComponent,
	strict = true,
): A2UIComponent {
	const { type, ...existingProps } = component;

	// Unknown component type - return as-is (or filter unknown types entirely)
	if (!isValidComponentType(type)) {
		console.warn(
			`[A2UI] Unknown component type "${type}" - skipping normalization`,
		);
		return component;
	}

	const defaults = COMPONENT_DEFAULT_PROPS[type] ?? {};
	const normalized: Record<string, unknown> = { type };
	const existingPropsRecord = existingProps as Record<string, unknown>;

	// First, add all valid existing props that pass schema validation
	for (const [key, value] of Object.entries(existingPropsRecord)) {
		if (isValidProperty(type, key)) {
			normalized[key] = value;
		} else if (!strict) {
			// In non-strict mode, keep unknown props
			normalized[key] = value;
		} else {
			console.warn(
				`[A2UI] Removing unknown property "${key}" from component type "${type}"`,
			);
		}
	}

	// Then, fill in missing required props with defaults
	for (const [key, defaultValue] of Object.entries(defaults)) {
		if (!(key in normalized)) {
			normalized[key] = defaultValue;
		}
	}

	return normalized as unknown as A2UIComponent;
}

/**
 * Normalizes an array of SurfaceComponents, filtering out invalid component types
 */
export function normalizeComponents(
	components: Array<{ id: string; component?: A2UIComponent; style?: Style }>,
	strict = true,
): Array<{ id: string; component: A2UIComponent; style?: Style }> {
	return components
		.filter((comp) => {
			const type = comp.component?.type;
			if (!type || !isValidComponentType(type)) {
				console.warn(
					`[A2UI] Filtering out unknown component type "${type ?? "undefined"}" (id: ${comp.id})`,
				);
				return false;
			}
			return true;
		})
		.map((comp) => ({
			...comp,
			component: comp.component
				? normalizeComponent(comp.component, strict)
				: createDefaultComponent("text"),
		}));
}
