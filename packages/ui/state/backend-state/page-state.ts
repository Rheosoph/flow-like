import type { SurfaceComponent } from "../../components/a2ui/types";
import type { Version } from "./widget-state";

export type PageLayoutType =
	| "freeform"
	| "stack"
	| "grid"
	| "sidebar"
	| "holyGrail";

export interface Spacing {
	value: string;
}

export interface BackgroundImage {
	url: { literalString: string } | { path: string };
	size?: string;
	position?: string;
	repeat?: string;
}

export type Background =
	| { color: string }
	| { image: BackgroundImage }
	| { gradient: unknown }
	| { blur: string };

export interface PageMeta {
	description?: string;
	ogImage?: string;
	keywords: string[];
	favicon?: string;
	themeColor?: string;
}

export interface WidgetInstance {
	widgetId: string;
	instanceId: string;
	position?: any;
	customizationValues: Record<string, Uint8Array>;
	/** Values for exposed props (key is the exposed prop id) */
	exposedPropValues?: Record<string, Uint8Array>;
	styleOverride?: any;
}

export type PageContent =
	| { Widget: WidgetInstance }
	| { Component: SurfaceComponent }
	| { ComponentRef: string };

/** Widget definition stored in page refs */
export interface IWidgetRef {
	id: string;
	name: string;
	description?: string;
	rootComponentId: string;
	components: SurfaceComponent[];
	dataModel?: unknown[];
	customizationOptions?: unknown[];
	exposedProps?: unknown[];
	actions?: unknown[];
	tags: string[];
	catalogId?: string;
	thumbnail?: string;
	version?: [number, number, number];
	createdAt: string;
	updatedAt: string;
}

export interface CanvasSettings {
	backgroundColor?: string;
	backgroundImage?: string;
	padding?: string;
	customCss?: string;
}

export interface IPage {
	id: string;
	name: string;
	route?: string;
	title?: string;
	/** Canvas settings for page styling (background, padding, custom CSS) */
	canvasSettings?: CanvasSettings;
	content: PageContent[];
	layoutType: PageLayoutType;
	attachedElementId?: string;
	meta?: PageMeta;
	components: SurfaceComponent[];
	version?: Version;
	createdAt: string;
	updatedAt: string;
	boardId?: string;
	/** Node ID (from events_simple) to execute when page loads */
	onLoadEventId?: string;
	/** Node ID to execute when page unloads/user navigates away */
	onUnloadEventId?: string;
	/** Node ID to execute on a timed interval */
	onIntervalEventId?: string;
	/** Interval time in seconds (must be > 0) */
	onIntervalSeconds?: number;
	/** Widget definitions referenced by widget instances on this page. Key is instance ID */
	widgetRefs?: Record<string, IWidgetRef>;
	/** When true, cache the last rendered state and show it instantly while onLoad runs */
	cache?: boolean;
}

export interface PageListItem {
	appId: string;
	pageId: string;
	boardId?: string;
	name: string;
	description?: string;
}

export interface IPageState {
	getPages(appId: string, boardId?: string): Promise<PageListItem[]>;
	getPage(appId: string, pageId: string, boardId?: string): Promise<IPage>;
	createPage(
		appId: string,
		pageId: string,
		name: string,
		route: string,
		boardId: string,
		title?: string,
	): Promise<IPage>;
	updatePage(appId: string, page: IPage): Promise<void>;
	deletePage(appId: string, pageId: string, boardId: string): Promise<void>;
	getOpenPages(): Promise<[string, string, string][]>;
	closePage(pageId: string): Promise<void>;
}
