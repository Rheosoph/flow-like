"use client";

import { redirect } from "next/navigation";

export default function RegistryInstalledPage() {
	redirect("/store/packages?tab=library");
}
