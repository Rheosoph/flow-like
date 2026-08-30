import type {
	DataEntry,
	SurfaceComponent,
	WidgetAction,
} from "../../components/a2ui/types";
import type { IMetadata } from "../../lib";
import { nowSystemTime } from "../../lib/time/now";

export type Version = [number, number, number];

export type VersionType = "Major" | "Minor" | "Patch";

export interface CustomizationOption {
	id: string;
	label: string;
	description?: string;
	customizationType: CustomizationType;
	defaultValue?: Uint8Array;
	validations: ValidationRule[];
	group?: string;
}

export type CustomizationType =
	| "String"
	| "Number"
	| "Boolean"
	| "Color"
	| "ImageUrl"
	| "Icon"
	| "Enum"
	| "Json";

export interface ValidationRule {
	ruleType: string;
	value?: Uint8Array;
	message?: string;
}

/** Exposed property from widget component that can be customized in pages */
export interface ExposedProp {
	id: string;
	label: string;
	description?: string;
	/** The component ID within the widget that this prop targets */
	targetComponentId: string;
	/** The property path on the component (e.g., "content", "style.className", "data") */
	propertyPath: string;
	propType: ExposedPropType;
	defaultValue?: Uint8Array;
	/** Group for organizing props in the UI (e.g., "Content", "Style", "Data") */
	group?: string;
}

export type ExposedPropType =
	| "String"
	| "Number"
	| "Boolean"
	| "Color"
	| "ImageUrl"
	| "Icon"
	| { Enum: { choices: string[] } }
	| "Json"
	| "TailwindClass"
	| "StyleObject"
	| "BoundValue";

export interface IWidget {
	id: string;
	name: string;
	description?: string;
	rootComponentId: string;
	components: SurfaceComponent[];
	dataModel: DataEntry[];
	customizationOptions: CustomizationOption[];
	/** Props exposed from widget components that can be customized when used in a page */
	exposedProps?: ExposedProp[];
	catalogId?: string;
	thumbnail?: string;
	tags: string[];
	version?: Version;
	createdAt: string;
	updatedAt: string;
	/** Widget actions that can be triggered by elements and bound to workflows */
	actions?: WidgetAction[];
}

export interface IWidgetState {
	getWidgets(
		appId: string,
		language?: string,
	): Promise<[string, string, IMetadata | undefined][]>;
	getWidget(
		appId: string,
		widgetId: string,
		version?: Version,
	): Promise<IWidget>;
	createWidget(
		appId: string,
		widgetId: string,
		name: string,
		description?: string,
	): Promise<IWidget>;
	updateWidget(appId: string, widget: IWidget): Promise<void>;
	/**
	 * A widget's name lives both on the widget record and in its metadata
	 * sidecar, and the sidecar wins wherever it is non-blank. Writing only one
	 * of them leaves the rename invisible, so every rename goes through here.
	 */
	renameWidget(
		appId: string,
		widgetId: string,
		name: string,
		language?: string,
	): Promise<void>;
	deleteWidget(appId: string, widgetId: string): Promise<void>;
	createWidgetVersion(
		appId: string,
		widgetId: string,
		versionType: VersionType,
	): Promise<Version>;
	getWidgetVersions(appId: string, widgetId: string): Promise<Version[]>;
	getOpenWidgets(): Promise<[string, string, string][]>;
	closeWidget(widgetId: string): Promise<void>;
	getWidgetMeta(
		appId: string,
		widgetId: string,
		language?: string,
	): Promise<IMetadata>;
	pushWidgetMeta(
		appId: string,
		widgetId: string,
		metadata: IMetadata,
		language?: string,
	): Promise<void>;
}

/**
 * Backend-agnostic body of {@link IWidgetState.renameWidget}: it is composed
 * purely of other interface calls, so both backends delegate here instead of
 * keeping two copies of the record-plus-sidecar ordering. Each half is skipped
 * when it already carries the name, so callers that cannot tell whether the
 * name changed can call this unconditionally.
 */
export async function applyWidgetRename(
	state: IWidgetState,
	appId: string,
	widgetId: string,
	name: string,
	language?: string,
): Promise<void> {
	// A blank name used to be survivable — the sidecar kept the old one and the fallback
	// masked it. Now that both sides are written it would erase the widget's only label,
	// so an empty rename is refused rather than half-applied.
	const trimmed = name.trim();
	if (!trimmed) {
		throw new Error("A widget name cannot be empty");
	}

	const widget = await state.getWidget(appId, widgetId);
	if (widget.name !== trimmed) {
		await state.updateWidget(appId, { ...widget, name: trimmed });
	}

	let metadata: IMetadata | undefined;
	try {
		metadata = await state.getWidgetMeta(appId, widgetId, language);
	} catch {
		metadata = undefined;
	}

	if (metadata?.name === trimmed) return;

	const now = nowSystemTime();
	const base: IMetadata = metadata ?? {
		name: trimmed,
		description: widget.description ?? "",
		long_description: "",
		tags: widget.tags ?? [],
		created_at: now,
		updated_at: now,
		preview_media: [],
	};

	await state.pushWidgetMeta(
		appId,
		widgetId,
		{ ...base, name: trimmed, updated_at: now },
		language,
	);
}
