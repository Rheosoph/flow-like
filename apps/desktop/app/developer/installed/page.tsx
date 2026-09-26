"use client";

import { redirect } from "next/navigation";

export default function DeveloperInstalledPage() {
	redirect("/store/packages?tab=library");
}
