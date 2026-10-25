import manifestJson from "../../../../core/a2ui-schema/src/component-manifest.json";

/**
 * How a prop is written and lowered. Generated from the typed Rust `*Props` structs
 * (`packages/core/a2ui-schema/src/authoring.rs`), so lowering here and in Rust agree.
 */
export type PropKind =
	| "bound"
	| "plain"
	| "list"
	| "object"
	| "style"
	| "opaque";

export interface PropSpec {
	readonly name: string;
	readonly kind: PropKind;
	readonly required?: boolean;
	readonly description?: string;
	readonly values?: readonly string[];
	readonly shape?: string;
	/** Valid on the wire but ignored by the renderer. */
	readonly unrendered?: boolean;
	readonly fields?: readonly PropSpec[];
}

export interface ComponentSpec {
	readonly type: string;
	readonly description?: string;
	readonly props: readonly PropSpec[];
}

export interface ChatUiPolicy {
	readonly components: readonly string[];
	readonly actions: readonly string[];
}

export interface ComponentManifest {
	readonly version: number;
	readonly components: readonly ComponentSpec[];
	readonly chat: ChatUiPolicy;
}

export const COMPONENT_MANIFEST = manifestJson as unknown as ComponentManifest;

export interface IndexedComponentSpec {
	readonly spec: ComponentSpec;
	readonly props: ReadonlyMap<string, PropSpec>;
}

const INDEX: ReadonlyMap<string, IndexedComponentSpec> = new Map(
	COMPONENT_MANIFEST.components.map((spec) => [
		spec.type,
		{ spec, props: new Map(spec.props.map((prop) => [prop.name, prop])) },
	]),
);

export function componentSpec(type: string): IndexedComponentSpec | undefined {
	return INDEX.get(type);
}

export function propSpec(type: string, prop: string): PropSpec | undefined {
	return INDEX.get(type)?.props.get(prop);
}

/** Component types a model may write into an `a2ui` chat fence. */
export const CHAT_UI_COMPONENTS: ReadonlySet<string> = new Set(
	COMPONENT_MANIFEST.chat.components,
);

/** Action names a chat fence may carry; every other action is stripped. */
export const CHAT_UI_ACTIONS: ReadonlySet<string> = new Set(
	COMPONENT_MANIFEST.chat.actions,
);
