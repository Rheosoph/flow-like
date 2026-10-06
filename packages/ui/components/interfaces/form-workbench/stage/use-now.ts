import { useEffect, useState } from "react";

/**
 * The current time, read at every render. While `ticking` the component renders once a second so a
 * live clock moves; otherwise nothing schedules a render (spec M5: the clock runs only while a run is live).
 */
export function useNow(ticking: boolean, intervalMs = 1000) {
	const [, setTick] = useState(0);
	useEffect(() => {
		if (!ticking) return;
		const timer = setInterval(() => setTick((tick) => tick + 1), intervalMs);
		return () => clearInterval(timer);
	}, [ticking, intervalMs]);
	return Date.now();
}
