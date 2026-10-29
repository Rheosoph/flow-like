const HEX_COLOR_PATTERN = /^#[0-9a-f]{6}$/i;

/** `<input type="color">` only takes #rrggbb; tokens, var() and rgb() stay editable as text. */
export function colorInputValue(value: string | undefined): string {
	return value && HEX_COLOR_PATTERN.test(value) ? value : "#ffffff";
}
