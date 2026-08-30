/**
 * Component schema definitions for A2UI validation.
 * This schema defines all valid properties for each component type,
 * used to validate and sanitize AI-generated components.
 */

import { SEMANTIC_BOX_TAGS } from "../a2ui/semantic-box-tags";

export type PropType =
	| "string"
	| "number"
	| "boolean"
	| "boundValue"
	| "children"
	| "options"
	| "json"
	| "actions"
	| "any";

export interface PropSchema {
	type: PropType;
	required?: boolean;
	enum?: string[];
	description?: string;
}

export type ComponentSchema = Record<string, PropSchema>;

const COMMON_COMPONENT_SCHEMA: ComponentSchema = {
	hidden: {
		type: "boundValue",
		description: "Hide this component in the rendered UI",
	},
};

// Registry of all valid component types and their allowed properties
export const COMPONENT_SCHEMAS: Record<string, ComponentSchema> = {
	// Layout
	row: {
		type: { type: "string", required: true },
		gap: { type: "boundValue" },
		align: {
			type: "boundValue",
			enum: ["start", "center", "end", "stretch", "baseline"],
		},
		justify: {
			type: "boundValue",
			enum: ["start", "center", "end", "between", "around", "evenly"],
		},
		wrap: { type: "boundValue" },
		reverse: { type: "boundValue" },
		children: { type: "children", required: true },
	},
	column: {
		type: { type: "string", required: true },
		gap: { type: "boundValue" },
		align: { type: "boundValue", enum: ["start", "center", "end", "stretch"] },
		justify: {
			type: "boundValue",
			enum: ["start", "center", "end", "between", "around", "evenly"],
		},
		wrap: { type: "boundValue" },
		reverse: { type: "boundValue" },
		children: { type: "children", required: true },
	},
	stack: {
		type: { type: "string", required: true },
		align: { type: "boundValue" },
		width: {
			type: "boundValue",
			description: "Width of stack container (required for proper display)",
		},
		height: {
			type: "boundValue",
			description: "Height of stack container (required for proper display)",
		},
		children: { type: "children", required: true },
	},
	grid: {
		type: { type: "string", required: true },
		columns: { type: "boundValue" },
		rows: { type: "boundValue" },
		gap: { type: "boundValue" },
		autoFlow: { type: "boundValue", enum: ["row", "column", "dense"] },
		children: { type: "children", required: true },
	},
	scrollArea: {
		type: { type: "string", required: true },
		direction: { type: "boundValue", enum: ["vertical", "horizontal", "both"] },
		children: { type: "children", required: true },
	},
	aspectRatio: {
		type: { type: "string", required: true },
		ratio: { type: "boundValue" },
		children: { type: "children" },
	},
	overlay: {
		type: { type: "string", required: true },
		children: { type: "children", required: true },
	},
	absolute: {
		type: { type: "string", required: true },
		width: { type: "boundValue" },
		height: { type: "boundValue" },
		children: { type: "children", required: true },
	},
	box: {
		type: { type: "string", required: true },
		as: {
			type: "boundValue",
			enum: [...SEMANTIC_BOX_TAGS],
		},
		children: { type: "children", required: false },
	},
	center: {
		type: { type: "string", required: true },
		inline: { type: "boundValue" },
		children: { type: "children", required: true },
	},
	spacer: {
		type: { type: "string", required: true },
		flex: { type: "boundValue" },
		size: { type: "boundValue" },
	},

	// Display
	text: {
		type: { type: "string", required: true },
		content: { type: "boundValue", required: true },
		variant: {
			type: "boundValue",
			enum: [
				"h1",
				"h2",
				"h3",
				"h4",
				"h5",
				"h6",
				"body",
				"caption",
				"code",
				"label",
			],
		},
		size: {
			type: "boundValue",
			enum: ["xs", "sm", "md", "lg", "xl", "2xl", "3xl"],
		},
		weight: {
			type: "boundValue",
			enum: ["normal", "medium", "semibold", "bold"],
		},
		color: { type: "boundValue" },
		align: { type: "boundValue", enum: ["left", "center", "right"] },
		truncate: { type: "boundValue" },
	},
	image: {
		type: { type: "string", required: true },
		src: { type: "boundValue", required: true },
		alt: { type: "boundValue" },
		fallback: { type: "boundValue" },
		fit: {
			type: "boundValue",
			enum: ["cover", "contain", "fill", "none", "scale-down"],
		},
		loading: { type: "boundValue", enum: ["lazy", "eager"] },
	},
	video: {
		type: { type: "string", required: true },
		src: { type: "boundValue", required: true },
		autoplay: { type: "boundValue" },
		loop: { type: "boundValue" },
		muted: { type: "boundValue" },
		controls: { type: "boundValue" },
		poster: { type: "boundValue" },
	},
	icon: {
		type: { type: "string", required: true },
		name: { type: "boundValue", required: true },
		size: { type: "boundValue" },
		color: { type: "boundValue" },
		strokeWidth: { type: "boundValue" },
	},
	markdown: {
		type: { type: "string", required: true },
		content: { type: "boundValue", required: true },
		allowHtml: { type: "boundValue" },
	},
	divider: {
		type: { type: "string", required: true },
		orientation: { type: "boundValue", enum: ["horizontal", "vertical"] },
		thickness: { type: "boundValue" },
	},
	badge: {
		type: { type: "string", required: true },
		content: { type: "boundValue", required: true },
		variant: {
			type: "boundValue",
			enum: ["default", "secondary", "outline", "destructive"],
		},
	},
	avatar: {
		type: { type: "string", required: true },
		src: { type: "boundValue" },
		fallback: { type: "boundValue" },
		size: { type: "boundValue", enum: ["xs", "sm", "md", "lg", "xl"] },
	},
	userProfile: {
		type: { type: "string", required: true },
		value: {
			type: "boundValue",
			required: true,
			description:
				'User subject/sub ID. The "local" sub of an unauthenticated execution renders the current user',
		},
		variant: {
			type: "boundValue",
			enum: ["avatar", "chip", "row", "detailed", "card"],
		},
		avatarSize: {
			type: "boundValue",
			enum: ["xs", "sm", "md", "lg", "xl", "2xl"],
		},
		showHover: { type: "boundValue" },
		showEmail: { type: "boundValue" },
		showDescription: { type: "boundValue" },
		showUserId: { type: "boundValue" },
		showProfileLink: { type: "boundValue" },
		fallbackLabel: { type: "boundValue" },
		muted: { type: "boundValue" },
	},
	progress: {
		type: { type: "string", required: true },
		value: { type: "boundValue", required: true },
		max: { type: "boundValue" },
		showLabel: { type: "boundValue" },
		variant: { type: "boundValue" },
	},
	spinner: {
		type: { type: "string", required: true },
		size: { type: "boundValue", enum: ["sm", "md", "lg"] },
	},
	skeleton: {
		type: { type: "string", required: true },
		width: { type: "boundValue" },
		height: { type: "boundValue" },
		rounded: { type: "boundValue" },
	},
	lottie: {
		type: { type: "string", required: true },
		src: { type: "boundValue", required: true },
		autoplay: { type: "boundValue" },
		loop: { type: "boundValue" },
		speed: { type: "boundValue" },
	},
	iframe: {
		type: { type: "string", required: true },
		src: { type: "boundValue" },
		srcdoc: { type: "boundValue" },
		width: { type: "boundValue" },
		height: { type: "boundValue" },
		title: { type: "boundValue" },
		loading: { type: "boundValue" },
		border: { type: "boundValue" },
		sandbox: { type: "boundValue" },
		allow: { type: "boundValue" },
		referrerPolicy: { type: "boundValue" },
	},
	plotlyChart: {
		type: { type: "string", required: true },
		chartType: { type: "boundValue" },
		title: { type: "boundValue" },
		series: { type: "any" },
		xAxis: { type: "any" },
		yAxis: { type: "any" },
		width: { type: "boundValue" },
		height: { type: "boundValue" },
		responsive: { type: "boundValue" },
		showLegend: { type: "boundValue" },
		legendPosition: { type: "boundValue" },
	},
	nivoChart: {
		type: { type: "string", required: true },
		chartType: { type: "boundValue" },
		data: { type: "boundValue" },
		width: { type: "boundValue" },
		height: { type: "boundValue" },
		indexBy: { type: "boundValue" },
		keys: { type: "boundValue" },
		colors: { type: "boundValue" },
		animate: { type: "boundValue" },
		showLegend: { type: "boundValue" },
		legendPosition: { type: "boundValue" },
		margin: { type: "boundValue" },
		axisBottom: { type: "boundValue" },
		axisLeft: { type: "boundValue" },
		barStyle: { type: "any" },
		lineStyle: { type: "any" },
		pieStyle: { type: "any" },
	},
	table: {
		type: { type: "string", required: true },
		columns: { type: "boundValue" },
		data: { type: "boundValue" },
		striped: { type: "boundValue" },
		bordered: { type: "boundValue" },
		hoverable: { type: "boundValue" },
		compact: { type: "boundValue" },
		stickyHeader: { type: "boundValue" },
		sortable: { type: "boundValue" },
		searchable: { type: "boundValue" },
		paginated: { type: "boundValue" },
		pageSize: { type: "boundValue" },
	},
	tableRow: {
		type: { type: "string", required: true },
		children: { type: "children" },
	},
	tableCell: {
		type: { type: "string", required: true },
		content: { type: "boundValue" },
		colSpan: { type: "boundValue" },
		rowSpan: { type: "boundValue" },
		align: { type: "boundValue" },
	},
	filePreview: {
		type: { type: "string", required: true },
		src: { type: "boundValue", required: true },
		url: { type: "boundValue" },
		filename: { type: "boundValue" },
		mimeType: { type: "boundValue" },
		fileType: {
			type: "boundValue",
			enum: ["pdf", "image", "video", "audio", "code", "text", "unknown"],
		},
		showControls: { type: "boundValue" },
		fit: {
			type: "boundValue",
			enum: ["contain", "cover", "fill", "none", "scaleDown", "scale-down"],
		},
		fallbackText: { type: "boundValue" },
		height: { type: "boundValue" },
		showDownload: { type: "boundValue" },
		loading: { type: "boundValue", enum: ["lazy", "eager"] },
		variant: {
			type: "boundValue",
			enum: [
				"conservative",
				"waveform",
				"orb",
				"vortex",
				"shader",
				"aurora",
				"pulse",
			],
		},
		autoPlay: { type: "boundValue" },
	},
	diffView: {
		type: { type: "string", required: true },
		original: { type: "boundValue", required: true },
		modified: { type: "boundValue", required: true },
		mode: { type: "boundValue", enum: ["split", "unified", "inline"] },
		kind: {
			type: "boundValue",
			enum: ["auto", "text", "code", "markdown", "json", "document"],
		},
		language: {
			type: "boundValue",
			enum: [
				"plaintext",
				"typescript",
				"javascript",
				"json",
				"python",
				"rust",
				"markdown",
				"html",
				"css",
				"bash",
				"yaml",
				"sql",
			],
		},
		markdownMode: { type: "boundValue", enum: ["source", "rendered"] },
		showLineNumbers: { type: "boundValue" },
		wordWrap: { type: "boundValue" },
		wordLevel: { type: "boundValue" },
		collapseUnchanged: { type: "boundValue" },
		contextLines: { type: "boundValue" },
		showStats: { type: "boundValue" },
		originalLabel: { type: "boundValue" },
		modifiedLabel: { type: "boundValue" },
		ignoreWhitespace: { type: "boundValue" },
		ignoreCase: { type: "boundValue" },
		trimTrailingWhitespace: { type: "boundValue" },
		swapSides: { type: "boundValue" },
	},
	boundingBoxOverlay: {
		type: { type: "string", required: true },
		src: { type: "boundValue", required: true },
		alt: { type: "boundValue" },
		boxes: { type: "boundValue" },
		showLabels: { type: "boundValue" },
		showConfidence: { type: "boundValue" },
		strokeWidth: { type: "boundValue" },
		fontSize: { type: "boundValue" },
		fit: { type: "boundValue", enum: ["contain", "cover", "fill"] },
		normalized: { type: "boundValue" },
		interactive: { type: "boundValue" },
		actions: { type: "actions" },
	},

	// Interactive
	button: {
		type: { type: "string", required: true },
		label: { type: "boundValue", required: true },
		variant: {
			type: "boundValue",
			enum: ["default", "secondary", "outline", "ghost", "destructive", "link"],
		},
		size: { type: "boundValue", enum: ["sm", "md", "lg", "icon"] },
		disabled: { type: "boundValue" },
		loading: { type: "boundValue" },
		icon: { type: "boundValue", description: "Lucide icon name" },
		iconPosition: { type: "boundValue", enum: ["left", "right"] },
		tooltip: { type: "boundValue" },
		actions: { type: "actions" },
	},
	feedback: {
		type: { type: "string", required: true },
		mode: {
			type: "boundValue",
			enum: ["icon", "compact", "segmented", "rating", "extended"],
		},
		size: { type: "boundValue", enum: ["sm", "md", "lg"] },
		title: { type: "boundValue" },
		description: { type: "boundValue" },
		positiveLabel: { type: "boundValue" },
		negativeLabel: { type: "boundValue" },
		positiveRating: { type: "boundValue" },
		negativeRating: { type: "boundValue" },
		showComment: { type: "boundValue" },
		commentMode: { type: "boundValue", enum: ["none", "inline", "modal"] },
		commentLabel: { type: "boundValue" },
		commentPlaceholder: { type: "boundValue" },
		commentTitle: { type: "boundValue" },
		commentDescription: { type: "boundValue" },
		commentSubmitLabel: { type: "boundValue" },
		commentCancelLabel: { type: "boundValue" },
		feedbackId: { type: "boundValue" },
		includeState: { type: "boundValue" },
		pageContextMode: { type: "boundValue", enum: ["none", "path", "query"] },
		pageContextQueryParamAllowlist: { type: "boundValue" },
		pageContextQueryParamDenylist: { type: "boundValue" },
		includePageHash: { type: "boundValue" },
		successMessage: { type: "boundValue" },
		disabled: { type: "boundValue" },
		actions: { type: "actions" },
	},
	appLink: {
		type: { type: "string", required: true },
		target: { type: "boundValue", enum: ["config", "settings", "overview"] },
		label: { type: "boundValue" },
		variant: {
			type: "boundValue",
			enum: ["default", "secondary", "outline", "ghost", "destructive", "link"],
		},
		size: { type: "boundValue", enum: ["sm", "md", "lg", "icon"] },
		icon: { type: "boundValue", description: "Lucide icon name" },
		iconPosition: { type: "boundValue", enum: ["left", "right"] },
		appId: { type: "boundValue" },
		eventId: { type: "boundValue" },
		disabled: { type: "boundValue" },
	},
	textField: {
		type: { type: "string", required: true },
		value: { type: "boundValue", required: true },
		placeholder: { type: "boundValue" },
		label: { type: "boundValue" },
		helperText: { type: "boundValue" },
		error: { type: "boundValue" },
		disabled: { type: "boundValue" },
		inputType: {
			type: "boundValue",
			enum: ["text", "email", "password", "number", "tel", "url", "search"],
		},
		multiline: { type: "boundValue" },
		rows: { type: "boundValue" },
		maxLength: { type: "boundValue" },
		required: { type: "boundValue" },
		debounceMs: {
			type: "boundValue",
			description:
				"Pause in ms before the 'input' event fires (default 400, min 100)",
		},
		actions: { type: "actions" },
	},
	richText: {
		type: { type: "string", required: true },
		value: { type: "boundValue", required: true },
		label: { type: "boundValue" },
		helperText: { type: "boundValue" },
		placeholder: { type: "boundValue" },
		error: { type: "boundValue" },
		disabled: { type: "boundValue" },
		readOnly: { type: "boundValue" },
		uploadPrefix: {
			type: "boundValue",
			description:
				"Storage folder for pasted or dropped images (default a2ui/<surface>/<component>)",
		},
		uploadScope: { type: "boundValue", enum: ["app", "user"] },
		minHeight: { type: "boundValue" },
		maxHeight: { type: "boundValue" },
		debounceMs: {
			type: "boundValue",
			description:
				"Pause in ms before the 'change' event fires (default 600, min 100)",
		},
		actions: { type: "actions" },
	},
	select: {
		type: { type: "string", required: true },
		value: { type: "boundValue", required: true },
		options: { type: "options", required: true },
		placeholder: { type: "boundValue" },
		label: { type: "boundValue" },
		disabled: { type: "boundValue" },
		multiple: { type: "boundValue" },
		searchable: { type: "boundValue" },
		actions: { type: "actions" },
	},
	slider: {
		type: { type: "string", required: true },
		value: { type: "boundValue", required: true },
		min: { type: "boundValue" },
		max: { type: "boundValue" },
		step: { type: "boundValue" },
		disabled: { type: "boundValue" },
		showValue: { type: "boundValue" },
		label: { type: "boundValue" },
		debounceMs: {
			type: "boundValue",
			description:
				"Pause in ms before the 'input' event fires (default 400, min 100)",
		},
		actions: { type: "actions" },
	},
	checkbox: {
		type: { type: "string", required: true },
		checked: { type: "boundValue", required: true },
		label: { type: "boundValue" },
		disabled: { type: "boundValue" },
		indeterminate: { type: "boundValue" },
		actions: { type: "actions" },
	},
	switch: {
		type: { type: "string", required: true },
		checked: { type: "boundValue", required: true },
		label: { type: "boundValue" },
		disabled: { type: "boundValue" },
		actions: { type: "actions" },
	},
	radioGroup: {
		type: { type: "string", required: true },
		value: { type: "boundValue", required: true },
		options: { type: "options", required: true },
		disabled: { type: "boundValue" },
		orientation: { type: "boundValue", enum: ["horizontal", "vertical"] },
		label: { type: "boundValue" },
		actions: { type: "actions" },
	},
	dateTimeInput: {
		type: { type: "string", required: true },
		value: { type: "boundValue", required: true },
		mode: { type: "boundValue", enum: ["date", "time", "datetime"] },
		disabled: { type: "boundValue" },
		label: { type: "boundValue" },
		actions: { type: "actions" },
	},
	fileInput: {
		type: { type: "string", required: true },
		value: { type: "boundValue" },
		label: { type: "boundValue" },
		accept: { type: "boundValue" },
		multiple: { type: "boundValue" },
		directory: { type: "boundValue" },
		disabled: { type: "boundValue" },
		actions: { type: "actions" },
	},
	imageInput: {
		type: { type: "string", required: true },
		value: { type: "boundValue" },
		label: { type: "boundValue" },
		accept: { type: "boundValue" },
		multiple: { type: "boundValue" },
		disabled: { type: "boundValue" },
		showPreview: { type: "boundValue" },
		actions: { type: "actions" },
	},
	voiceInput: {
		type: { type: "string", required: true },
		value: { type: "boundValue" },
		label: { type: "boundValue" },
		helperText: { type: "boundValue" },
		mode: { type: "boundValue", enum: ["record", "stt"] },
		invoke: { type: "boundValue", enum: ["manual", "hold", "auto"] },
		variant: {
			type: "boundValue",
			enum: [
				"conservative",
				"waveform",
				"orb",
				"vortex",
				"shader",
				"aurora",
				"pulse",
			],
		},
		size: { type: "boundValue", enum: ["sm", "md", "lg"] },
		color: { type: "boundValue" },
		recordingColor: { type: "boundValue" },
		resultMode: {
			type: "boundValue",
			enum: ["player", "autoplay", "summary"],
		},
		maxDuration: { type: "boundValue" },
		autoStop: { type: "boundValue" },
		silenceThreshold: { type: "boundValue" },
		silenceDuration: { type: "boundValue" },
		disabled: { type: "boundValue" },
		error: { type: "boundValue" },
		visualizer: { type: "boundValue", enum: ["waveform", "bars"] },
		actions: { type: "actions" },
	},
	link: {
		type: { type: "string", required: true },
		label: { type: "boundValue", required: true },
		href: { type: "boundValue" },
		variant: { type: "boundValue", enum: ["default", "muted"] },
		underline: { type: "boundValue", enum: ["always", "hover", "none"] },
		disabled: { type: "boundValue" },
		external: { type: "boundValue" },
		actions: { type: "actions" },
	},
	imageLabeler: {
		type: { type: "string", required: true },
		src: { type: "boundValue", required: true },
		regions: { type: "boundValue" },
		tools: { type: "boundValue" },
		showLabels: { type: "boundValue" },
		actions: { type: "actions" },
	},
	imageHotspot: {
		type: { type: "string", required: true },
		src: { type: "boundValue", required: true },
		hotspots: { type: "boundValue" },
		actions: { type: "actions" },
	},

	// Container
	card: {
		type: { type: "string", required: true },
		title: { type: "boundValue" },
		description: { type: "boundValue" },
		hoverable: { type: "boundValue" },
		clickable: { type: "boundValue" },
		children: { type: "children" },
		actions: { type: "actions" },
	},
	modal: {
		type: { type: "string", required: true },
		open: { type: "boundValue" },
		title: { type: "boundValue" },
		description: { type: "boundValue" },
		closeOnOverlay: { type: "boundValue" },
		closeOnEscape: { type: "boundValue" },
		showCloseButton: { type: "boundValue" },
		size: { type: "boundValue", enum: ["sm", "md", "lg", "xl", "full"] },
		children: { type: "children" },
		actions: { type: "actions" },
	},
	tabs: {
		type: { type: "string", required: true },
		value: { type: "boundValue" },
		items: { type: "any" },
		orientation: { type: "boundValue", enum: ["horizontal", "vertical"] },
		variant: { type: "boundValue", enum: ["default", "pills", "underline"] },
		actions: { type: "actions" },
	},
	accordion: {
		type: { type: "string", required: true },
		items: { type: "any" },
		multiple: { type: "boundValue" },
		collapsible: { type: "boundValue" },
		defaultExpanded: { type: "boundValue" },
		actions: { type: "actions" },
	},
	drawer: {
		type: { type: "string", required: true },
		open: { type: "boundValue" },
		side: { type: "boundValue", enum: ["left", "right", "top", "bottom"] },
		title: { type: "boundValue" },
		size: { type: "boundValue" },
		overlay: { type: "boundValue" },
		closable: { type: "boundValue" },
		children: { type: "children" },
		actions: { type: "actions" },
	},
	tooltip: {
		type: { type: "string", required: true },
		content: { type: "boundValue", required: true },
		side: { type: "boundValue", enum: ["top", "right", "bottom", "left"] },
		delayMs: { type: "boundValue" },
		children: { type: "children", required: true },
	},
	popover: {
		type: { type: "string", required: true },
		open: { type: "boundValue" },
		contentComponentId: { type: "string" },
		side: { type: "boundValue", enum: ["top", "right", "bottom", "left"] },
		trigger: { type: "boundValue", enum: ["click", "hover"] },
		closeOnClickOutside: { type: "boundValue" },
		children: { type: "children" },
		actions: { type: "actions" },
	},

	// Game
	canvas2d: {
		type: { type: "string", required: true },
		width: { type: "boundValue" },
		height: { type: "boundValue" },
		backgroundColor: { type: "boundValue" },
		pixelPerfect: { type: "boundValue" },
		children: { type: "children" },
	},
	sprite: {
		type: { type: "string", required: true },
		src: { type: "boundValue", required: true },
		x: { type: "boundValue" },
		y: { type: "boundValue" },
		rotation: { type: "boundValue" },
		scale: { type: "boundValue" },
		opacity: { type: "boundValue" },
		flipX: { type: "boundValue" },
		flipY: { type: "boundValue" },
		zIndex: { type: "boundValue" },
	},
	shape: {
		type: { type: "string", required: true },
		shapeType: { type: "boundValue", required: true },
		x: { type: "boundValue" },
		y: { type: "boundValue" },
		width: { type: "boundValue" },
		height: { type: "boundValue" },
		fill: { type: "boundValue" },
		stroke: { type: "boundValue" },
		strokeWidth: { type: "boundValue" },
	},
	scene3d: {
		type: { type: "string", required: true },
		width: { type: "boundValue" },
		height: { type: "boundValue" },
		cameraType: { type: "boundValue", enum: ["perspective", "orthographic"] },
		backgroundColor: { type: "boundValue" },
		children: { type: "children" },
	},
	model3d: {
		type: { type: "string", required: true },
		src: { type: "boundValue", required: true },
		position: { type: "boundValue" },
		rotation: { type: "boundValue" },
		scale: { type: "boundValue" },
		castShadow: { type: "boundValue" },
		receiveShadow: { type: "boundValue" },
		autoRotate: { type: "boundValue" },
		rotateSpeed: { type: "boundValue" },
		viewerHeight: { type: "boundValue" },
		backgroundColor: { type: "boundValue" },
		cameraDistance: { type: "boundValue" },
		fov: { type: "boundValue" },
		cameraAngle: {
			type: "boundValue",
			enum: ["front", "side", "top", "isometric"],
		},
		cameraPosition: { type: "boundValue" },
		cameraTarget: { type: "boundValue" },
		enableControls: { type: "boundValue" },
		enableZoom: { type: "boundValue" },
		enablePan: { type: "boundValue" },
		autoRotateCamera: { type: "boundValue" },
		cameraRotateSpeed: { type: "boundValue" },
		lightingPreset: {
			type: "boundValue",
			enum: ["neutral", "warm", "cool", "studio", "dramatic"],
		},
		ambientLight: { type: "boundValue" },
		directionalLight: { type: "boundValue" },
		fillLight: { type: "boundValue" },
		rimLight: { type: "boundValue" },
		lightColor: { type: "boundValue" },
		showGround: { type: "boundValue" },
		groundColor: { type: "boundValue" },
		groundSize: { type: "boundValue" },
		groundOffsetY: { type: "boundValue" },
		groundFollowCamera: { type: "boundValue" },
		enableReflections: { type: "boundValue" },
		environment: {
			type: "boundValue",
			enum: [
				"studio",
				"sunset",
				"dawn",
				"night",
				"warehouse",
				"forest",
				"apartment",
				"city",
				"park",
				"lobby",
			],
		},
		environmentSource: {
			type: "boundValue",
			enum: ["local", "preset", "polyhaven", "custom"],
		},
		useHdrBackground: { type: "boundValue" },
		polyhavenHdri: {
			type: "boundValue",
			enum: [
				"studio_small_03",
				"studio_small_09",
				"brown_photostudio_02",
				"empty_warehouse_01",
				"industrial_sunset_02",
				"sunset_in_the_chalk_quarry",
				"rooftop_night",
				"abandoned_factory_canteen_01",
				"forest_slope",
				"green_point_park",
				"lebombo",
				"spruit_sunrise",
				"syferfontein_18d_clear_puresky",
				"venice_sunset",
				"potsdamer_platz",
			],
		},
		polyhavenResolution: {
			type: "boundValue",
			enum: ["1k", "2k", "4k", "8k"],
		},
		hdriUrl: { type: "boundValue" },
	},
	dialogue: {
		type: { type: "string", required: true },
		text: { type: "boundValue", required: true },
		speakerName: { type: "boundValue" },
		typewriter: { type: "boundValue" },
		typewriterSpeed: { type: "boundValue" },
	},
	characterPortrait: {
		type: { type: "string", required: true },
		image: { type: "boundValue", required: true },
		position: { type: "boundValue", enum: ["left", "center", "right"] },
		size: { type: "boundValue", enum: ["small", "medium", "large"] },
		dimmed: { type: "boundValue" },
	},
	choiceMenu: {
		type: { type: "string", required: true },
		choices: { type: "boundValue", required: true },
		title: { type: "boundValue" },
		layout: { type: "boundValue", enum: ["vertical", "horizontal", "grid"] },
		actions: { type: "actions" },
	},
	inventoryGrid: {
		type: { type: "string", required: true },
		items: { type: "boundValue", required: true },
		columns: { type: "boundValue" },
		rows: { type: "boundValue" },
		cellSize: { type: "boundValue" },
		actions: { type: "actions" },
	},
	healthBar: {
		type: { type: "string", required: true },
		value: { type: "boundValue", required: true },
		maxValue: { type: "boundValue" },
		label: { type: "boundValue" },
		showValue: { type: "boundValue" },
		fillColor: { type: "boundValue" },
		backgroundColor: { type: "boundValue" },
		variant: { type: "boundValue", enum: ["bar", "segmented", "circular"] },
	},
	miniMap: {
		type: { type: "string", required: true },
		mapImage: { type: "boundValue" },
		width: { type: "boundValue" },
		height: { type: "boundValue" },
		markers: { type: "boundValue" },
		playerX: { type: "boundValue" },
		playerY: { type: "boundValue" },
		playerRotation: { type: "boundValue" },
		actions: { type: "actions" },
	},
	geoMap: {
		type: { type: "string", required: true },
		viewport: { type: "boundValue" },
		markers: { type: "boundValue" },
		routes: { type: "boundValue" },
		showControls: { type: "boundValue" },
		showZoom: { type: "boundValue" },
		showCompass: { type: "boundValue" },
		showLocate: { type: "boundValue" },
		showFullscreen: { type: "boundValue" },
		interactive: { type: "boundValue" },
		controlPosition: {
			type: "boundValue",
			enum: ["top-left", "top-right", "bottom-left", "bottom-right"],
		},
		clusterMarkers: { type: "boundValue" },
		clusterRadius: { type: "boundValue" },
		clusterMaxZoom: { type: "boundValue" },
		actions: { type: "actions" },
	},
	graph: {
		type: { type: "string", required: true },
		nodes: { type: "boundValue", required: true },
		edges: { type: "boundValue" },
		labelStyles: { type: "boundValue" },
		showToolbar: { type: "boundValue" },
		showSearch: { type: "boundValue" },
		showLegend: { type: "boundValue" },
		showInspector: { type: "boundValue" },
		height: { type: "boundValue" },
		actions: { type: "actions" },
	},
	ontologyGraph: {
		type: { type: "string", required: true },
		ontologyId: { type: "boundValue", required: true },
		appId: { type: "boundValue" },
		limit: { type: "boundValue" },
		allowExpand: { type: "boundValue" },
		allowSearch: { type: "boundValue" },
		allowPaths: { type: "boundValue" },
		allowActions: { type: "boundValue" },
		allowCypher: { type: "boundValue" },
		allowStyleEdit: { type: "boundValue" },
		allowLimitChange: { type: "boundValue" },
		showToolbar: { type: "boundValue" },
		showLegend: { type: "boundValue" },
		height: { type: "boundValue" },
		actions: { type: "actions" },
	},
	calendar: {
		type: { type: "string", required: true },
		events: { type: "boundValue", required: true },
		view: {
			type: "boundValue",
			enum: ["month", "week", "day", "agenda"],
		},
		date: { type: "boundValue" },
		title: { type: "boundValue" },
		density: {
			type: "boundValue",
			enum: ["compact", "default", "comfortable"],
		},
		editable: { type: "boundValue" },
		selectable: { type: "boundValue" },
		firstDayOfWeek: { type: "boundValue" },
		minTime: { type: "boundValue" },
		maxTime: { type: "boundValue" },
		slotDuration: { type: "boundValue" },
		showWeekends: { type: "boundValue" },
		showNowIndicator: { type: "boundValue" },
		showAllDay: { type: "boundValue" },
		showViewSwitcher: { type: "boundValue" },
		locale: { type: "boundValue" },
		height: { type: "boundValue" },
		responsive: { type: "boundValue" },
		compactBreakpoint: { type: "boundValue" },
		actions: { type: "actions" },
	},
	gantt: {
		type: { type: "string", required: true },
		tasks: { type: "boundValue", required: true },
		view: {
			type: "boundValue",
			enum: ["day", "week", "month", "quarter", "compact"],
		},
		title: { type: "boundValue" },
		density: {
			type: "boundValue",
			enum: ["compact", "default", "comfortable"],
		},
		editable: { type: "boundValue" },
		draggable: { type: "boundValue" },
		resizable: { type: "boundValue" },
		showDependencies: { type: "boundValue" },
		showProgress: { type: "boundValue" },
		showToday: { type: "boundValue" },
		showViewSwitcher: { type: "boundValue" },
		showTaskList: { type: "boundValue" },
		taskListWidth: { type: "boundValue" },
		shadeWeekends: { type: "boundValue" },
		rowHeight: { type: "boundValue" },
		columns: { type: "boundValue" },
		height: { type: "boundValue" },
		responsive: { type: "boundValue" },
		compactBreakpoint: { type: "boundValue" },
		actions: { type: "actions" },
	},
};

/**
 * Get all valid component types
 */
export function getValidComponentTypes(): string[] {
	return Object.keys(COMPONENT_SCHEMAS);
}

/**
 * Check if a component type is valid
 */
export function isValidComponentType(type: string): boolean {
	return type in COMPONENT_SCHEMAS;
}

/**
 * Get the schema for a component type
 */
export function getComponentSchema(type: string): ComponentSchema | undefined {
	const schema = COMPONENT_SCHEMAS[type];
	return schema ? { ...COMMON_COMPONENT_SCHEMA, ...schema } : undefined;
}

/**
 * Get all valid property names for a component type
 */
export function getValidProperties(type: string): string[] {
	const schema = COMPONENT_SCHEMAS[type];
	return schema
		? Array.from(
				new Set([
					...Object.keys(COMMON_COMPONENT_SCHEMA),
					...Object.keys(schema),
				]),
			)
		: [];
}

/**
 * Check if a property is valid for a component type
 */
export function isValidProperty(type: string, propName: string): boolean {
	const schema = COMPONENT_SCHEMAS[type];
	return schema
		? propName in schema || propName in COMMON_COMPONENT_SCHEMA
		: false;
}
