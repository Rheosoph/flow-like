import type { Metadata, Viewport } from "next";
import "@flow-like/flow-like-ui/global.css";
import { Inter, Source_Serif_4 } from "next/font/google";
import Script from "next/script";
import { ClientProviders } from "../components/client-providers";

const inter = Inter({ subsets: ["latin"], preload: true });
const reading = Source_Serif_4({
	axes: ["opsz"],
	variable: "--font-reading",
	preload: false,
});

const siteUrl = process.env.NEXT_PUBLIC_SITE_URL || "https://app.flow-like.com";
const metadataBase = new URL(siteUrl);

export const metadata: Metadata = {
	title: {
		default: "Flow-Like Web App | Local-First Workflow Automation",
		template: "%s | Flow-Like Web App",
	},
	description:
		"Build and run type-safe, local-first workflow automation in the browser. Self-hosted, auditable, and Rust-powered.",
	keywords: [
		"workflow automation web app",
		"self-hosted workflow automation",
		"local-first workflow engine",
		"type-safe workflows",
		"rust workflow automation",
		"flow-like",
	],
	alternates: {
		canonical: "/",
	},
	authors: [{ name: "Rheosoph GmbH" }],
	creator: "Rheosoph GmbH",
	metadataBase,
	openGraph: {
		type: "website",
		locale: "en_US",
		url: siteUrl,
		siteName: "Flow-Like",
		title: "Flow-Like Web App | Local-First Workflow Automation",
		description:
			"Build and run type-safe, local-first workflow automation in the browser. Self-hosted, auditable, and Rust-powered.",
		images: [
			{
				url: "/og.png",
				width: 1600,
				height: 900,
				alt: "Flow-Like web app",
			},
		],
	},
	twitter: {
		card: "summary_large_image",
		site: "@greatco_de",
		creator: "@greatco_de",
		title: "Flow-Like Web App | Local-First Workflow Automation",
		description:
			"Build and run type-safe, local-first workflow automation in the browser. Self-hosted, auditable, and Rust-powered.",
		images: ["/og.png"],
	},
	icons: {
		icon: [
			{ url: "/favicon.svg", type: "image/svg+xml" },
			{ url: "/favicon-32x32.png", sizes: "32x32", type: "image/png" },
			{ url: "/favicon-16x16.png", sizes: "16x16", type: "image/png" },
		],
		apple: [{ url: "/apple-touch-icon.png", sizes: "180x180" }],
	},
	manifest: "/site.webmanifest",
	robots: {
		index: true,
		follow: true,
		googleBot: {
			index: true,
			follow: true,
			"max-image-preview": "large",
			"max-snippet": -1,
			"max-video-preview": -1,
		},
	},
};

export const viewport: Viewport = {
	themeColor: "#ffffff",
	width: "device-width",
	initialScale: 1,
	maximumScale: 1,
	viewportFit: "cover",
};

export default function RootLayout({
	children,
}: Readonly<{
	children: React.ReactNode;
}>) {
	return (
		<html
			lang="en"
			className={reading.variable}
			suppressHydrationWarning
			suppressContentEditableWarning
		>
			<body className={inter.className}>
				{process.env.NEXT_PUBLIC_FLOW_LIKE_RUNTIME_CONFIG === "1" && (
					<Script src="/runtime-config.js" strategy="beforeInteractive" />
				)}
				<ClientProviders>{children}</ClientProviders>
			</body>
		</html>
	);
}
