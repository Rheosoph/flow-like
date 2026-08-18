import { ArrowRight, Github, Linkedin, Twitter, Youtube } from "lucide-react";
import { useEffect, useState } from "react";
import { translationsCommon } from "../i18n/locales/pages/common";

type Lang =
	| "en"
	| "de"
	| "es"
	| "fr"
	| "zh"
	| "ja"
	| "ko"
	| "pt"
	| "it"
	| "nl"
	| "sv";

function useTranslation() {
	const [lang, setLang] = useState<Lang>("en");

	useEffect(() => {
		if (typeof window === "undefined") return;
		const path = window.location.pathname;
		const langs: Lang[] = [
			"de",
			"es",
			"fr",
			"zh",
			"ja",
			"ko",
			"pt",
			"it",
			"nl",
			"sv",
		];
		for (const l of langs) {
			if (path.startsWith(`/${l}/`) || path === `/${l}`) {
				setLang(l);
				return;
			}
		}
		setLang("en");
	}, []);

	const t = (key: string): string => {
		return translationsCommon[lang]?.[key] ?? translationsCommon.en[key] ?? key;
	};

	return { t, lang };
}

function FooterLink({
	href,
	children,
	external,
}: { href: string; children: React.ReactNode; external?: boolean }) {
	return (
		<a
			href={href}
			className="text-muted-foreground hover:text-foreground transition-colors text-sm"
			{...(external ? { target: "_blank", rel: "noreferrer" } : {})}
		>
			{children}
		</a>
	);
}

function FooterSection({
	title,
	children,
}: { title: string; children: React.ReactNode }) {
	return (
		<div className="flex flex-col gap-3">
			<h4 className="font-semibold text-sm">{title}</h4>
			<div className="flex flex-col gap-2">{children}</div>
		</div>
	);
}

export function BlogFooter() {
	const { t, lang } = useTranslation();

	const localizeHref = (href: string) => {
		if (lang === "en" || href.startsWith("http") || href.startsWith("mailto:"))
			return href;
		return `/${lang}${href}`;
	};

	return (
		<footer className="w-full border-t border-border/40 bg-background/50 backdrop-blur-sm">
			<div className="max-w-7xl mx-auto px-4 sm:px-6 lg:px-8">
				{/* CTA Section */}
				<div className="py-8 border-b border-border/40">
					<div className="flex flex-col sm:flex-row items-center justify-between gap-4">
						<div>
							<h3 className="text-lg font-semibold">{t("footer.cta.title")}</h3>
							<p className="text-muted-foreground text-sm">
								{t("footer.cta.description")}
							</p>
						</div>
						<a
							href="/download"
							className="inline-flex items-center gap-2 bg-primary text-primary-foreground px-6 py-2.5 rounded-lg font-medium hover:bg-primary/90 transition-colors"
						>
							{t("footer.cta.button")}
							<ArrowRight className="w-4 h-4" />
						</a>
					</div>
				</div>

				{/* Main Footer Grid */}
				<div className="py-10 grid grid-cols-2 md:grid-cols-4 lg:grid-cols-5 gap-8">
					{/* Product */}
					<FooterSection title={t("footer.section.product")}>
						<FooterLink href="/download">
							{t("footer.link.download")}
						</FooterLink>
						<FooterLink href="/compare">{t("footer.link.compare")}</FooterLink>
						<FooterLink href="/modern-bi">
							{t("footer.link.modernBi")}
						</FooterLink>
						<FooterLink href={localizeHref("/integrations")}>
							{t("footer.link.integrations")}
						</FooterLink>
						<FooterLink href="/store/">{t("footer.link.store")}</FooterLink>
						<FooterLink href={localizeHref("/whitelabel")}>
							{t("footer.link.whitelabel")}
						</FooterLink>
						<FooterLink href={localizeHref("/security")}>
							{t("footer.link.security")}
						</FooterLink>
					</FooterSection>

					{/* Resources */}
					<FooterSection title={t("footer.section.resources")}>
						<FooterLink
							href="https://docs.flow-like.com/start/getting-started"
							external
						>
							{t("footer.link.gettingStarted")}
						</FooterLink>
						<FooterLink
							href="https://docs.flow-like.com/start/what-is-flow-like"
							external
						>
							{t("footer.link.whatIs")}
						</FooterLink>
						<FooterLink href="https://docs.flow-like.com/self-hosting" external>
							{t("footer.link.selfHosting")}
						</FooterLink>
						<FooterLink href="https://docs.flow-like.com/nodes" external>
							{t("footer.link.nodes")}
						</FooterLink>
					</FooterSection>

					{/* Blog */}
					<FooterSection title={t("footer.section.blog")}>
						<FooterLink href="/blog/2026-01-01-alpha-0-0-7">
							{t("footer.link.latestRelease")}
						</FooterLink>
						<FooterLink href="/blog/2025-08-09-agents-vs-automation">
							{t("footer.link.agentsVsAutomation")}
						</FooterLink>
						<FooterLink href="/blog/2025-09-13-n8n-flow-like">
							{t("footer.link.n8nComparison")}
						</FooterLink>
						<FooterLink href="/blog">{t("footer.link.allPosts")}</FooterLink>
					</FooterSection>

					{/* Company */}
					<FooterSection title={t("footer.section.company")}>
						<FooterLink href="https://github.com/Rheosoph/flow-like" external>
							{t("footer.link.github")}
						</FooterLink>
						<FooterLink
							href="https://github.com/Rheosoph/flow-like/discussions"
							external
						>
							{t("footer.link.community")}
						</FooterLink>
						<FooterLink href="mailto:contact@flow-like.com">
							{t("footer.link.contact")}
						</FooterLink>
					</FooterSection>

					{/* Legal */}
					<FooterSection title={t("footer.section.legal")}>
						<FooterLink href="/eula">{t("footer.eula")}</FooterLink>
						<FooterLink href="/privacy-policy">
							{t("footer.privacy")}
						</FooterLink>
						<FooterLink href="https://great-co.de/legal-notice" external>
							{t("footer.legal")}
						</FooterLink>
						<FooterLink href="/thirdparty">
							{t("footer.link.thirdParty")}
						</FooterLink>
						<FooterLink href="/data-deletion">
							{t("footer.dataDeletion")}
						</FooterLink>
					</FooterSection>
				</div>

				{/* Bottom Bar */}
				<div className="py-6 border-t border-border/40 flex flex-col sm:flex-row items-center justify-between gap-4">
					<div className="flex items-center gap-2">
						<img src="/favicon.svg" alt="Flow-Like" className="w-5 h-5" />
						<span className="text-sm text-muted-foreground">
							{t("footer.copyright")}
						</span>
					</div>

					{/* Social Links */}
					<div className="flex items-center gap-4">
						<a
							href="https://github.com/Rheosoph/flow-like"
							target="_blank"
							rel="noreferrer"
							className="text-muted-foreground hover:text-foreground transition-colors"
							aria-label="GitHub"
						>
							<Github className="w-5 h-5" />
						</a>
						<a
							href="https://twitter.com/flowlokeapp"
							target="_blank"
							rel="noreferrer"
							className="text-muted-foreground hover:text-foreground transition-colors"
							aria-label="Twitter"
						>
							<Twitter className="w-5 h-5" />
						</a>
						<a
							href="https://linkedin.com/company/flow-like"
							target="_blank"
							rel="noreferrer"
							className="text-muted-foreground hover:text-foreground transition-colors"
							aria-label="LinkedIn"
						>
							<Linkedin className="w-5 h-5" />
						</a>
						<a
							href="https://youtube.com/@flow-like"
							target="_blank"
							rel="noreferrer"
							className="text-muted-foreground hover:text-foreground transition-colors"
							aria-label="YouTube"
						>
							<Youtube className="w-5 h-5" />
						</a>
					</div>
				</div>
			</div>
		</footer>
	);
}
