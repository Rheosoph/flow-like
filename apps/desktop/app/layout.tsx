import "@flow-like/flow-like-ui/global.css";
import type { Viewport } from "next";
import {
	Architects_Daughter,
	DM_Sans,
	Fira_Code,
	Geist,
	Geist_Mono,
	IBM_Plex_Mono,
	IBM_Plex_Sans,
	Inter,
	JetBrains_Mono,
	Libre_Baskerville,
	Lora,
	Merriweather,
	Montserrat,
	Open_Sans,
	Outfit,
	Oxanium,
	Playfair_Display,
	Plus_Jakarta_Sans,
	Poppins,
	Roboto,
	Roboto_Mono,
	Source_Code_Pro,
	Source_Serif_4,
	Space_Grotesk,
	Space_Mono,
} from "next/font/google";
import { Providers } from "./providers";

/**
 * `viewport-fit=cover` MUST be present in the server-rendered HTML.
 *
 * WebKit resolves every `env(safe-area-inset-*)` to 0px unless the viewport meta
 * carries `viewport-fit=cover` at parse time, and it does not reliably recompute
 * them when the meta is patched later from JS (WebKit #191872 / #272779). Since
 * the native shell also disables `contentInsetAdjustmentBehavior`, a missing
 * `cover` means the web content renders full-bleed with zero insets — i.e. the
 * header lands under the Dynamic Island and the bottom nav under the home
 * indicator. This export is the only thing that puts it in the shipped HTML;
 * `viewport.test.ts` guards it.
 *
 * Requires this file to stay a Server Component — Next.js ignores `viewport`
 * exports from "use client" modules. Client providers live in ./providers.tsx.
 */
export const viewport: Viewport = {
	width: "device-width",
	initialScale: 1,
	viewportFit: "cover",
	interactiveWidget: "resizes-content",
};

const inter = Inter({ subsets: ["latin"], preload: true });
const dmSans = DM_Sans({ subsets: ["latin"], preload: true });
const firaCode = Fira_Code({ subsets: ["latin"], preload: true });
const geist = Geist({ subsets: ["latin"], preload: true });
const geistMono = Geist_Mono({ subsets: ["latin"], preload: true });
const ibmPlexMono = IBM_Plex_Mono({
	subsets: ["latin"],
	weight: ["100", "200", "300", "400", "500", "600", "700"],
	preload: true,
});
const ibmPlexSans = IBM_Plex_Sans({
	subsets: ["latin"],
	weight: ["100", "200", "300", "400", "500", "600", "700"],
	preload: true,
});
const jetBrainsMono = JetBrains_Mono({ subsets: ["latin"], preload: true });
const libreBaskerville = Libre_Baskerville({
	subsets: ["latin"],
	weight: ["400", "700"],
	preload: true,
});
const lora = Lora({ subsets: ["latin"], preload: true });
const merriweather = Merriweather({ subsets: ["latin"], preload: true });
const montserrat = Montserrat({ subsets: ["latin"], preload: true });
const openSans = Open_Sans({ subsets: ["latin"], preload: true });
const outfit = Outfit({ subsets: ["latin"], preload: true });
const oxanium = Oxanium({ subsets: ["latin"], preload: true });
const playfairDisplay = Playfair_Display({ subsets: ["latin"], preload: true });
const plusJakartaSans = Plus_Jakarta_Sans({
	subsets: ["latin"],
	preload: true,
});
const poppins = Poppins({
	subsets: ["latin"],
	weight: ["100", "200", "300", "400", "500", "600", "700", "800", "900"],
	preload: true,
});
const roboto = Roboto({
	subsets: ["latin"],
	weight: ["100", "300", "400", "500", "700", "900"],
	preload: true,
});
const robotoMono = Roboto_Mono({ subsets: ["latin"], preload: true });
const sourceCodePro = Source_Code_Pro({ subsets: ["latin"], preload: true });
const sourceSerif4 = Source_Serif_4({ subsets: ["latin"], preload: true });
const spaceGrotesk = Space_Grotesk({ subsets: ["latin"], preload: true });
const spaceMono = Space_Mono({
	subsets: ["latin"],
	weight: ["400", "700"],
	preload: true,
});
const architectsDaughter = Architects_Daughter({
	subsets: ["latin"],
	weight: ["400"],
	preload: true,
});

export default function RootLayout({
	children,
}: Readonly<{
	children: React.ReactNode;
}>) {
	return (
		<html
			lang="en"
			data-desktop-app="true"
			suppressHydrationWarning
			suppressContentEditableWarning
			className="min-h-screen"
		>
			<body className={inter.className} data-desktop-app="true">
				<Providers>{children}</Providers>
			</body>
		</html>
	);
}
