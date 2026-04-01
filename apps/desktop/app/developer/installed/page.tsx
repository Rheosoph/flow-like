"use client";

import { redirect } from "next/navigation";

export default function DeveloperInstalledPage() {
	redirect("/settings/registry/installed");
}
