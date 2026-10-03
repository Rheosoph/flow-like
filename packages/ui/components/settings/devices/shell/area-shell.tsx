"use client";

import { type ReactNode, type Ref, useEffect, useRef } from "react";
import { cx } from "../primitives/tone";

/** `page`: the account host draws the frame. `card`: the app config card already has one. */
export type AreaFrame = "page" | "card";

export interface AreaShellProps {
	frame: AreaFrame;
	topbar: ReactNode;
	/** The device rail: a docked column, or an overlay that renders nothing in flow. */
	rail?: ReactNode;
	/** The activity tray; it positions itself over the right edge of the page. */
	tray?: ReactNode;
	statusBar?: ReactNode;
	/** Page-level notices above the screen (unresolved link, locked keys after a switch). */
	banners?: ReactNode;
	/** The page scrolls back to the top when this changes: another object, section or flow. */
	scrollKey?: string;
	/** The `@container/devices` element, for the area's width bucket. */
	rootRef?: Ref<HTMLDivElement>;
	children: ReactNode;
}

const FRAME: Record<AreaFrame, string> = {
	page: "h-full rounded-lg border border-border bg-background",
	card: "h-full",
};

/** The page scroller. */
export const AREA_MAIN_SELECTOR = "[data-area-main]";
/** A page marks its own filter field with `data-devices-filter`; `/` focuses it instead of the rail filter (SPEC §3.4). */
export const PAGE_FILTER_SELECTOR = "[data-area-main] [data-devices-filter]";

/** The app's base layer gives every `p` a 28 px line; inside the area a paragraph follows its container unless it sets its own. */
const PLAIN_PARAGRAPHS = "[:where(&_p)]:leading-[inherit]";

/**
 * SPEC §3.1 inside Flow-Like's content column (M-UI §3.3): top bar, rail, one
 * page scroller, tray and status bar, with no `position: fixed` anywhere.
 */
export function AreaShell(props: Readonly<AreaShellProps>) {
	const { frame, topbar, rail, tray, statusBar, rootRef } = props;
	return (
		<div
			ref={rootRef}
			data-shell="area"
			data-frame={frame}
			className={cx(
				"@container/devices relative flex min-h-0 w-full flex-col overflow-hidden text-foreground",
				PLAIN_PARAGRAPHS,
				FRAME[frame],
			)}
		>
			{topbar}
			<div className="relative flex min-h-0 flex-1">
				{rail}
				<AreaPage banners={props.banners} scrollKey={props.scrollKey}>
					{props.children}
				</AreaPage>
				{tray}
			</div>
			{statusBar}
		</div>
	);
}

/** The one page scroller: notices first, then the screen. */
function AreaPage({
	banners,
	scrollKey,
	children,
}: Readonly<Pick<AreaShellProps, "banners" | "scrollKey" | "children">>) {
	const main = useRef<HTMLDivElement>(null);
	// biome-ignore lint/correctness/useExhaustiveDependencies: the key is the trigger
	useEffect(() => {
		if (main.current) main.current.scrollTop = 0;
	}, [scrollKey]);
	return (
		<div
			ref={main}
			data-area-main=""
			tabIndex={-1}
			className="min-w-0 flex-1 overflow-y-auto outline-none"
		>
			<div className="mx-auto flex w-full max-w-370 flex-col gap-6 px-6 pt-5 pb-8 @max-[720px]/devices:px-4">
				{banners}
				{children}
			</div>
		</div>
	);
}
