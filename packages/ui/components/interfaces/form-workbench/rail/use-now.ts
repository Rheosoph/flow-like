"use client";

import { useEffect, useState } from "react";

/** The current time, ticking once a second only while `active` (a live run's clock). */
export function useNow(active: boolean, intervalMs = 1000) {
	const [now, setNow] = useState(() => Date.now());

	useEffect(() => {
		if (!active) return;
		setNow(Date.now());
		const timer = setInterval(() => setNow(Date.now()), intervalMs);
		return () => clearInterval(timer);
	}, [active, intervalMs]);

	return now;
}
