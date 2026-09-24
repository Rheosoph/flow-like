import { cn } from "../../../../lib/utils";

/** Whether a grid tile spans one row ("short") at each breakpoint of the Explore container. */
export interface TileShape {
	/** md: 6 columns. */
	mdShort: boolean;
	/** xl: 12 columns. */
	short: boolean;
}

/** Literal class lists per breakpoint and shape, so Tailwind sees every class. */
export type ResponsiveVariants = Record<
	"mdRow" | "mdColumn" | "xlRow" | "xlColumn",
	string
>;

export function responsive(
	{ mdShort, short }: TileShape,
	variants: ResponsiveVariants,
): string {
	return cn(
		mdShort ? variants.mdRow : variants.mdColumn,
		short ? variants.xlRow : variants.xlColumn,
	);
}
