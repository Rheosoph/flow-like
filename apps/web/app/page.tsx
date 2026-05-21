"use client";
import { HomeSwimlanes, Skeleton, useBackend } from "@flow-like/flow-like-ui";
import { useRouter } from "next/navigation";
import { useEffect, useState } from "react";

export default function Home() {
	const backend = useBackend();
	const router = useRouter();
	const [isCheckingProfiles, setIsCheckingProfiles] = useState(true);

	// For web, we'll skip the profile check for now
	useEffect(() => {
		setIsCheckingProfiles(false);
	}, []);

	if (isCheckingProfiles) {
		return (
			<div className="flex min-h-screen items-center justify-center">
				<Skeleton className="h-[400px] w-[600px]" />
			</div>
		);
	}

	return (
		<main className="flex flex-col flex-1 w-full min-h-0 overflow-hidden">
			<HomeSwimlanes />
		</main>
	);
}
