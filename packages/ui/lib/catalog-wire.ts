import type { INode } from "./schema/flow/node";
import type { IPin } from "./schema/flow/pin";

/**
 * The node catalog as `packages/catalog/src/wire.rs` writes it for `?format=lean`.
 *
 * A section states every pin schema once and the usual value of every field once. Built-in
 * nodes come without ids, because theirs are random per hub process and every placement
 * replaces them. The built-in nodes are the same for every app, so they travel as their own
 * document, and an app's document carries only what the app adds plus the digest of the
 * built-in catalog it belongs to.
 */
export const CATALOG_WIRE_VERSION = 1;

/** Selects the sectioned catalog. A hub that predates it ignores this and sends the plain list. */
export const LEAN_CATALOG_QUERY = "format=lean";

/** At most the built-in catalogs of two builds (mid-deploy) from a hub and one from this device. */
const MAX_BUILT_INS = 3;

type Fields = Record<string, unknown>;

interface ICatalogSection {
	schemas: string[];
	node_defaults: Fields;
	pin_defaults: Fields;
	option_defaults: Fields;
	nodes: (Fields & { pins?: Fields[] })[];
}

interface ICatalogDocument {
	v: number;
	built_in?: string;
	etag?: string;
	sections: ICatalogSection[];
}

/** One decoded catalog document, or a plain node list. */
export interface ICatalogPart {
	/** Digest of the built-in catalog these nodes are or extend; absent for a plain list. */
	builtIn?: string;
	/** What to send as `If-None-Match` to learn whether this part changed. */
	etag?: string;
	nodes: INode[];
}

/**
 * Defaults split by how they may be copied: scalars are spread as one object, which is what
 * keeps expanding ~13,000 pins cheap, while empty lists and maps are made anew for each
 * object so that two nodes never share one.
 */
interface IDefaults {
	scalars: Fields;
	lists: string[];
	maps: string[];
}

const isFields = (value: unknown): value is Fields =>
	typeof value === "object" && value !== null && !Array.isArray(value);

const textOf = (value: unknown) =>
	typeof value === "string" ? value : undefined;

function defaultsOf(stated: Fields): IDefaults {
	const defaults: IDefaults = { scalars: {}, lists: [], maps: [] };
	for (const [key, value] of Object.entries(stated)) {
		if (Array.isArray(value)) defaults.lists.push(key);
		else if (isFields(value)) defaults.maps.push(key);
		else defaults.scalars[key] = value;
	}
	return defaults;
}

const withDefaults = (defaults: IDefaults, stated: Fields): Fields => {
	const full: Fields = { ...defaults.scalars, ...stated };
	// Only an absent field takes the default; a stated `null` is the node's own value.
	for (const key of defaults.lists) {
		if (full[key] === undefined) full[key] = [];
	}
	for (const key of defaults.maps) {
		if (full[key] === undefined) full[key] = {};
	}
	return full;
};

/** Ids only have to be unique and stable within one document; `taken` spans its sections. */
const claim = (taken: Set<string>, wanted: string): string => {
	let id = wanted;
	for (let copy = 2; taken.has(id); copy++) id = `${wanted}~${copy}`;
	taken.add(id);
	return id;
};

function decodeSection(section: ICatalogSection, taken: Set<string>): INode[] {
	const nodeDefaults = defaultsOf(section.node_defaults);
	const pinDefaults = defaultsOf(section.pin_defaults);
	const optionDefaults = defaultsOf(section.option_defaults);

	return section.nodes.map((stated) => {
		const node = withDefaults(nodeDefaults, stated);
		// A stated id was chosen by whoever defined the node and stays as it is. Pin ids hang
		// off `own`, which is unique even when two package nodes state the same id.
		const own = claim(
			taken,
			typeof node.id === "string" ? node.id : `catalog:${String(node.name)}`,
		);
		const pins: Record<string, Fields> = {};
		(stated.pins ?? []).forEach((statedPin, position) => {
			const pin = withDefaults(pinDefaults, statedPin);
			if (typeof pin.schema === "number") {
				pin.schema = section.schemas[pin.schema];
			}
			if (isFields(pin.options)) {
				pin.options = withDefaults(optionDefaults, pin.options);
			}
			pin.id = `pin:${own}:${position}`;
			pins[pin.id as string] = pin;
		});
		if (typeof node.id !== "string") node.id = own;
		node.pins = pins as Record<string, IPin>;
		return node as INode;
	});
}

const isSection = (value: unknown): value is ICatalogSection =>
	isFields(value) &&
	Array.isArray(value.schemas) &&
	Array.isArray(value.nodes) &&
	isFields(value.node_defaults) &&
	isFields(value.pin_defaults) &&
	isFields(value.option_defaults);

const isDocument = (value: Fields): value is Fields & ICatalogDocument =>
	Array.isArray(value.sections) && value.sections.every(isSection);

/**
 * The nodes of one catalog answer, whichever form the source answered in.
 *
 * `undefined` means the answer is a catalog layout this build does not know (a newer hub);
 * the caller then asks for the plain list. Anything that is no catalog at all throws.
 */
export function decodeCatalog(payload: unknown): ICatalogPart | undefined {
	if (Array.isArray(payload)) return { nodes: payload as INode[] };
	if (!isFields(payload) || typeof payload.v !== "number") {
		throw new Error(
			"The node catalog is neither a list nor a catalog document",
		);
	}
	if (payload.v !== CATALOG_WIRE_VERSION) return undefined;
	if (!isDocument(payload)) {
		throw new Error("The node catalog is missing parts of its sections");
	}
	const taken = new Set<string>();
	return {
		builtIn: textOf(payload.built_in),
		etag: textOf(payload.etag),
		nodes: payload.sections.flatMap((section) => decodeSection(section, taken)),
	};
}

/** Where catalogs come from: a hub, or this device's own registry. */
export interface ICatalogSource {
	/**
	 * What an app adds to the built-in catalog. A read that names the copy the caller holds
	 * may be answered `notModified`; a source without tags always answers with `data`.
	 */
	app(
		appId: string,
		etag?: string,
	): Promise<{ readonly notModified: boolean; readonly data?: unknown }>;
	/** The built-in catalog with this digest, as named by an app's document. */
	builtIn(digest: string): Promise<unknown>;
	/** The app's whole catalog as a plain list, for when the lean form cannot be used. */
	plain(appId: string): Promise<INode[]>;
}

/**
 * Reads app catalogs from one source so that the built-in nodes are downloaded and decoded
 * once for all of its apps, and an unchanged app catalog costs neither a download nor a decode.
 */
export class CatalogReader {
	private readonly builtIns = new Map<string, Promise<INode[]>>();
	private readonly held = new Map<
		string,
		{ readonly etag: string; readonly nodes: INode[] }
	>();

	constructor(private readonly source: ICatalogSource) {}

	async read(appId: string): Promise<INode[]> {
		const held = this.held.get(appId);
		const answer = await this.source.app(appId, held?.etag);
		if (answer.notModified && held) return held.nodes;

		const app = decodeCatalog(answer.data);
		if (!app) return this.source.plain(appId);
		const nodes = await this.complete(appId, app);
		if (app.etag) this.held.set(appId, { etag: app.etag, nodes });
		else this.held.delete(appId);
		return nodes;
	}

	private async complete(appId: string, app: ICatalogPart): Promise<INode[]> {
		if (!app.builtIn) return app.nodes;
		try {
			return [...(await this.builtIn(app.builtIn)), ...app.nodes];
		} catch (error) {
			// Two builds answer side by side during a deploy, and the one asked for the
			// built-in catalog may not be the one that named it. The plain list is whole.
			console.warn(
				`Built-in node catalog ${app.builtIn} unavailable, reading the plain list:`,
				error,
			);
			return this.source.plain(appId);
		}
	}

	private builtIn(digest: string): Promise<INode[]> {
		const known = this.builtIns.get(digest);
		if (known) return known;

		const loading = this.source.builtIn(digest).then((payload) => {
			const part = decodeCatalog(payload);
			if (part?.builtIn !== digest) {
				throw new Error(
					`Asked for built-in node catalog ${digest}, got ${part?.builtIn ?? "another layout"}`,
				);
			}
			return part.nodes;
		});
		// A failed read must not answer the next one.
		loading.catch(() => this.builtIns.delete(digest));
		this.builtIns.set(digest, loading);
		for (const oldest of this.builtIns.keys()) {
			if (this.builtIns.size <= MAX_BUILT_INS) break;
			this.builtIns.delete(oldest);
		}
		return loading;
	}
}
