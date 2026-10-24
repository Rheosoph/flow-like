import {
	authoredComponentId,
	sanitizeComponent,
} from "../../flowpilot/validateComponents";
import { getComponentChildren } from "../children";
import { getRegisteredTypes } from "../component-type-registry";
import { firstEventAction } from "../event-handlers";
import type {
	Action,
	DataEntry,
	EventHandlers,
	Style,
	Surface,
	SurfaceComponent,
} from "../types";
import {
	CHAT_EXTERNAL_LINK_ACTION,
	CHAT_SEND_MESSAGE_ACTION,
	MAX_CHAT_MESSAGE_CHARS,
	isAllowedChatAssetSource,
	mergeChatData,
	parseHttpUrl,
} from "./chat-actions";
import { isJsonObject } from "./lower";
import { CHAT_UI_COMPONENTS, componentSpec } from "./manifest";
import { type StreamValue, createStreamSplitter } from "./stream";

/**
 * Turns the body of an ```` ```a2ui ```` chat fence into a sandboxed Surface,
 * incrementally while it streams.
 *
 * Every closed object is lowered, validated and put through the chat policy
 * once, cached by its source text, so an unchanged component keeps its object
 * identity across tokens and the renderer re-renders only what changed. The
 * policy keeps the component types and actions listed in the manifest's `chat`
 * section, drops custom CSS escapes, admits only https/data:image media and
 * forces links to http(s) URLs that open outside the app.
 */

export const MAX_CHAT_COMPONENTS = 120;

export interface ChatSurfaceOptions {
	/** The reply is still streaming: unresolved child ids render a skeleton. */
	readonly streaming: boolean;
	/** A follow-up sender exists; without one, send_message buttons render disabled. */
	readonly canSendMessages: boolean;
}

export interface ChatSurfaceState {
	/** Null until a component exists. */
	readonly surface: Surface | null;
	/** Merged `{"data": …}` lines (the surface's initial data model). */
	readonly data: Readonly<Record<string, unknown>>;
	/** An object has opened but not closed yet. */
	readonly pending: boolean;
	readonly componentCount: number;
	/** Objects the chat policy or the validator removed. */
	readonly droppedCount: number;
	/** Lines abandoned as malformed JSON. */
	readonly malformedCount: number;
}

export interface ChatSurfaceBuilder {
	build(body: string, options: ChatSurfaceOptions): ChatSurfaceState;
}

type Prepared =
	| {
			readonly kind: "component";
			readonly id: string;
			readonly component: SurfaceComponent;
	  }
	| { readonly kind: "dropped"; readonly id?: string }
	| { readonly kind: "data"; readonly value: Record<string, unknown> };

type ComponentRecord = Record<string, SurfaceComponent>;
type Props = Record<string, unknown>;

const DROPPED: Prepared = { kind: "dropped" };
const EMPTY_DATA: Readonly<Record<string, unknown>> = Object.freeze({});
const UNSAFE_KEYS: ReadonlySet<string> = new Set([
	"__proto__",
	"constructor",
	"prototype",
]);
/** CSS that can load a resource or hide an escape behind an escape sequence. */
const UNSAFE_CSS_VALUE = /url\s*\(|image-set\s*\(|expression\s*\(|@import|\\/i;
/** Utility classes add `&` for arbitrary variants, which can target the host page. */
const UNSAFE_CLASS_TOKEN =
	/url\s*\(|image-set\s*\(|expression\s*\(|@import|\\|&/i;
const MAX_STYLE_DEPTH = 6;

let chatTypes: ReadonlySet<string> | undefined;

function chatComponentTypes(): ReadonlySet<string> {
	chatTypes ??= new Set(
		getRegisteredTypes().filter((type) => CHAT_UI_COMPONENTS.has(type)),
	);
	return chatTypes;
}

function asProps(component: SurfaceComponent): Props {
	return component.component as unknown as Props;
}

function withProps(
	component: SurfaceComponent,
	props: Props,
): SurfaceComponent {
	return {
		...component,
		component: props as unknown as SurfaceComponent["component"],
	};
}

/** The object of a `{"data": {...}}` line. */
function dataLineOf(value: Props) {
	const data = value.data;
	const isComponent = ["type", "component", "id"].some((key) =>
		Object.hasOwn(value, key),
	);
	return isJsonObject(data) && !isComponent ? data : undefined;
}

function literalString(value: unknown) {
	return isJsonObject(value) && typeof value.literalString === "string"
		? value.literalString
		: undefined;
}

function sendMessageAction(context: Props): Action | undefined {
	const message =
		typeof context.message === "string" ? context.message.trim() : "";
	if (!message) return undefined;
	return {
		name: CHAT_SEND_MESSAGE_ACTION,
		context: { message: message.slice(0, MAX_CHAT_MESSAGE_CHARS) },
	};
}

function externalLinkAction(context: Props): Action | undefined {
	const url = parseHttpUrl(context.url);
	return url
		? { name: CHAT_EXTERNAL_LINK_ACTION, context: { url } }
		: undefined;
}

/** The only actions a chat surface runs, rebuilt from their documented context alone. */
const CHAT_ACTION_BUILDERS: ReadonlyMap<
	string,
	(context: Props) => Action | undefined
> = new Map([
	[CHAT_SEND_MESSAGE_ACTION, sendMessageAction],
	["external_link", externalLinkAction],
]);

function chatAction(raw: unknown) {
	if (!isJsonObject(raw) || typeof raw.name !== "string") return undefined;
	const build = CHAT_ACTION_BUILDERS.get(raw.name);
	return build?.(isJsonObject(raw.context) ? raw.context : {});
}

function chatActions(raw: unknown) {
	return (Array.isArray(raw) ? raw : [])
		.map(chatAction)
		.filter((action): action is Action => action !== undefined);
}

function chatEventHandlers(raw: unknown) {
	const handlers: [string, Action[]][] = [];
	for (const [event, list] of Object.entries(isJsonObject(raw) ? raw : {})) {
		if (UNSAFE_KEYS.has(event) || !Array.isArray(list)) continue;
		const actions = chatActions(list);
		// An authored empty list suppresses the legacy fallback; a list whose every action
		// was stripped must not.
		if (list.length === 0 || actions.length > 0)
			handlers.push([event, actions]);
	}
	return Object.fromEntries(handlers);
}

const CLASS_SEPARATOR = /\s+/;

const isSafeClassToken = (token: string) =>
	token.length > 0 && !UNSAFE_CLASS_TOKEN.test(token);

function chatClassName(value: string) {
	return value.split(CLASS_SEPARATOR).filter(isSafeClassToken).join(" ");
}

function scrubStyleString(key: string, value: string) {
	if (key === "className") return chatClassName(value) || undefined;
	return UNSAFE_CSS_VALUE.test(value) ? undefined : value;
}

function scrubStyleObject(value: Props, depth: number) {
	const entries: [string, unknown][] = [];
	for (const [name, entry] of Object.entries(value)) {
		if (UNSAFE_KEYS.has(name)) continue;
		const scrubbed = scrubStyleValue(name, entry, depth + 1);
		if (scrubbed !== undefined) entries.push([name, scrubbed]);
	}
	return Object.fromEntries(entries);
}

function scrubStyleValue(key: string, value: unknown, depth: number): unknown {
	if (depth > MAX_STYLE_DEPTH) return undefined;
	if (typeof value === "string") return scrubStyleString(key, value);
	if (Array.isArray(value)) {
		return value
			.map((item) => scrubStyleValue("", item, depth + 1))
			.filter((item) => item !== undefined);
	}
	return isJsonObject(value) ? scrubStyleObject(value, depth) : value;
}

/** Background images must be https/data:image literals; a path could resolve to anything. */
function isAllowedBackground(background: unknown): boolean {
	if (!isJsonObject(background) || !Object.hasOwn(background, "image")) {
		return true;
	}
	const image = background.image;
	return (
		isJsonObject(image) && isAllowedChatAssetSource(literalString(image.url))
	);
}

/** A model-authored style without resource loads, CSS escapes or host-targeting classes. */
export function sanitizeChatStyle(style: unknown): Style | undefined {
	if (!isJsonObject(style)) return undefined;
	const { background, ...rest } = scrubStyleValue("", style, 0) as Props;
	const scrubbed =
		background !== undefined && isAllowedBackground(background)
			? { ...rest, background }
			: rest;
	return Object.keys(scrubbed).length > 0 ? (scrubbed as Style) : undefined;
}

function chatLink(
	component: SurfaceComponent,
	props: Props,
): SurfaceComponent | null {
	const url = parseHttpUrl(literalString(props.href));
	const { route: _route, queryParams: _queryParams, ...rest } = props;
	if (url) {
		return withProps(component, {
			...rest,
			href: { literalString: url },
			external: true,
			target: "_blank",
		});
	}
	const navigateAction = firstEventAction(
		rest.eventHandlers as EventHandlers | undefined,
		"navigate",
		rest.actions as Action[] | undefined,
	);
	// A link with a navigate action renders as a button, so its href is never followed.
	if (navigateAction) {
		return withProps(component, { ...rest, href: { literalString: "" } });
	}
	// Anything but an http(s) literal could navigate the app itself: keep the label as text.
	if (rest.label === undefined) return null;
	return withProps(component, { type: "text", content: rest.label });
}

/** The narrowest column a reply's grid keeps before it drops one. */
const CHAT_GRID_MIN_COLUMN = "13rem";
const CSS_LENGTH = /^\d+(?:\.\d+)?(?:px|rem|em)?$/;

function cssLength(value: unknown) {
	if (!isJsonObject(value)) return undefined;
	if (typeof value.literalNumber === "number")
		return `${value.literalNumber}px`;
	const text = literalString(value)?.trim();
	if (!text || !CSS_LENGTH.test(text)) return undefined;
	return /\d$/.test(text) ? `${text}px` : text;
}

function literalInteger(value: unknown) {
	if (!isJsonObject(value)) return undefined;
	if (typeof value.literalNumber === "number") return value.literalNumber;
	const text = literalString(value)?.trim();
	return text && /^\d+$/.test(text) ? Number(text) : undefined;
}

function columnCount(value: unknown) {
	const count = literalInteger(value);
	return count !== undefined && Number.isInteger(count) && count > 1
		? count
		: undefined;
}

/** Keeps at most the reply's column count but drops columns below a readable width. */
function responsiveGrid(props: Props): Props {
	const count = columnCount(props.columns);
	if (!count) return props;
	const gap = cssLength(props.columnGap) ?? cssLength(props.gap) ?? "0px";
	const track = `max(${CHAT_GRID_MIN_COLUMN}, calc((100% - ${count - 1} * ${gap}) / ${count}))`;
	return {
		...props,
		columns: {
			literalString: `repeat(auto-fill, minmax(min(100%, ${track}), 1fr))`,
		},
	};
}

/** A reply never knows the reader's width: grids shed columns and rows wrap unless told not to. */
function chatLayout(type: string, props: Props): Props {
	if (type === "grid") return responsiveGrid(props);
	if (type !== "row" || props.wrap !== undefined) return props;
	return { ...props, wrap: { literalBool: true } };
}

/** The chat policy for one validated component; null removes it. */
function applyChatPolicy(entry: SurfaceComponent): SurfaceComponent | null {
	const {
		actions: rawActions,
		eventHandlers: rawHandlers,
		...rest
	} = asProps(entry);
	const type = rest.type as string;
	const actions = chatActions(rawActions);
	const handlers = chatEventHandlers(rawHandlers);
	// A text field's legacy action runs on blur; in a reply it runs on Enter only.
	const submitOnly = type === "textField" && actions.length > 0;
	if (submitOnly) handlers.submit = [...(handlers.submit ?? []), ...actions];

	const styleProps = new Set(["style"]);
	for (const prop of componentSpec(type)?.spec.props ?? []) {
		if (prop.kind === "style") styleProps.add(prop.name);
	}
	const props: Props = {};
	for (const [key, value] of Object.entries(rest)) {
		if (!styleProps.has(key)) {
			props[key] = value;
			continue;
		}
		const style = sanitizeChatStyle(value);
		if (style) props[key] = style;
	}
	if (!submitOnly && actions.length > 0) props.actions = actions;
	if (Object.keys(handlers).length > 0) props.eventHandlers = handlers;

	const style = sanitizeChatStyle(entry.style);
	const sanitized: SurfaceComponent = style
		? { id: entry.id, style, component: entry.component }
		: { id: entry.id, component: entry.component };
	return type === "link"
		? chatLink(sanitized, props)
		: withProps(sanitized, chatLayout(type, props));
}

function prepareValue(value: StreamValue): Prepared {
	if (value.error !== undefined || !isJsonObject(value.value)) return DROPPED;
	const json = value.value;
	const data = dataLineOf(json);
	if (data) return { kind: "data", value: data };
	const id = authoredComponentId(json);
	const sanitized = sanitizeComponent(json, [], {
		registeredTypes: chatComponentTypes(),
	});
	const component = sanitized && applyChatPolicy(sanitized);
	return component
		? { kind: "component", id: component.id, component }
		: { kind: "dropped", id };
}

function sendsMessage(props: Props): boolean {
	const lists = [
		props.actions,
		...(isJsonObject(props.eventHandlers)
			? Object.values(props.eventHandlers)
			: []),
	];
	return lists.some(
		(list) =>
			Array.isArray(list) &&
			list.some(
				(action) =>
					isJsonObject(action) && action.name === CHAT_SEND_MESSAGE_ACTION,
			),
	);
}

function placeholderFor(id: string): SurfaceComponent {
	return {
		id,
		style: { className: "bg-muted" },
		component: {
			type: "skeleton",
			width: { literalString: "100%" },
			height: { literalString: "2rem" },
		} as unknown as SurfaceComponent["component"],
	};
}

/**
 * Every component renders at most once: a reference to a component already placed
 * (a cycle, a second parent, a duplicate in one list) is dropped. Returns the child
 * ids each rewritten parent keeps, and the referenced ids with no component.
 */
function placeTree(
	record: ComponentRecord,
	rootId: string,
): { rewrites: Map<string, Set<string>>; dangling: string[] } {
	const placed = new Set<string>([rootId]);
	const rewrites = new Map<string, Set<string>>();
	const dangling: string[] = [];
	const visit = (id: string) => {
		const kept = new Set<string>();
		let droppedReference = false;
		for (const ref of getComponentChildren(record[id])) {
			if (placed.has(ref)) {
				droppedReference = true;
				continue;
			}
			placed.add(ref);
			kept.add(ref);
			if (record[ref]) visit(ref);
			else dangling.push(ref);
		}
		if (droppedReference) rewrites.set(id, kept);
	};
	visit(rootId);
	return { rewrites, dangling };
}

type Claim = (id: unknown) => id is string;

/** Props whose items name the component they render (`contentComponentId`). */
const CONTENT_ITEMS: ReadonlyMap<string, string> = new Map([
	["tabs", "tabs"],
	["accordion", "items"],
]);

/** Accepts each kept id once, in reference order. */
function claimOnce(kept: ReadonlySet<string>): Claim {
	const claimed = new Set<string>();
	return (id: unknown): id is string => {
		if (typeof id !== "string" || !kept.has(id) || claimed.has(id)) {
			return false;
		}
		claimed.add(id);
		return true;
	};
}

function prunedChildren(children: unknown, claim: Claim) {
	if (!isJsonObject(children)) return children;
	if (Array.isArray(children.explicitList)) {
		const ids = children.explicitList.filter(claim);
		return ids.length > 0 ? { explicitList: ids } : undefined;
	}
	if (isJsonObject(children.template)) {
		return claim(children.template.templateComponentId) ? children : undefined;
	}
	return children;
}

/** Rewrite one component's child references to `kept`, first occurrence only. */
function pruneChildRefs(
	component: SurfaceComponent,
	kept: ReadonlySet<string>,
): SurfaceComponent {
	const claim = claimOnce(kept);
	const { children, ...props }: Props = asProps(component);
	const nextChildren = prunedChildren(children, claim);
	if (nextChildren !== undefined) props.children = nextChildren;
	const contentKey = CONTENT_ITEMS.get(props.type as string);
	const items = contentKey ? props[contentKey] : undefined;
	if (contentKey && Array.isArray(items)) {
		props[contentKey] = items.map((item) =>
			isJsonObject(item) && !claim(item.contentComponentId)
				? { ...item, contentComponentId: "" }
				: item,
		);
	}
	return withProps(component, props);
}

interface Collected {
	readonly components: readonly SurfaceComponent[];
	readonly ids: ReadonlySet<string>;
	/** Every authored id, rendered or dropped; none of them gets a placeholder. */
	readonly knownIds: ReadonlySet<string>;
	readonly sources: readonly Props[];
	readonly droppedCount: number;
}

/** The first component per id wins; past the cap the rest are dropped. */
function collectPrepared(items: readonly Prepared[]) {
	const components: SurfaceComponent[] = [];
	const ids = new Set<string>();
	const knownIds = new Set<string>();
	const sources: Props[] = [];
	let droppedCount = 0;
	for (const item of items) {
		if (item.kind === "data") {
			sources.push(item.value);
			continue;
		}
		if (item.id) knownIds.add(item.id);
		if (
			item.kind === "dropped" ||
			ids.has(item.id) ||
			components.length >= MAX_CHAT_COMPONENTS
		) {
			droppedCount++;
			continue;
		}
		ids.add(item.id);
		components.push(item.component);
	}
	return { components, ids, knownIds, sources, droppedCount };
}

function sameRecord(left: ComponentRecord, right: ComponentRecord): boolean {
	const keys = Object.keys(left);
	if (keys.length !== Object.keys(right).length) return false;
	return keys.every((key) => left[key] === right[key]);
}

function sameList(
	left: readonly unknown[],
	right: readonly unknown[],
): boolean {
	return (
		left.length === right.length &&
		left.every((value, index) => value === right[index])
	);
}

export function createChatSurfaceBuilder(
	surfaceId: string,
): ChatSurfaceBuilder {
	const splitter = createStreamSplitter();
	const prepared = new Map<string, Prepared>();
	const placeholders = new Map<string, SurfaceComponent>();
	const disabledSends = new WeakMap<SurfaceComponent, SurfaceComponent>();
	const prunedRefs = new WeakMap<
		SurfaceComponent,
		{ readonly key: string; readonly component: SurfaceComponent }
	>();
	let lastValues: readonly StreamValue[] | undefined;
	let lastOptions: ChatSurfaceOptions | undefined;
	let lastState: ChatSurfaceState | undefined;
	let lastRecord: ComponentRecord = {};
	let lastData: {
		readonly sources: readonly Props[];
		readonly data: Readonly<Record<string, unknown>>;
		readonly model: DataEntry[];
	} = { sources: [], data: EMPTY_DATA, model: [] };

	const prepare = (value: StreamValue): Prepared => {
		let entry = prepared.get(value.text);
		if (!entry) {
			entry = prepareValue(value);
			prepared.set(value.text, entry);
		}
		return entry;
	};

	const sendDisabled = (component: SurfaceComponent): SurfaceComponent => {
		const props = asProps(component);
		if (props.type !== "button" || !sendsMessage(props)) return component;
		let disabled = disabledSends.get(component);
		if (!disabled) {
			disabled = withProps(component, {
				...props,
				disabled: { literalBool: true },
			});
			disabledSends.set(component, disabled);
		}
		return disabled;
	};

	const pruned = (
		component: SurfaceComponent,
		kept: ReadonlySet<string>,
	): SurfaceComponent => {
		const key = [...kept].join("\u0000");
		const cached = prunedRefs.get(component);
		if (cached?.key === key) return cached.component;
		const next = pruneChildRefs(component, kept);
		prunedRefs.set(component, { key, component: next });
		return next;
	};

	const placeholder = (id: string): SurfaceComponent => {
		let entry = placeholders.get(id);
		if (!entry) {
			entry = placeholderFor(id);
			placeholders.set(id, entry);
		}
		return entry;
	};

	const dataModel = (sources: readonly Props[]) => {
		if (!sameList(sources, lastData.sources)) {
			const data = sources.reduce<Record<string, unknown>>(
				(merged, source) => mergeChatData(merged, source),
				{},
			);
			lastData = {
				sources,
				data,
				model: Object.entries(data).map(([path, value]) => ({ path, value })),
			};
		}
		return lastData;
	};

	const componentRecord = (
		collected: Collected,
		rootId: string,
		options: ChatSurfaceOptions,
	): ComponentRecord => {
		const record: ComponentRecord = {};
		for (const component of collected.components) {
			record[component.id] = options.canSendMessages
				? component
				: sendDisabled(component);
		}
		const { rewrites, dangling } = placeTree(record, rootId);
		for (const [id, kept] of rewrites) {
			const component = record[id];
			if (component) record[id] = pruned(component, kept);
		}
		if (options.streaming) {
			const unresolved = dangling.filter((id) => !collected.knownIds.has(id));
			for (const id of unresolved) record[id] = placeholder(id);
		}
		return sameRecord(record, lastRecord) ? lastRecord : record;
	};

	const surfaceFor = (
		record: ComponentRecord,
		rootId: string,
		model: DataEntry[],
	): Surface => {
		lastRecord = record;
		const previous = lastState?.surface;
		return previous &&
			previous.components === record &&
			previous.rootComponentId === rootId &&
			previous.dataModel === model
			? previous
			: {
					id: surfaceId,
					rootComponentId: rootId,
					components: record,
					dataModel: model,
				};
	};

	const assemble = (
		values: readonly StreamValue[],
		options: ChatSurfaceOptions,
		pending: boolean,
		malformedCount: number,
	): ChatSurfaceState => {
		const collected = collectPrepared(values.map(prepare));
		const { data, model } = dataModel(collected.sources);
		const base = {
			data,
			pending,
			componentCount: collected.components.length,
			droppedCount: collected.droppedCount,
			malformedCount,
		};
		const first = collected.components[0];
		if (!first) return { ...base, surface: null };
		const rootId = collected.ids.has("root") ? "root" : first.id;
		const record = componentRecord(collected, rootId, options);
		return { ...base, surface: surfaceFor(record, rootId, model) };
	};

	return {
		build(body, options) {
			const snapshot = splitter.push(body);
			const pending = snapshot.pending !== "";
			if (
				lastState &&
				lastValues === snapshot.values &&
				lastOptions?.streaming === options.streaming &&
				lastOptions?.canSendMessages === options.canSendMessages
			) {
				if (
					lastState.pending !== pending ||
					lastState.malformedCount !== snapshot.malformed
				) {
					lastState = {
						...lastState,
						pending,
						malformedCount: snapshot.malformed,
					};
				}
				return lastState;
			}
			lastValues = snapshot.values;
			lastOptions = options;
			lastState = assemble(
				snapshot.values,
				options,
				pending,
				snapshot.malformed,
			);
			return lastState;
		},
	};
}
