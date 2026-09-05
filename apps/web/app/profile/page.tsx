"use client";

import {
	PublicProfilePage,
	PublicProfileSkeleton,
} from "@flow-like/flow-like-ui/components/profile/public-profile-page";
import { Suspense } from "react";

export default function ProfilePage() {
	return (
		<Suspense fallback={<PublicProfileSkeleton />}>
			<PublicProfilePage />
		</Suspense>
	);
}
