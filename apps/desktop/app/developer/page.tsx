"use client";

import { redirect } from "next/navigation";

export default function DeveloperPage() {
	redirect("/store/packages?tab=mine");
}
