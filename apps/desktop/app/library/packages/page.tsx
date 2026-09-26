"use client";

import { redirect } from "next/navigation";

export default function LibraryPackagesPage() {
	redirect("/store/packages?tab=library");
}
