const TABLE_WRAPPER_CLASS = "flowbook-table-scroll";

/**
 * Keeps Markdown tables semantic while giving the web edition a dedicated,
 * keyboard-focusable container for narrow-screen horizontal scrolling.
 */
export default function rehypeFlowbookTables() {
	return (tree) => {
		wrapTables(tree);
	};
}

function wrapTables(node) {
	if (!Array.isArray(node?.children)) return;

	for (let index = 0; index < node.children.length; index += 1) {
		const child = node.children[index];

		if (child?.type === "element" && child.tagName === "table") {
			addColumnScopes(child);

			node.children[index] = {
				type: "element",
				tagName: "div",
				properties: {
					className: [TABLE_WRAPPER_CLASS],
					tabIndex: 0,
					role: "region",
					ariaLabel: getTableLabel(child),
				},
				children: [child],
			};
			continue;
		}

		wrapTables(child);
	}
}

function addColumnScopes(table) {
	for (const section of table.children ?? []) {
		if (section?.type !== "element" || section.tagName !== "thead") continue;

		visitElements(section, (element) => {
			if (element.tagName === "th") {
				element.properties = { ...element.properties, scope: "col" };
			}
		});
	}
}

function getTableLabel(table) {
	const headings = [];

	visitElements(table, (element) => {
		if (element.tagName === "th" && headings.length < 2) {
			const heading = getText(element).trim();
			if (heading) headings.push(heading);
		}
	});

	return headings.length > 0
		? `Scrollable table: ${headings.join(" and ")}`
		: "Scrollable data table";
}

function visitElements(node, visitor) {
	if (!Array.isArray(node?.children)) return;

	for (const child of node.children) {
		if (child?.type === "element") visitor(child);
		visitElements(child, visitor);
	}
}

function getText(node) {
	if (node?.type === "text") return node.value ?? "";
	if (!Array.isArray(node?.children)) return "";
	return node.children.map(getText).join("");
}
