"use client";

import { useCallback, useMemo, useState } from "react";
import type { IApp } from "../../../lib/schema/app/app";
import type { IAppShowcase } from "../../../lib/schema/app/showcase";
import {
	type ShowcaseMapNode,
	type ShowcaseNodeKind,
	type ShowcaseSectionKey,
	buildGraph,
	showcaseAccess,
	visibleSections,
} from "./model";
import { ShowcaseActivity } from "./showcase-activity";
import { ShowcaseData } from "./showcase-data";
import { ShowcaseHood } from "./showcase-hood";
import { ShowcaseMap } from "./showcase-map";
import { ShowcaseDetail, type ShowcaseSelection } from "./showcase-sheet";
import {
	ShowcasePages,
	ShowcaseTemplates,
	ShowcaseWidgets,
} from "./showcase-surfaces";
import { ShowcaseTriggers } from "./showcase-triggers";

export function StoreShowcase({
	app,
	showcase,
}: Readonly<{ app: IApp; showcase: IAppShowcase }>) {
	const [selection, setSelection] = useState<ShowcaseSelection | null>(null);
	const graph = useMemo(() => buildGraph(showcase), [showcase]);
	const sections = useMemo(
		() => visibleSections(showcase, app.app_type),
		[showcase, app.app_type],
	);
	const access = showcaseAccess(app.price);

	const open = useCallback(
		(kind: ShowcaseNodeKind, id: string) => setSelection({ kind, id }),
		[],
	);
	const openNode = useCallback(
		(n: ShowcaseMapNode) => open(n.kind, n.id),
		[open],
	);
	const openTableOrSource = useCallback(
		(kind: "table" | "source", id: string) => open(kind, id),
		[open],
	);
	const close = useCallback(() => setSelection(null), []);

	if (sections.length === 0) return null;

	const render = (key: ShowcaseSectionKey) => {
		switch (key) {
			case "map":
				return <ShowcaseMap graph={graph} onOpen={openNode} />;
			case "data":
				return <ShowcaseData showcase={showcase} onOpen={openTableOrSource} />;
			case "triggers":
				return (
					<ShowcaseTriggers
						events={showcase.events}
						access={access}
						onOpen={(id) => open("event", id)}
					/>
				);
			case "pages":
				return (
					<ShowcasePages
						pages={showcase.pages}
						onOpen={(id) => open("page", id)}
					/>
				);
			case "widgets":
				return (
					<ShowcaseWidgets
						widgets={showcase.widgets}
						onOpen={(id) => open("widget", id)}
					/>
				);
			case "templates":
				return <ShowcaseTemplates templates={showcase.templates} />;
			case "hood":
				return showcase.hood ? (
					<ShowcaseHood
						hood={showcase.hood}
						flows={showcase.flows}
						onOpenFlow={(id) => open("flow", id)}
					/>
				) : null;
			default:
				return showcase.activity ? (
					<ShowcaseActivity activity={showcase.activity} />
				) : null;
		}
	};

	return (
		<div className="space-y-12" data-store-showcase>
			{sections.map((k) => (
				<div key={k}>{render(k)}</div>
			))}
			<ShowcaseDetail
				showcase={showcase}
				graph={graph}
				access={access}
				selection={selection}
				onSelect={setSelection}
				onClose={close}
			/>
		</div>
	);
}
