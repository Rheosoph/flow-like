"use client";

import {
	type ReactNode,
	useCallback,
	useLayoutEffect,
	useMemo,
	useState,
} from "react";
import { cn } from "../../lib/utils";
import { LoadingScreen } from "../ui/loading-screen";
import { PageInterface, type PageInterfaceProps } from "./page-interface";
import { PageLoadIndicator, PageLoadStatus } from "./page-load-indicator";
import { PageTransitionContext } from "./page-transition-context";

type PageTarget = {
	readonly kind: "page";
	readonly key: string;
	readonly props: PageInterfaceProps;
};

export type PageView =
	| { readonly kind: "loading" }
	| PageTarget
	| { readonly kind: "content"; readonly node: ReactNode };

interface ShownPage {
	readonly scope: string;
	readonly key: string;
	readonly props: PageInterfaceProps;
}

interface PageLayer {
	readonly key: string;
	readonly props: PageInterfaceProps;
	readonly offscreen: boolean;
}

type HoldPage = (key: string, holding: boolean) => void;

const NO_HOLDS: ReadonlySet<string> = new Set();
const LAYER_CLASS =
	"flex flex-col grow h-full w-full max-h-full overflow-hidden";

function pageTarget(view: PageView) {
	return view.kind === "page" ? view : null;
}

function shownInScope(shown: ShownPage | null, scope: string) {
	return shown !== null && shown.scope === scope ? shown : null;
}

function isSettled(
	key: string | undefined,
	settledKey: string | null,
	holds: ReadonlySet<string>,
) {
	return key !== undefined && key === settledKey && !holds.has(key);
}

/** The page on screen after this view: the incoming page once it can show content. */
function resolveShownPage(
	previous: ShownPage | null,
	view: PageView,
	scope: string,
	settled: boolean,
) {
	if (view.kind !== "page") return view.kind === "loading" ? previous : null;
	if (!previous || previous.key !== view.key)
		return settled ? { scope, key: view.key, props: view.props } : previous;
	return previous.props === view.props
		? previous
		: { ...previous, props: view.props };
}

function transitionLayers(
	shown: ShownPage | null,
	incoming: PageTarget | null,
) {
	const layers: PageLayer[] = [];
	if (shown) {
		layers.push({ key: shown.key, props: shown.props, offscreen: false });
	}
	if (incoming && incoming.key !== shown?.key) {
		layers.push({
			key: incoming.key,
			props: incoming.props,
			offscreen: shown !== null,
		});
	}
	return layers;
}

function withHold(holds: ReadonlySet<string>, key: string, holding: boolean) {
	if (holds.has(key) === holding) return holds;
	const updated = new Set(holds);
	if (holding) updated.add(key);
	else updated.delete(key);
	return updated;
}

function PageTransitionLayer({
	layer,
	onHold,
}: Readonly<{ layer: PageLayer; onHold: HoldPage }>) {
	const { key, offscreen } = layer;
	const hold = useCallback(
		(holding: boolean) => onHold(key, holding),
		[key, onHold],
	);
	const slot = useMemo(() => ({ offscreen, hold }), [offscreen, hold]);
	return (
		<div
			aria-hidden={offscreen || undefined}
			inert={offscreen || undefined}
			className={cn(
				LAYER_CLASS,
				offscreen && "pointer-events-none invisible absolute inset-0",
			)}
		>
			<PageTransitionContext.Provider value={slot}>
				<PageInterface {...layer.props} />
			</PageTransitionContext.Provider>
		</div>
	);
}

function PageTransitionStack({
	shown,
	incoming,
	onHold,
}: Readonly<{
	shown: ShownPage | null;
	incoming: PageTarget | null;
	onHold: HoldPage;
}>) {
	const navigating = shown !== null && shown.key !== incoming?.key;
	return (
		<div className={cn("relative", LAYER_CLASS)}>
			<PageLoadStatus loading={navigating} />
			{navigating && <PageLoadIndicator />}
			{transitionLayers(shown, incoming).map((layer) => (
				<PageTransitionLayer key={layer.key} layer={layer} onHold={onHold} />
			))}
		</div>
	);
}

/**
 * Moving between pages keeps the page on screen, under a progress bar, until the next one can
 * show content — a browser holds a page the same way until its successor paints. The next page
 * mounts out of sight meanwhile, so its onLoad run starts at once. Opening an app, or leaving any
 * other interface, still shows the loading screens: there is no page to hold.
 */
export function PageTransition({
	scope,
	view,
}: Readonly<{
	/** A page is only held for a successor in the same app and account. */
	scope: string;
	view: PageView;
}>) {
	const [shown, setShown] = useState<ShownPage | null>(null);
	const [settledKey, setSettledKey] = useState<string | null>(null);
	const [holds, setHolds] = useState(NO_HOLDS);

	const incoming = pageTarget(view);
	const next = resolveShownPage(
		shownInScope(shown, scope),
		view,
		scope,
		isSettled(incoming?.key, settledKey, holds),
	);
	if (next !== shown) setShown(next);

	// Runs after the incoming page's own layout effects, so a page that asked to hold has done so.
	const incomingKey = incoming?.key ?? null;
	useLayoutEffect(() => {
		setSettledKey(incomingKey);
	}, [incomingKey]);

	const onHold = useCallback<HoldPage>((key, holding) => {
		setHolds((current) => withHold(current, key, holding));
	}, []);

	if (view.kind === "content") return <>{view.node}</>;
	if (!next && !incoming) return <LoadingScreen />;
	return (
		<PageTransitionStack shown={next} incoming={incoming} onHold={onHold} />
	);
}
