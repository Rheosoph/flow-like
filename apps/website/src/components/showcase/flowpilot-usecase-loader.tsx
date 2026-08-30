"use client";

import { lazy, Suspense, useEffect, useRef, useState } from "react";

const InteractiveFlowPilot = lazy(() => import("./flowpilot-usecase"));

export default function FlowPilotUseCaseLoader({
	className,
}: Readonly<{ className?: string }>) {
	const rootRef = useRef<HTMLDivElement | null>(null);
	const [shouldLoad, setShouldLoad] = useState(false);

	useEffect(() => {
		const root = rootRef.current;
		if (!root || typeof IntersectionObserver === "undefined") {
			setShouldLoad(true);
			return;
		}

		const observer = new IntersectionObserver(
			(entries) => {
				if (!entries.some((entry) => entry.isIntersecting)) return;
				observer.disconnect();
				setShouldLoad(true);
			},
			{ rootMargin: "120px 0px", threshold: 0.05 },
		);
		observer.observe(root);
		return () => observer.disconnect();
	}, []);

	return (
		<div ref={rootRef} className={className}>
			{shouldLoad && (
				<Suspense fallback={null}>
					<InteractiveFlowPilot className="h-full" />
				</Suspense>
			)}
		</div>
	);
}
