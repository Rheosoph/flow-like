import * as React from "react";

const MOBILE_BREAKPOINT = 768;

export function useIsMobile() {
	// Initialise synchronously so the first client paint already reflects the
	// device width (drives the sidebar Sheet-vs-fixed branch and the mobile
	// bottom nav) instead of flashing the desktop layout for a frame.
	const [isMobile, setIsMobile] = React.useState<boolean>(() =>
		typeof window !== "undefined"
			? window.matchMedia(`(max-width: ${MOBILE_BREAKPOINT - 1}px)`).matches
			: false,
	);

	React.useEffect(() => {
		const mql = window.matchMedia(`(max-width: ${MOBILE_BREAKPOINT - 1}px)`);
		const onChange = () => {
			setIsMobile(window.innerWidth < MOBILE_BREAKPOINT);
		};
		mql.addEventListener("change", onChange);
		setIsMobile(window.innerWidth < MOBILE_BREAKPOINT);
		return () => mql.removeEventListener("change", onChange);
	}, []);

	return isMobile;
}
