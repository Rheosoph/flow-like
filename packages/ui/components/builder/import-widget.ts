import { createId } from "@paralleldrive/cuid2";
import type { IMetadata } from "../../lib/schema/bit/bit";
import { nowSystemTime } from "../../lib/time/now";
import type {
	IWidget,
	IWidgetState,
	Version,
} from "../../state/backend-state/widget-state";

const COMPONENT_REFERENCE_KEYS = new Set([
	"child",
	"entryPointChild",
	"contentChild",
	"rootComponentId",
	"root_component_id",
	"contentComponentId",
	"baseComponentId",
	"componentId",
	"component_id",
	"templateComponentId",
	"sourceComponentId",
	"targetComponentId",
	"target_component_id",
	"elementId",
	"element_id",
]);
const COMPONENT_LIST_KEYS = new Set([
	"explicitList",
	"element_ids",
	"elementIds",
]);
const INSTANCE_REFERENCE_KEYS = new Set([
	"instanceId",
	"instance_id",
	"widgetInstanceId",
	"widget_instance_id",
]);

interface WidgetCopy {
	sourceAppId: string;
	source: IWidget;
	widget: IWidget;
	componentIds: Map<string, string>;
	instanceIds: Map<string, string>;
	metadata?: IMetadata;
}

function record(value: unknown): value is Record<string, unknown> {
	return !!value && typeof value === "object" && !Array.isArray(value);
}

function key(appId: string, widgetId: string, version?: Version): string {
	return JSON.stringify([appId, widgetId, version ?? null]);
}

function widgetVersion(value: unknown): Version | undefined {
	return Array.isArray(value) &&
		value.length === 3 &&
		value.every((part) => Number.isInteger(part) && part >= 0)
		? (value as Version)
		: undefined;
}

/** Strings and binding paths are content unless their field names identify references. */
function rewriteBoundReference(
	value: unknown,
	translate: (id: string) => string,
): unknown {
	if (typeof value === "string") return translate(value);
	if (!record(value)) return value;
	const result = { ...value };
	for (const field of ["literalString", "defaultValue", "default_value"]) {
		if (typeof result[field] === "string")
			result[field] = translate(result[field]);
	}
	return result;
}

function rewritePropertyValue(
	value: unknown,
	rewrite: (value: unknown) => unknown,
	propertyPath?: string,
	componentIds?: Map<string, string>,
): unknown {
	let result = rewrite(value);
	const field = propertyPath?.split(".").at(-1);
	if (field && componentIds) {
		const translate = (id: string) => componentIds.get(id) ?? id;
		if (COMPONENT_REFERENCE_KEYS.has(field)) {
			result = rewriteBoundReference(result, translate);
		} else if (COMPONENT_LIST_KEYS.has(field) && Array.isArray(result)) {
			result = result.map((id) => rewriteBoundReference(id, translate));
		}
	}
	return result;
}

function rewriteDefault(
	value: unknown,
	rewrite: (value: unknown) => unknown,
	propertyPath?: string,
	componentIds?: Map<string, string>,
): unknown {
	const bytes =
		value instanceof Uint8Array
			? value
			: Array.isArray(value) &&
					value.every(
						(byte) => Number.isInteger(byte) && byte >= 0 && byte <= 255,
					)
				? new Uint8Array(value)
				: undefined;
	let decoded = value;
	if (bytes) {
		try {
			decoded = JSON.parse(new TextDecoder().decode(bytes));
		} catch {
			return value;
		}
	}
	const result = rewritePropertyValue(
		decoded,
		rewrite,
		propertyPath,
		componentIds,
	);
	if (!bytes) return result;
	const encoded = new TextEncoder().encode(JSON.stringify(result));
	return value instanceof Uint8Array ? encoded : Array.from(encoded);
}

function rewriteWidget(
	copy: WidgetCopy,
	copies: Map<string, WidgetCopy>,
	targetAppId: string,
) {
	const componentId = (id: string) => copy.componentIds.get(id) ?? id;
	const rewrite = (value: unknown, scope: WidgetCopy = copy): unknown => {
		if (value instanceof Uint8Array) return value;
		if (Array.isArray(value))
			return value.map((child) => rewrite(child, scope));
		if (!record(value)) return value;
		const componentId = (id: string) => scope.componentIds.get(id) ?? id;
		const instanceId = (id: string) => scope.instanceIds.get(id) ?? id;
		const result: Record<string, unknown> = {};
		const appId =
			typeof value.appId === "string" && value.appId
				? value.appId
				: typeof value.app_id === "string" && value.app_id
					? value.app_id
					: scope.sourceAppId;
		const widgetId = value.widgetId ?? value.widget_id;
		const dependency =
			value.type !== "microWidgetInstance" && typeof widgetId === "string"
				? copies.get(key(appId, widgetId, widgetVersion(value.version)))
				: undefined;
		for (const [field, child] of Object.entries(value)) {
			if (COMPONENT_REFERENCE_KEYS.has(field)) {
				result[field] = rewriteBoundReference(
					rewrite(child, scope),
					componentId,
				);
			} else if (COMPONENT_LIST_KEYS.has(field) && Array.isArray(child)) {
				result[field] = child.map((id) =>
					rewriteBoundReference(id, componentId),
				);
			} else if (INSTANCE_REFERENCE_KEYS.has(field)) {
				result[field] = rewriteBoundReference(child, instanceId);
			} else if ((field === "appId" || field === "app_id") && dependency) {
				result[field] = targetAppId;
			} else if (
				(field === "widgetId" || field === "widget_id") &&
				dependency
			) {
				result[field] = dependency.widget.id;
			} else if (field === "version" && dependency) {
				// The imported definition starts a new revision history.
			} else if (field === "literalJson" && typeof child === "string") {
				try {
					result[field] = JSON.stringify(rewrite(JSON.parse(child), scope));
				} catch {
					result[field] = child;
				}
			} else if (field === "inlineWidgetDef") {
				// Inline definitions own their component IDs independently of the parent.
				result[field] = structuredClone(child);
			} else if (
				value.type === "widgetInstance" &&
				!dependency &&
				(field === "runtimeChildUpdates" || field === "exposedPropValues")
			) {
				// A widget already in the destination keeps its own component ID space.
				result[field] = structuredClone(child);
			} else if (field === "runtimeChildUpdates" && record(child)) {
				const childScope = dependency ?? scope;
				result[field] = Object.fromEntries(
					Object.entries(child).map(([id, updates]) => [
						childScope.componentIds.get(id) ?? id,
						rewrite(updates, childScope),
					]),
				);
			} else if (field === "exposedPropValues" && dependency && record(child)) {
				result[field] = Object.fromEntries(
					Object.entries(child).map(([id, value]) => [
						id,
						rewritePropertyValue(
							value,
							(value) => rewrite(value, dependency),
							dependency.source.exposedProps?.find((prop) => prop.id === id)
								?.propertyPath,
							dependency.componentIds,
						),
					]),
				);
			} else {
				result[field] = rewrite(child, scope);
			}
		}
		if (value.type === "widgetInstance" && dependency) {
			result.appId = targetAppId;
		}
		return result;
	};

	const widget = copy.widget;
	widget.components = copy.source.components.map((component) => {
		const cloned = rewrite(component) as typeof component;
		cloned.id = componentId(component.id);
		if ("id" in cloned.component) cloned.component.id = cloned.id;
		return cloned;
	});
	widget.dataModel = rewrite(copy.source.dataModel) as IWidget["dataModel"];
	widget.actions = rewrite(copy.source.actions) as IWidget["actions"];
	widget.exposedProps = copy.source.exposedProps?.map((prop) => ({
		...prop,
		targetComponentId: componentId(prop.targetComponentId),
		...(prop.defaultValue === undefined
			? {}
			: {
					defaultValue: rewriteDefault(
						prop.defaultValue,
						rewrite,
						prop.propertyPath,
						copy.componentIds,
					) as Uint8Array,
				}),
	}));
	widget.customizationOptions = copy.source.customizationOptions.map(
		(option) => ({
			...option,
			...(option.defaultValue === undefined
				? {}
				: {
						defaultValue: rewriteDefault(
							option.defaultValue,
							rewrite,
						) as Uint8Array,
					}),
		}),
	);
}

/** Copy a foreign widget and its widget dependencies before placing an instance. */
export async function importWidget(
	state: IWidgetState,
	sourceAppId: string,
	widgetId: string,
	targetAppId: string,
): Promise<IWidget> {
	if (sourceAppId === targetAppId)
		return state.getWidget(sourceAppId, widgetId);
	const copies = new Map<string, WidgetCopy>();
	const ordered: WidgetCopy[] = [];
	const createdAt = new Date().toISOString();
	const prepare = async (
		appId: string,
		id: string,
		version?: Version,
	): Promise<WidgetCopy> => {
		const sourceKey = key(appId, id, version);
		const existing = copies.get(sourceKey);
		if (existing) return existing;
		const source = structuredClone(
			await state.getWidgetAuthoritative(appId, id, version),
		);
		if (!source.components?.length || !source.rootComponentId) {
			throw new Error(
				`Widget ${source.name || id} has no components to import`,
			);
		}
		const componentIds = new Map<string, string>();
		const rootId = source.components.some(
			(component) => component.id === "root",
		)
			? "root"
			: source.components.some(
						(component) => component.id === source.rootComponentId,
					)
				? source.rootComponentId
				: source.components[0].id;
		const instanceIds = new Map<string, string>();
		for (const component of source.components) {
			if (componentIds.has(component.id)) {
				throw new Error(
					`Widget ${source.name || id} has duplicate component IDs`,
				);
			}
			// WidgetBuilder uses this reserved root when opening or saving a definition.
			componentIds.set(
				component.id,
				component.id === rootId
					? "root"
					: `${component.component.type}-${createId()}`,
			);
			if (
				component.component.type === "widgetInstance" ||
				component.component.type === "microWidgetInstance"
			) {
				instanceIds.set(component.component.instanceId, `widget-${createId()}`);
			}
		}
		const copy: WidgetCopy = {
			sourceAppId: appId,
			source,
			componentIds,
			instanceIds,
			widget: {
				...structuredClone(source),
				id: createId(),
				rootComponentId: "root",
				version: [0, 0, 1],
				createdAt,
				updatedAt: createdAt,
			},
		};
		// Allocate before following dependencies so shared children and cycles reuse the same copy.
		copies.set(sourceKey, copy);
		for (const component of source.components) {
			if (component.component.type !== "widgetInstance") continue;
			const dependency = component.component;
			const dependencyAppId = dependency.appId || appId;
			if (dependencyAppId === targetAppId) continue;
			await prepare(
				dependencyAppId,
				dependency.widgetId,
				widgetVersion(
					(dependency as unknown as Record<string, unknown>).version,
				),
			);
		}
		try {
			copy.metadata = await state.getWidgetMeta(appId, id);
		} catch {
			// A definition remains usable when its optional metadata sidecar is absent.
		}
		if (copy.metadata?.name.trim())
			copy.widget.name = copy.metadata.name.trim();
		ordered.push(copy);
		return copy;
	};

	const root = await prepare(sourceAppId, widgetId);
	for (const copy of ordered) rewriteWidget(copy, copies, targetAppId);
	const attempted: string[] = [];
	try {
		for (const copy of ordered) {
			// An upsert can fail after the remote write succeeds, so include its ID in cleanup.
			attempted.push(copy.widget.id);
			await state.updateWidget(targetAppId, copy.widget);
			const now = nowSystemTime();
			await state.pushWidgetMeta(targetAppId, copy.widget.id, {
				...(copy.metadata ?? {
					description: copy.widget.description ?? "",
					long_description: "",
					tags: copy.widget.tags,
					preview_media: [],
				}),
				name: copy.widget.name,
				created_at: now,
				updated_at: now,
			});
		}
	} catch (error) {
		for (const id of attempted.reverse()) {
			try {
				await state.deleteWidget(targetAppId, id);
			} catch (cleanupError) {
				console.warn(
					"Failed to remove an incomplete widget import",
					id,
					cleanupError,
				);
			}
		}
		throw error;
	}
	return root.widget;
}
