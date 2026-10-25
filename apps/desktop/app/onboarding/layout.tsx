"use client";

import { usePathname } from "next/navigation";
import { Suspense } from "react";
import styles from "./onboarding.module.css";

export default function OnboardingLayout({
	children,
}: Readonly<{ children: React.ReactNode }>) {
	const pathname = usePathname();
	const isSetupStep =
		pathname.startsWith("/onboarding/") && pathname !== "/onboarding/";

	return (
		<div
			className={[
				styles.shell,
				isSetupStep &&
					"flex flex-col items-center justify-start md:justify-center px-4",
			]
				.filter(Boolean)
				.join(" ")}
		>
			{isSetupStep && <BackgroundFX />}
			<Suspense fallback={null}>{children}</Suspense>
		</div>
	);
}

function BackgroundFX() {
	return (
		<div
			aria-hidden="true"
			className="pointer-events-none fixed inset-0 z-0 overflow-hidden"
		>
			<div className="absolute inset-0 bg-gradient-to-b from-primary/10 via-background to-secondary/10" />
			<div className="absolute -top-24 -left-24 h-80 w-80 rounded-full bg-gradient-to-tr from-primary/30 via-primary/20 to-transparent blur-3xl motion-safe:animate-pulse" />
			<div className="absolute -bottom-32 -right-24 h-[28rem] w-[28rem] rounded-full bg-gradient-to-tr from-secondary/30 via-secondary/20 to-transparent blur-3xl motion-safe:animate-[spin_40s_linear_infinite]" />
		</div>
	);
}
