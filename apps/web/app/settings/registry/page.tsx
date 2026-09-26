"use client";

import { redirect } from "next/navigation";

export default function RegistryPage() {
	redirect("/store/packages?tab=library");
}
