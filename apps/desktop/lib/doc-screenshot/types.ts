export const DOC_SCREENSHOT_PLAN_SCHEMA =
	"flow-like.doc-screenshot-plan/v1" as const;
export const DOC_SCREENSHOT_TAURI_FIXTURE_SCHEMA =
	"flow-like.doc-screenshot-tauri-fixture/v1" as const;
export const DOC_SCREENSHOT_HTTP_FIXTURE_SCHEMA =
	"flow-like.doc-screenshot-http-fixture/v1" as const;
export const DOC_SCREENSHOT_RESULT_SCHEMA =
	"flow-like.doc-screenshot-result/v1" as const;

export type DocScreenshotApp = "desktop" | "web";
export type DocScreenshotTheme = "light" | "dark";
export type DocScreenshotFormat = "png" | "webp" | "jpeg";
export type DocScreenshotCaptureMode = "viewport" | "fullPage" | "element";
export type DocScreenshotMouseButton = "left" | "middle" | "right";
export type DocScreenshotKeyboardModifier =
	| "Alt"
	| "Control"
	| "Meta"
	| "Shift";

export type JsonPrimitive = string | number | boolean | null;
export type JsonValue =
	| JsonPrimitive
	| JsonValue[]
	| { [key: string]: JsonValue };

export interface DocScreenshotViewport {
	width: number;
	height: number;
	deviceScaleFactor: number;
}

export interface DocScreenshotDefaults {
	viewport: DocScreenshotViewport;
	theme: DocScreenshotTheme;
	format: DocScreenshotFormat;
	quality?: number;
	timeoutMs: number;
	settleMs: number;
	disableAnimations: boolean;
	hideScrollbars: boolean;
}

export type DocScreenshotQueryValue =
	| JsonPrimitive
	| JsonPrimitive[]
	| undefined;

export interface DocScreenshotTarget {
	selector: string;
	index?: number;
}

export interface DocScreenshotGotoStep {
	type: "goto";
	path: string;
	query?: Record<string, DocScreenshotQueryValue>;
}

export interface DocScreenshotClickStep extends DocScreenshotTarget {
	type: "click";
	button?: DocScreenshotMouseButton;
	clickCount?: number;
	modifiers?: DocScreenshotKeyboardModifier[];
}

export interface DocScreenshotDragStep extends DocScreenshotTarget {
	type: "drag";
	targetSelector: string;
	targetIndex?: number;
	steps?: number;
	button?: DocScreenshotMouseButton;
	release?: boolean;
}

export interface DocScreenshotFillStep extends DocScreenshotTarget {
	type: "fill";
	value?: string;
	valueEnv?: string;
}

export interface DocScreenshotTypeStep extends DocScreenshotTarget {
	type: "type";
	value?: string;
	valueEnv?: string;
	delayMs?: number;
}

export interface DocScreenshotPressStep {
	type: "press";
	key: string;
	selector?: string;
	index?: number;
}

export interface DocScreenshotSelectStep extends DocScreenshotTarget {
	type: "select";
	values: string[];
}

export interface DocScreenshotCheckStep extends DocScreenshotTarget {
	type: "check";
	checked?: boolean;
}

export interface DocScreenshotHoverStep extends DocScreenshotTarget {
	type: "hover";
}

export interface DocScreenshotScrollStep {
	type: "scroll";
	selector?: string;
	index?: number;
	x?: number;
	y?: number;
}

export interface DocScreenshotWaitForStep {
	type: "waitFor";
	selector?: string;
	urlIncludes?: string;
	text?: string;
	state?: "attached" | "visible" | "hidden" | "detached";
	timeoutMs?: number;
}

export interface DocScreenshotDelayStep {
	type: "delay";
	ms: number;
}

export interface DocScreenshotCaptureStep {
	type: "capture";
	name: string;
	output?: string;
	mode?: DocScreenshotCaptureMode;
	selector?: string;
	index?: number;
	padding?: number;
	format?: DocScreenshotFormat;
	quality?: number;
	hideSelectors?: string[];
}

export type DocScreenshotStep =
	| DocScreenshotGotoStep
	| DocScreenshotClickStep
	| DocScreenshotDragStep
	| DocScreenshotFillStep
	| DocScreenshotTypeStep
	| DocScreenshotPressStep
	| DocScreenshotSelectStep
	| DocScreenshotCheckStep
	| DocScreenshotHoverStep
	| DocScreenshotScrollStep
	| DocScreenshotWaitForStep
	| DocScreenshotDelayStep
	| DocScreenshotCaptureStep;

export interface DocScreenshotScenario {
	name: string;
	path: string;
	query?: Record<string, DocScreenshotQueryValue>;
	viewport?: Partial<DocScreenshotViewport>;
	theme?: DocScreenshotTheme;
	localStorage?: Record<string, string>;
	sessionStorage?: Record<string, string>;
	steps: DocScreenshotStep[];
}

export interface DocScreenshotPlan {
	schema: typeof DOC_SCREENSHOT_PLAN_SCHEMA;
	app: DocScreenshotApp;
	baseUrl?: string;
	outputDir: string;
	tauriFixture?: string;
	httpFixture?: string;
	defaults: DocScreenshotDefaults;
	scenarios: DocScreenshotScenario[];
}

export interface DocScreenshotTauriFixture {
	schema: typeof DOC_SCREENSHOT_TAURI_FIXTURE_SCHEMA;
	strict: boolean;
	responses: Record<string, JsonValue | { $error: string }>;
}

export interface DocScreenshotHttpFixtureRequest {
	method: string;
	url: string;
	body?: string;
}

export interface DocScreenshotHttpFixtureResponse {
	status: number;
	headers: Record<string, string>;
	body?: string;
	json?: JsonValue;
}

export interface DocScreenshotHttpFixtureRoute {
	request: DocScreenshotHttpFixtureRequest;
	response: DocScreenshotHttpFixtureResponse;
}

export interface DocScreenshotHttpFixture {
	schema: typeof DOC_SCREENSHOT_HTTP_FIXTURE_SCHEMA;
	strict: boolean;
	blockedOrigins: string[];
	blockedEndpoints: string[];
	routes: DocScreenshotHttpFixtureRoute[];
}

export interface DocScreenshotStepResult {
	index: number;
	type: DocScreenshotStep["type"];
	status: "passed" | "failed";
	durationMs: number;
	urlAfter: string;
	error?: string;
}

export interface DocScreenshotArtifact {
	id: string;
	path: string;
	mimeType: string;
	bytes: number;
	sha256: string;
	mode: DocScreenshotCaptureMode;
	selector?: string;
	capturedAt: string;
	css: {
		width: number;
		height: number;
	};
	pixels: {
		width: number;
		height: number;
	};
	effectiveScale: {
		x: number;
		y: number;
	};
}

export interface DocScreenshotScenarioResult {
	name: string;
	passed: boolean;
	requestedUrl: string;
	finalUrl: string;
	title: string;
	httpStatus?: number;
	durationMs: number;
	render: {
		viewport: DocScreenshotViewport;
		theme: DocScreenshotTheme;
		locale: string;
		timezone: string;
		reducedMotion: true;
		colorProfile: "srgb";
		disableAnimations: boolean;
		hideScrollbars: boolean;
		settleMs: number;
	};
	steps: DocScreenshotStepResult[];
	artifacts: DocScreenshotArtifact[];
	diagnostics: {
		consoleErrors: number;
		pageErrors: number;
		requestFailures: number;
		warnings: number;
	};
	error?: string;
}

export interface DocScreenshotResult {
	schema: typeof DOC_SCREENSHOT_RESULT_SCHEMA;
	runId: string;
	passed: boolean;
	startedAt: string;
	finishedAt: string;
	durationMs: number;
	baseUrl: string;
	browser: {
		product: "Chromium";
		version: string;
		headless: true;
	};
	scenarios: DocScreenshotScenarioResult[];
	summary: {
		scenarios: number;
		scenariosPassed: number;
		screenshots: number;
	};
}
