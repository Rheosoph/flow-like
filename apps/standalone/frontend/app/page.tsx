"use client";
import dynamic from "next/dynamic";

const Service = dynamic(() => import("../client"), { ssr: false });
export default function Page() {
	return <Service />;
}
