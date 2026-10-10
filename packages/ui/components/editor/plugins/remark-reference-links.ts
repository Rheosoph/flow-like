type ReferenceNode = {
	type: string;
	identifier?: string;
	url?: string;
	title?: string | null;
	alt?: string;
	children?: ReferenceNode[];
	[key: string]: unknown;
};

const referenceKey = (identifier = "") =>
	identifier.replace(/\s+/g, " ").trim().toLowerCase();

/** Plate converts inline links, so resolve Markdown definitions before conversion. */
export function remarkReferenceLinks() {
	return (root: { type: string }) => {
		const tree = root as ReferenceNode;
		const definitions = new Map<string, ReferenceNode>();
		const collect = (node: ReferenceNode) => {
			if (node.type === "definition") {
				const key = referenceKey(node.identifier);
				if (!definitions.has(key)) definitions.set(key, node);
			}
			node.children?.forEach(collect);
		};
		collect(tree);
		const resolve = (node: ReferenceNode): ReferenceNode => {
			if (node.type === "linkReference" || node.type === "imageReference") {
				const definition = definitions.get(referenceKey(node.identifier));
				if (definition) {
					return {
						...node,
						type: node.type === "linkReference" ? "link" : "image",
						url: definition.url,
						title: definition.title,
						...(node.children ? { children: node.children.map(resolve) } : {}),
					};
				}
			}
			if (node.children) node.children = node.children.map(resolve);
			return node;
		};
		resolve(tree);
	};
}
