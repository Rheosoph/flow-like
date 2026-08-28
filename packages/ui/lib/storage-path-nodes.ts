import { createId } from "@paralleldrive/cuid2";
import type { INode } from "./schema/flow/node";
import type { IPin } from "./schema/flow/pin";
import type { IStorageScope } from "./storage-tree";
import { convertJsonToUint8Array } from "./uint8";

/**
 * The two nodes that address a stored file, built from the catalog rather than a frozen
 * JSON blob, so a pin rename in the catalog cannot leave a dead template behind.
 *
 * Which directory node is correct is not a guess — it is what the storage APIs list:
 * app storage is `apps/<appId>/upload`, which is `get_upload_dir()` and therefore
 * `path_from_upload_dir`; user storage is `users/<sub>/apps/<appId>`, which is
 * `get_user_dir(node = false)` and therefore `path_from_user_dir`. `path_from_storage_dir`
 * is a *third* directory (`<board>/storage`) that nothing in the file browser ever shows.
 */
const DIR_NODE_BY_SCOPE: Record<IStorageScope, string> = {
	app: "path_from_upload_dir",
	user: "path_from_user_dir",
};

const CHILD_NODE = "child";
/** Enough that the two nodes do not overlap at the default zoom. */
const CHILD_OFFSET_X = 260;

function clonePins(
	pins: Record<string, IPin>,
	remap: Map<string, string>,
): Record<string, IPin> {
	const cloned: Record<string, IPin> = {};
	for (const pin of Object.values(pins)) {
		const id = createId();
		remap.set(pin.id, id);
		cloned[id] = { ...pin, id, connected_to: [], depends_on: [] };
	}
	return cloned;
}

function findPin(node: INode, name: string): IPin | undefined {
	return Object.values(node.pins).find((pin) => pin.name === name);
}

/**
 * A `<dir> → child("relative/path")` pair, wired and positioned, ready to paste.
 *
 * Returns null when the catalog does not carry both nodes — an app whose catalog was
 * filtered, or one loaded before the catalog arrived.
 */
export function buildStoragePathNodes({
	catalog,
	scope,
	path,
	position = { x: 0, y: 0 },
}: {
	catalog: readonly INode[] | undefined;
	scope: IStorageScope;
	/** Path relative to the storage root. `child` splits it on `/` at runtime. */
	path: string;
	position?: { x: number; y: number };
}): { nodes: INode[] } | null {
	const dirTemplate = catalog?.find(
		(node) => node.name === DIR_NODE_BY_SCOPE[scope],
	);
	const childTemplate = catalog?.find((node) => node.name === CHILD_NODE);
	if (!dirTemplate || !childTemplate) return null;

	const remap = new Map<string, string>();
	const dir: INode = {
		...dirTemplate,
		id: createId(),
		coordinates: [position.x, position.y, 0],
		pins: clonePins(dirTemplate.pins, remap),
	};
	const child: INode = {
		...childTemplate,
		id: createId(),
		coordinates: [position.x + CHILD_OFFSET_X, position.y, 0],
		pins: clonePins(childTemplate.pins, remap),
	};

	const dirOut = findPin(dir, "path");
	const parentIn = findPin(child, "parent_path");
	const childName = findPin(child, "child_name");
	if (!dirOut || !parentIn || !childName) return null;

	dirOut.connected_to = [parentIn.id];
	parentIn.depends_on = [dirOut.id];
	childName.default_value = convertJsonToUint8Array(path);

	// User storage has an app-wide root and a per-node one; a file the browser listed is
	// always in the app-wide one.
	const nodeScope = findPin(dir, "node_scope");
	if (nodeScope) nodeScope.default_value = convertJsonToUint8Array(false);

	return { nodes: [dir, child] };
}
