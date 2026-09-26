export interface LocalNodeRef {
	name: string;
	friendly_name?: string;
	description?: string;
}

export interface RemoteNodeRef {
	name: string;
	friendlyName?: string;
	description?: string;
}

export interface DiffedNode {
	name: string;
	label: string;
}

export interface NodeDiff {
	added: DiffedNode[];
	removed: DiffedNode[];
	/** Same node name, different description. */
	changed: DiffedNode[];
	unchanged: number;
}

function normalized(text: string | undefined): string {
	return (text ?? "").trim().replace(/\s+/g, " ");
}

/** Node-level changes of a checkout against the registry's live version, matched by node name. */
export function diffNodes(
	local: readonly LocalNodeRef[],
	remote: readonly RemoteNodeRef[],
): NodeDiff {
	const remoteByName = new Map(remote.map((node) => [node.name, node]));
	const localNames = new Set(local.map((node) => node.name));
	const diff: NodeDiff = { added: [], removed: [], changed: [], unchanged: 0 };

	for (const node of local) {
		const label = node.friendly_name || node.name;
		const published = remoteByName.get(node.name);
		if (!published) diff.added.push({ name: node.name, label });
		else if (normalized(published.description) !== normalized(node.description))
			diff.changed.push({ name: node.name, label });
		else diff.unchanged += 1;
	}
	for (const node of remote) {
		if (!localNames.has(node.name))
			diff.removed.push({
				name: node.name,
				label: node.friendlyName || node.name,
			});
	}
	return diff;
}

export function hasNodeChanges(diff: NodeDiff): boolean {
	return (
		diff.added.length > 0 || diff.removed.length > 0 || diff.changed.length > 0
	);
}
