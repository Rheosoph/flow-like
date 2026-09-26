import type { ProbeCheck, ProbeChecks, ProbeReport } from "./report";

function element<K extends keyof HTMLElementTagNameMap>(
	tag: K,
	className?: string,
	text?: string,
): HTMLElementTagNameMap[K] {
	const node = document.createElement(tag);
	if (className) node.className = className;
	if (text !== undefined) node.textContent = text;
	return node;
}

export function renderBanner(root: HTMLElement, text: string): void {
	root.prepend(element("p", "banner", text));
}

export class ResultsTable {
	readonly root = element("table", "results");
	private readonly body = element("tbody");
	private readonly rows = new Map<string, HTMLTableRowElement>();

	constructor() {
		const head = element("thead");
		const row = element("tr");
		for (const label of ["Check", "Status", "Expected", "Observed"]) {
			row.append(element("th", undefined, label));
		}
		head.append(row);
		this.root.append(head, this.body);
	}

	clear(): void {
		this.rows.clear();
		this.body.replaceChildren();
	}

	set(id: string, result: ProbeCheck): void {
		const row = this.rows.get(id) ?? element("tr");
		row.replaceChildren(
			element("td", "mono", id),
			element("td", `status status-${result.status}`, result.status),
			element("td", undefined, result.expected),
			element("td", undefined, result.observed),
		);
		if (!this.rows.has(id)) {
			this.rows.set(id, row);
			this.body.append(row);
		}
	}

	setAll(checks: ProbeChecks): void {
		for (const [id, result] of Object.entries(checks)) this.set(id, result);
	}
}

export function summaryText(report: ProbeReport): string {
	const { pass, fail, review, skip } = report.summary;
	const verdict = report.passed ? "No failures" : "Failures found";
	return `${verdict}: ${pass} pass, ${fail} fail, ${review} review, ${skip} skip (${report.phase}, expecting ${report.expectation})`;
}
