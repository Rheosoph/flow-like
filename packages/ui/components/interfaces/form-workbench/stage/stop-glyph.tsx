/** The canvas's Stop square: a 12-unit rect at 6,6 of a 24 grid, filled and stroked, the same one the dock's Stop draws. */
export function StopGlyph() {
	return (
		<svg
			viewBox="0 0 24 24"
			aria-hidden="true"
			className="size-3.5 shrink-0 fill-current stroke-current"
			strokeWidth={2}
			strokeLinejoin="round"
		>
			<rect width="12" height="12" x="6" y="6" rx="1.5" />
		</svg>
	);
}
