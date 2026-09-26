"use client";

import { redirect } from "next/navigation";

export default function LibraryPackagesPublishPage() {
	redirect("/store/packages?tab=mine");
}
