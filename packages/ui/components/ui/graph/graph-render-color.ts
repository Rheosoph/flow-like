/** Sigma's WebGL programs use ONE / ONE_MINUS_SRC_ALPHA blending. */
export function graphRgba(hex: string, alpha: number): string {
	const color =
		hex.length === 4
			? `#${hex[1]}${hex[1]}${hex[2]}${hex[2]}${hex[3]}${hex[3]}`
			: hex;
	const opacity = Math.max(0, Math.min(1, alpha));
	const channel = (offset: number) =>
		Math.round(Number.parseInt(color.slice(offset, offset + 2), 16) * opacity);
	return `rgba(${channel(1)},${channel(3)},${channel(5)},${opacity})`;
}
