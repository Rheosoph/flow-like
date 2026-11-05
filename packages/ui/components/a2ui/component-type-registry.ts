import {
	type A2UIComponentType,
	COMPONENT_PROPS,
} from "./component-prop-manifest";

/**
 * Pure registry of component type names accepted by A2UI validation.
 *
 * Renderers live in ComponentRegistry.tsx. Keeping names here lets validation run in workers
 * and test processes without loading React, browser-only controls, or renderer dependencies.
 */
const registeredTypes = new Set<string>(
	Object.keys(COMPONENT_PROPS) as A2UIComponentType[],
);

/** Keep dynamic extension types in sync when a renderer is registered. */
export function registerComponentType(type: string): void {
	registeredTypes.add(type);
}

export function getRegisteredTypes(): string[] {
	return [...registeredTypes];
}
