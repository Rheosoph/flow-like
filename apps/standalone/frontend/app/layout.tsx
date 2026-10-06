import "@flow-like/flow-like-ui/global.css";
import "./standalone.css";
import type { Metadata } from "next";
import { Source_Serif_4 } from "next/font/google";

const reading = Source_Serif_4({
	axes: ["opsz"],
	variable: "--font-reading",
	preload: false,
});

export const metadata: Metadata = {
	title: "Flow-Like service",
	robots: { index: false, follow: false },
};

export default function Layout({ children }: { children: React.ReactNode }) {
	return (
		<html lang="en" className={reading.variable} suppressHydrationWarning>
			<body>{children}</body>
		</html>
	);
}
