import "@flow-like/flow-like-ui/global.css";
import "./standalone.css";
import type { Metadata } from "next";

export const metadata: Metadata = {
	title: "Flow-Like service",
	robots: { index: false, follow: false },
};

export default function Layout({ children }: { children: React.ReactNode }) {
	return (
		<html lang="en" suppressHydrationWarning>
			<body>{children}</body>
		</html>
	);
}
