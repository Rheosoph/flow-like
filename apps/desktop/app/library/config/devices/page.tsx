"use client";

import { DevicesPage } from "@flow-like/flow-like-ui/components/settings/devices/devices-page";
import { useSearchParams } from "next/navigation";

export default function Page() {
	const projectId = useSearchParams().get("id");
	return projectId ? <DevicesPage projectId={projectId} /> : null;
}
