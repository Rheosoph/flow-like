"use client";

import {
	DevicesArea,
	DevicesAreaSkeleton,
} from "@flow-like/flow-like-ui/components/settings/devices";
import { Suspense } from "react";

export default function Page() {
	return (
		<Suspense fallback={<DevicesAreaSkeleton scope="account" />}>
			<DevicesArea scope="account" />
		</Suspense>
	);
}
