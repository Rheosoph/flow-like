import { ChartBar, ChevronDown, Menu, X } from "lucide-react";
import { useEffect, useRef, useState } from "react";
import { createPortal } from "react-dom";
import { BsDiscord, BsGithub } from "react-icons/bs";
import {
	LuActivity,
	LuArrowRight,
	LuBookMarked,
	LuBookOpen,
	LuBot,
	LuBuilding2,
	LuCpu,
	LuDownload,
	LuExternalLink,
	LuFactory,
	LuFileStack,
	LuFileText,
	LuGlobe,
	LuLandmark,
	LuPackage,
	LuScale,
	LuServer,
	LuShieldCheck,
	LuZap,
} from "react-icons/lu";
import { translationsCommon } from "../i18n/locales/pages/common";

const languages = {
	en: "English",
	de: "Deutsch",
	es: "Español",
	fr: "Français",
	zh: "中文",
	ja: "日本語",
	ko: "한국어",
	pt: "Português",
	it: "Italiano",
	nl: "Nederlands",
	sv: "Svenska",
} as const;
const webAppUrl = "https://app.flow-like.com";
const studioName = "Flow-Like Studio";

const langFlags: Record<string, string> = {
	en: "🇺🇸",
	de: "🇩🇪",
	es: "🇪🇸",
	fr: "🇫🇷",
	zh: "🇨🇳",
	ja: "🇯🇵",
	ko: "🇰🇷",
	pt: "🇧🇷",
	it: "🇮🇹",
	nl: "🇳🇱",
	sv: "🇸🇪",
};

type Lang = keyof typeof languages;

function resolveLang(path: string): Lang {
	for (const l of Object.keys(languages) as Lang[]) {
		if (l !== "en" && (path.startsWith(`/${l}/`) || path === `/${l}`)) return l;
	}
	return "en";
}

// The header is server-rendered, so the first client render has to produce the
// same markup: the pathname the layout passes down wins, and reading the real
// location is only a fallback for call sites that pass nothing.
function usePathname(pathname?: string) {
	const [path, setPath] = useState(pathname ?? "/");

	useEffect(() => {
		if (pathname) return;
		setPath(window.location.pathname);
	}, [pathname]);

	return path;
}

function useTranslation(path: string) {
	const lang = resolveLang(path);
	const t = (key: string): string =>
		translationsCommon[lang]?.[key] ?? translationsCommon.en[key] ?? key;
	return { t, lang };
}

function getLocalizedPath(path: string, targetLang: Lang) {
	let rest = path;
	for (const l of Object.keys(languages)) {
		if (rest.startsWith(`/${l}/`) || rest === `/${l}`) {
			rest = rest.slice(l.length + 1) || "/";
			break;
		}
	}
	if (targetLang === "en") {
		return rest || "/";
	}
	return `/${targetLang}${rest === "/" ? "" : rest}`;
}

function localizeHref(lang: Lang, href: string): string {
	if (lang === "en" || href.startsWith("http") || href.startsWith("mailto:"))
		return href;
	return `/${lang}${href}`;
}

interface HeaderProps {
	pathname?: string;
	darkHero?: boolean;
}

interface DropdownItem {
	label: string;
	href: string;
	icon?: React.ComponentType<{ className?: string }>;
	description?: string;
	external?: boolean;
	highlight?: boolean;
}

interface SolutionsGroup {
	heading: string;
	items: DropdownItem[];
}

function useHoverMenu(delay = 80) {
	const [open, setOpen] = useState(false);
	const timeoutRef = useRef<ReturnType<typeof setTimeout> | null>(null);
	const handleMouseEnter = () => {
		if (timeoutRef.current) clearTimeout(timeoutRef.current);
		setOpen(true);
	};
	const handleMouseLeave = () => {
		timeoutRef.current = setTimeout(() => setOpen(false), delay);
	};
	return { open, setOpen, handleMouseEnter, handleMouseLeave };
}

function useGitHubStars() {
	const [stars, setStars] = useState<number | null>(null);
	useEffect(() => {
		const CACHE_KEY = "gh_stars";
		const CACHE_TTL = 3600000;
		try {
			const cached = sessionStorage.getItem(CACHE_KEY);
			if (cached) {
				const { count, ts } = JSON.parse(cached);
				if (Date.now() - ts < CACHE_TTL) {
					setStars(count);
					return;
				}
			}
		} catch {}
		fetch("https://api.github.com/repos/Rheosoph/flow-like", {
			headers: { Accept: "application/vnd.github.v3+json" },
		})
			.then((r) => (r.ok ? r.json() : Promise.reject()))
			.then((data) => {
				const count = data?.stargazers_count;
				if (typeof count === "number" && count > 0) {
					setStars(count);
					try {
						sessionStorage.setItem(
							CACHE_KEY,
							JSON.stringify({ count, ts: Date.now() }),
						);
					} catch {}
				}
			})
			.catch(() => {});
	}, []);
	return stars;
}

function DropdownLink({
	item,
	onClose,
}: { item: DropdownItem; onClose: () => void }) {
	return (
		<a
			href={item.href}
			target={item.external ? "_blank" : undefined}
			rel={item.external ? "noreferrer" : undefined}
			className={`group flex items-center gap-3 px-3 py-2.5 rounded-lg transition-all duration-200 ${
				item.highlight
					? "text-primary hover:bg-primary/10"
					: "text-foreground/80 hover:bg-muted/60 hover:text-foreground"
			}`}
			onClick={onClose}
		>
			{item.icon && (
				<div
					className={`p-1.5 rounded-md shrink-0 transition-colors duration-200 ${
						item.highlight
							? "bg-primary/10 text-primary"
							: "bg-muted/60 text-foreground/60 group-hover:bg-muted group-hover:text-foreground"
					}`}
				>
					<item.icon className="w-3.5 h-3.5" />
				</div>
			)}
			<div className="flex-1 min-w-0">
				<div className="flex items-center gap-1.5">
					<span className="font-medium text-sm leading-tight">
						{item.label}
					</span>
					{item.external && <LuExternalLink className="w-3 h-3 opacity-40" />}
				</div>
				{item.description && (
					<p className="text-xs text-muted-foreground mt-0.5 leading-snug">
						{item.description}
					</p>
				)}
			</div>
		</a>
	);
}

function NavDropdown({
	label,
	items,
}: {
	label: string;
	items: DropdownItem[];
}) {
	const { open, setOpen, handleMouseEnter, handleMouseLeave } = useHoverMenu();

	return (
		<div
			className="relative"
			onMouseEnter={handleMouseEnter}
			onMouseLeave={handleMouseLeave}
		>
			<button
				type="button"
				className="flex items-center gap-1 text-sm font-medium text-foreground/70 hover:text-foreground transition-colors duration-300 px-3 py-2"
				onClick={() => setOpen(!open)}
			>
				{label}
				<ChevronDown
					className={`w-3.5 h-3.5 transition-transform duration-300 ${open ? "rotate-180" : ""}`}
				/>
			</button>

			{open && (
				<div className="absolute top-full left-0 pt-2 z-50">
					<div className="bg-background/95 backdrop-blur-lg border border-border/50 rounded-xl shadow-xl shadow-black/10 p-2 min-w-60">
						{items.map((item) => (
							<DropdownLink
								key={item.href}
								item={item}
								onClose={() => setOpen(false)}
							/>
						))}
					</div>
				</div>
			)}
		</div>
	);
}

function NavSolutionsDropdown({ groups }: { groups: SolutionsGroup[] }) {
	const { open, setOpen, handleMouseEnter, handleMouseLeave } = useHoverMenu();

	return (
		<div
			className="relative"
			onMouseEnter={handleMouseEnter}
			onMouseLeave={handleMouseLeave}
		>
			<button
				type="button"
				className="flex items-center gap-1 text-sm font-medium text-foreground/70 hover:text-foreground transition-colors duration-300 px-3 py-2"
				onClick={() => setOpen(!open)}
			>
				Solutions
				<ChevronDown
					className={`w-3.5 h-3.5 transition-transform duration-300 ${open ? "rotate-180" : ""}`}
				/>
			</button>

			{open && (
				<div className="absolute top-full left-0 pt-2 z-50">
					<div className="bg-background/95 backdrop-blur-lg border border-border/50 rounded-xl shadow-xl shadow-black/15 overflow-hidden min-w-160">
						<div className="grid grid-cols-3">
							{groups.map((group, i) => (
								<div
									key={group.heading}
									className={`p-3 ${i < groups.length - 1 ? "border-r border-border/30" : ""}`}
								>
									<p className="text-[10px] font-semibold uppercase tracking-widest text-muted-foreground px-3 pb-2 pt-1">
										{group.heading}
									</p>
									<div className="space-y-0.5">
										{group.items.map((item) => (
											<DropdownLink
												key={item.href}
												item={item}
												onClose={() => setOpen(false)}
											/>
										))}
									</div>
								</div>
							))}
						</div>
					</div>
				</div>
			)}
		</div>
	);
}

function LanguageSelector({
	currentLang,
	path,
}: { currentLang: Lang; path: string }) {
	const { open, setOpen, handleMouseEnter, handleMouseLeave } = useHoverMenu();

	return (
		<div
			className="relative"
			onMouseEnter={handleMouseEnter}
			onMouseLeave={handleMouseLeave}
		>
			<button
				type="button"
				onClick={() => setOpen(!open)}
				className="flex items-center gap-1.5 px-2 py-1.5 rounded-lg text-sm text-foreground/70 hover:text-foreground hover:bg-muted/50 transition-all duration-300"
				aria-label="Select language"
			>
				<span className="text-base leading-none">{langFlags[currentLang]}</span>
				<span className="uppercase text-xs font-medium">{currentLang}</span>
				<ChevronDown
					className={`w-3 h-3 transition-transform duration-300 ${open ? "rotate-180" : ""}`}
				/>
			</button>

			{open && (
				<div className="absolute top-full right-0 pt-2 z-50">
					<div className="bg-background/95 backdrop-blur-lg border border-border/50 rounded-xl shadow-xl shadow-black/10 p-2 min-w-45 max-h-80 overflow-y-auto">
						{(Object.entries(languages) as [Lang, string][]).map(
							([code, name]) => (
								<a
									key={code}
									href={getLocalizedPath(path, code)}
									className={`flex items-center gap-2.5 px-3 py-2 rounded-lg text-sm transition-all duration-300 ${
										code === currentLang
											? "bg-primary/10 text-primary font-medium"
											: "text-foreground/70 hover:bg-muted/50 hover:text-foreground"
									}`}
									onClick={() => setOpen(false)}
								>
									<span className="text-lg">{langFlags[code]}</span>
									<span>{name}</span>
								</a>
							),
						)}
					</div>
				</div>
			)}
		</div>
	);
}

function MobileMenu({
	open,
	onClose,
	t,
	currentLang,
	path,
	stars,
}: {
	open: boolean;
	onClose: () => void;
	t: (key: string) => string;
	currentLang: Lang;
	path: string;
	stars: number | null;
}) {
	const [mounted, setMounted] = useState(false);
	const [langOpen, setLangOpen] = useState(false);
	const dialogRef = useRef<HTMLDialogElement>(null);
	const onCloseRef = useRef(onClose);
	onCloseRef.current = onClose;

	useEffect(() => setMounted(true), []);

	useEffect(() => {
		if (!open) return;
		const previouslyFocused = document.activeElement as HTMLElement | null;
		document.body.style.overflow = "hidden";
		const focusableSelector =
			'a[href], button:not([disabled]), select:not([disabled]), [tabindex]:not([tabindex="-1"])';
		const frame = window.requestAnimationFrame(() => {
			dialogRef.current
				?.querySelector<HTMLElement>("[data-mobile-menu-close]")
				?.focus();
		});
		const handleKeyDown = (event: KeyboardEvent) => {
			if (event.key === "Escape") {
				event.preventDefault();
				onCloseRef.current();
				return;
			}
			if (event.key !== "Tab") return;
			const focusable = Array.from(
				dialogRef.current?.querySelectorAll<HTMLElement>(focusableSelector) ??
					[],
			);
			if (!focusable.length) return;
			const first = focusable[0];
			const last = focusable[focusable.length - 1];
			if (event.shiftKey && document.activeElement === first) {
				event.preventDefault();
				last?.focus();
			} else if (!event.shiftKey && document.activeElement === last) {
				event.preventDefault();
				first?.focus();
			}
		};
		document.addEventListener("keydown", handleKeyDown);
		return () => {
			window.cancelAnimationFrame(frame);
			document.removeEventListener("keydown", handleKeyDown);
			document.body.style.overflow = "";
			previouslyFocused?.focus();
		};
	}, [open]);

	if (!mounted || !open) return null;

	return createPortal(
		<dialog
			ref={dialogRef}
			id="mobile-navigation"
			open
			className="fixed inset-0 z-100 m-0 h-full w-full max-h-none max-w-none border-0 bg-transparent p-0 text-inherit lg:hidden"
			aria-modal="true"
			aria-label="Navigation menu"
		>
			<button
				type="button"
				aria-label="Close menu"
				aria-hidden="true"
				tabIndex={-1}
				onClick={onClose}
				className="absolute inset-0 w-full h-full bg-black/40 backdrop-blur-sm"
			/>
			<div className="absolute top-0 right-0 w-full max-w-sm h-full bg-background/95 backdrop-blur-lg border-l border-border/50 shadow-2xl overflow-y-auto">
				<div className="flex items-center justify-between p-4 border-b border-border/30 sticky top-0 bg-background/95 backdrop-blur-lg z-10">
					<a href="/" className="flex items-center gap-2" onClick={onClose}>
						<img alt="logo" src="/icon.webp" className="h-8 w-8" />
						<span className="font-semibold text-lg">Flow Like</span>
					</a>
					<button
						type="button"
						data-mobile-menu-close
						onClick={onClose}
						className="p-2 rounded-lg hover:bg-muted/50 transition-colors duration-300"
						aria-label="Close menu"
					>
						<X className="w-5 h-5" />
					</button>
				</div>

				<nav className="p-4 space-y-6">
					{/* Solutions Section */}
					<div>
						<p className="text-xs text-muted-foreground uppercase tracking-wider mb-3 px-3 font-medium">
							Solutions
						</p>
						<div className="space-y-1">
							<p className="text-[10px] font-semibold uppercase tracking-widest text-muted-foreground px-3 pt-3 pb-1">
								By Role
							</p>
							<MobileNavItem
								href="/developers"
								icon={LuBookOpen}
								label="Developers"
								onClick={onClose}
							/>
							<MobileNavItem
								href="/pitch"
								icon={LuBuilding2}
								label="CIOs & CTOs"
								onClick={onClose}
							/>
							<p className="text-[10px] font-semibold uppercase tracking-widest text-muted-foreground px-3 pt-3 pb-1">
								By Use Case
							</p>
							<MobileNavItem
								href="/modern-bi"
								icon={ChartBar}
								label="Business Intelligence"
								onClick={onClose}
							/>
							<MobileNavItem
								href="/industries/ai-agents"
								icon={LuBot}
								label="AI Agent Workflows"
								onClick={onClose}
							/>
							<MobileNavItem
								href="/use-cases/process-automation"
								icon={LuActivity}
								label="Process Automation"
								onClick={onClose}
							/>
							<MobileNavItem
								href="/use-cases/iot"
								icon={LuCpu}
								label="IoT & Sensor Data"
								onClick={onClose}
							/>
							<p className="text-[10px] font-semibold uppercase tracking-widest text-muted-foreground px-3 pt-3 pb-1">
								By Industry
							</p>
							<MobileNavItem
								href="/industries/shopfloor"
								icon={LuFactory}
								label="Manufacturing"
								onClick={onClose}
							/>
							<MobileNavItem
								href="/industries/finance"
								icon={LuLandmark}
								label="Finance & Banking"
								onClick={onClose}
							/>
							<MobileNavItem
								href="/industries/office"
								icon={LuFileStack}
								label="Professional Services"
								onClick={onClose}
							/>
							<MobileNavItem
								href="/industries/gov-defense"
								icon={LuShieldCheck}
								label="Gov & Defense"
								onClick={onClose}
							/>
						</div>
					</div>

					{/* Resources Section */}
					<div>
						<p className="text-xs text-muted-foreground uppercase tracking-wider mb-3 px-3 font-medium">
							Resources
						</p>
						<div className="space-y-1">
							<MobileNavItem
								href="https://docs.flow-like.com"
								icon={LuBookMarked}
								label={t("header.docs")}
								external
								onClick={onClose}
							/>
							<MobileNavItem
								href="https://docs.flow-like.com/start/getting-started"
								icon={LuBookOpen}
								label="Getting Started"
								external
								onClick={onClose}
							/>
							<MobileNavItem
								href="https://docs.flow-like.com/self-hosting"
								icon={LuServer}
								label="Self-Hosting"
								external
								onClick={onClose}
							/>
							<MobileNavItem
								href="/store/"
								icon={LuPackage}
								label={t("header.store")}
								onClick={onClose}
							/>
							<MobileNavItem
								href="/blog/"
								icon={LuFileText}
								label={t("header.blog")}
								onClick={onClose}
							/>
							<MobileNavItem
								href="/compare"
								icon={LuScale}
								label="Compare"
								onClick={onClose}
							/>
							<MobileNavItem
								href="/pricing"
								icon={LuZap}
								label="Pricing"
								onClick={onClose}
							/>
						</div>
					</div>

					{/* Community Section */}
					<div>
						<p className="text-xs text-muted-foreground uppercase tracking-wider mb-3 px-3 font-medium">
							Community
						</p>
						<div className="flex gap-2 px-3">
							<a
								href="https://github.com/Rheosoph/flow-like"
								target="_blank"
								rel="noreferrer"
								className="flex-1 flex items-center justify-center gap-2 py-2.5 rounded-lg bg-muted/50 hover:bg-muted transition-colors duration-300"
							>
								<BsGithub className="w-4 h-4" />
								<span className="text-sm">GitHub</span>
								{stars !== null && (
									<span className="inline-flex items-center gap-1 px-1.5 py-0.5 rounded-full bg-amber-500/10 border border-amber-500/20 text-[11px] font-semibold tabular-nums text-amber-500/80">
										<svg
											className="w-3 h-3 text-amber-400"
											viewBox="0 0 24 24"
											fill="currentColor"
											aria-hidden="true"
										>
											<path d="M12 2l3.09 6.26L22 9.27l-5 4.87 1.18 6.88L12 17.77l-6.18 3.25L7 14.14 2 9.27l6.91-1.01L12 2z" />
										</svg>
										{stars >= 1000 ? `${(stars / 1000).toFixed(1)}k` : stars}
									</span>
								)}
							</a>
							<a
								href="https://discord.com/invite/mdBA9kMjFJ/"
								target="_blank"
								rel="noreferrer"
								className="flex-1 flex items-center justify-center gap-2 py-2.5 rounded-lg bg-muted/50 hover:bg-muted transition-colors duration-300"
							>
								<BsDiscord className="w-4 h-4" />
								<span className="text-sm">Discord</span>
							</a>
						</div>
					</div>

					{/* Language Section */}
					<div>
						<p className="text-xs text-muted-foreground uppercase tracking-wider mb-3 px-3 font-medium">
							Language
						</p>
						<button
							type="button"
							onClick={() => setLangOpen(!langOpen)}
							className="w-full flex items-center justify-between px-3 py-3 rounded-lg hover:bg-muted/50 transition-colors duration-300"
						>
							<div className="flex items-center gap-3">
								<LuGlobe className="w-5 h-5" />
								<span className="font-medium">
									{langFlags[currentLang]} {languages[currentLang]}
								</span>
							</div>
							<ChevronDown
								className={`w-4 h-4 transition-transform duration-300 ${langOpen ? "rotate-180" : ""}`}
							/>
						</button>
						{langOpen && (
							<div className="mt-2 grid grid-cols-2 gap-1 px-3">
								{(Object.entries(languages) as [Lang, string][]).map(
									([code, name]) => (
										<a
											key={code}
											href={getLocalizedPath(path, code)}
											className={`flex items-center gap-2 px-3 py-2 rounded-lg text-sm transition-all duration-300 ${
												code === currentLang
													? "bg-primary/10 text-primary font-medium"
													: "text-foreground/70 hover:bg-muted/50"
											}`}
											onClick={onClose}
										>
											<span className="text-lg">{langFlags[code]}</span>
											<span className="truncate">{name}</span>
										</a>
									),
								)}
							</div>
						)}
					</div>
				</nav>

				<div className="sticky bottom-0 p-4 border-t border-border/30 bg-background/95 backdrop-blur-lg">
					<a
						href={webAppUrl}
						target="_blank"
						rel="noreferrer"
						onClick={onClose}
						className="w-full mb-2 group flex items-center justify-center gap-2 py-2.5 px-4 rounded-lg border border-border/70 bg-background text-foreground font-medium hover:bg-muted/40 transition-colors duration-300"
					>
						<LuExternalLink className="w-4 h-4" />
						Open Web App
					</a>
					<a
						href="/download"
						onClick={onClose}
						className="w-full group flex items-center justify-center gap-2 py-2.5 px-4 rounded-lg bg-primary text-primary-foreground font-medium hover:bg-primary/90 transition-colors duration-300"
					>
						<LuDownload className="w-4 h-4" />
						{t("header.download")} Studio
						<LuArrowRight className="w-4 h-4 ml-auto transition-transform duration-300 group-hover:translate-x-1" />
					</a>
				</div>
			</div>
		</dialog>,
		document.body,
	);
}

function MobileNavItem({
	href,
	icon: Icon,
	label,
	highlight,
	external,
	onClick,
}: {
	href: string;
	icon: React.ComponentType<{ className?: string }>;
	label: string;
	highlight?: boolean;
	external?: boolean;
	onClick: () => void;
}) {
	return (
		<a
			href={href}
			target={external ? "_blank" : undefined}
			rel={external ? "noreferrer" : undefined}
			onClick={onClick}
			className={`flex items-center gap-3 px-3 py-2.5 rounded-lg transition-all ${
				highlight
					? "text-primary bg-primary/5 hover:bg-primary/10"
					: "text-foreground/80 hover:bg-muted/50"
			}`}
		>
			<Icon className="w-5 h-5" />
			<span className="font-medium">{label}</span>
			{external && (
				<LuExternalLink className="w-3.5 h-3.5 ml-auto opacity-40" />
			)}
		</a>
	);
}

export function Header({ pathname, darkHero = false }: HeaderProps) {
	const path = usePathname(pathname);
	const { t, lang } = useTranslation(path);
	const [mobileMenuOpen, setMobileMenuOpen] = useState(false);
	const [scrolled, setScrolled] = useState(false);
	// Pages with a dark hero (e.g. the landing) mark it with [data-dark-hero];
	// while the transparent header sits over it, invert the header tokens. The
	// layout declares it up front so the server-rendered header already has the
	// right palette; the DOM probe covers pages that only mark the hero.
	const [heroDark, setHeroDark] = useState(darkHero);
	const stars = useGitHubStars();

	useEffect(() => {
		const handleScroll = () => setScrolled(window.scrollY > 20);
		handleScroll();
		window.addEventListener("scroll", handleScroll, { passive: true });
		return () => window.removeEventListener("scroll", handleScroll);
	}, []);

	useEffect(() => {
		if (darkHero) return;
		setHeroDark(!!document.querySelector("[data-dark-hero]"));
	}, [darkHero]);

	const overDarkHero = heroDark && !scrolled;

	const solutionsGroups: SolutionsGroup[] = [
		{
			heading: "By Role",
			items: [
				{
					label: "Developers",
					href: "/developers",
					icon: LuBookOpen,
					description: "Custom nodes, SDKs & WASM plugins",
				},
				{
					label: "CIOs & CTOs",
					href: "/pitch",
					icon: LuBuilding2,
					description: "Executive overview & ROI case",
				},
			],
		},
		{
			heading: "By Use Case",
			items: [
				{
					label: "Business Intelligence",
					href: "/modern-bi",
					icon: ChartBar,
					description: "Dashboards, reports & data pipelines",
				},
				{
					label: "AI Agent Workflows",
					href: "/industries/ai-agents",
					icon: LuBot,
					description: "LLMs, RAG, tool-use & multi-agent",
				},
				{
					label: "Process Automation",
					href: "/use-cases/process-automation",
					icon: LuActivity,
					description: "Forms, approvals & back-office flows",
				},
				{
					label: "IoT & Sensor Data",
					href: "/use-cases/iot",
					icon: LuCpu,
					description: "PLCs, SCADA & real-time streams",
				},
			],
		},
		{
			heading: "By Industry",
			items: [
				{
					label: "Manufacturing",
					href: "/industries/shopfloor",
					icon: LuFactory,
					description: "Shopfloor, machines & OT systems",
				},
				{
					label: "Finance & Banking",
					href: "/industries/finance",
					icon: LuLandmark,
					description: "Reconciliation, risk & compliance",
				},
				{
					label: "Professional Services",
					href: "/industries/office",
					icon: LuFileStack,
					description: "Legal, consulting & document-heavy ops",
				},
				{
					label: "Gov & Defense",
					href: "/industries/gov-defense",
					icon: LuShieldCheck,
					description: "Air-gapped, sovereign & classified",
				},
			],
		},
	];

	const resourceItems: DropdownItem[] = [
		{
			label: t("header.docs"),
			href: "https://docs.flow-like.com",
			icon: LuBookMarked,
			external: true,
		},
		{
			label: "Getting Started",
			href: "https://docs.flow-like.com/start/getting-started",
			icon: LuBookOpen,
			external: true,
		},
		{
			label: "Self-Hosting",
			href: "https://docs.flow-like.com/self-hosting",
			icon: LuServer,
			external: true,
		},
		{
			label: t("header.integrations"),
			href: localizeHref(lang, "/integrations"),
			icon: LuGlobe,
		},
		{
			label: t("header.store"),
			href: "/store/",
			icon: LuPackage,
		},
		{
			label: t("header.security"),
			href: localizeHref(lang, "/security"),
			icon: LuShieldCheck,
		},
		{
			label: t("header.blog"),
			href: "/blog/",
			icon: LuFileText,
		},
		{
			label: "Compare",
			href: "/compare",
			icon: LuScale,
		},
	];

	return (
		<>
			<header
				className={`w-full fixed top-0 left-0 right-0 z-50 transition-all duration-300 ${
					scrolled
						? "h-14 bg-background/60 backdrop-blur-xl border-b border-border/20"
						: "h-16 bg-transparent"
				} ${overDarkHero ? "dark" : ""}`}
			>
				<div className="max-w-7xl mx-auto h-full px-4 flex items-center justify-between">
					{/* Logo */}
					<a href="/" className="flex items-center gap-2.5 group shrink-0">
						<img
							alt="Flow Like logo"
							src="/icon.webp"
							className={`transition-all duration-300 ${scrolled ? "h-8 w-8" : "h-10 w-10"}`}
						/>
						<span className="font-semibold text-lg tracking-tight text-foreground group-hover:text-primary transition-colors duration-300">
							Flow Like
						</span>
					</a>

					{/* Desktop Navigation */}
					<nav className="hidden lg:flex items-center gap-1">
						<NavSolutionsDropdown groups={solutionsGroups} />
						<NavDropdown label="Resources" items={resourceItems} />
						<a
							href="/pricing"
							className="px-3 py-2 text-sm font-medium text-foreground/70 hover:text-foreground transition-colors duration-300"
						>
							Pricing
						</a>
						<div className="flex items-center border-l border-border/40 ml-1 pl-1 gap-0.5">
							<a
								href="https://github.com/Rheosoph/flow-like"
								target="_blank"
								rel="noreferrer"
								aria-label="GitHub"
								className="p-2 rounded-lg text-foreground/60 hover:text-foreground hover:bg-muted/50 transition-all duration-300 flex items-center gap-1.5"
							>
								<BsGithub className="w-4 h-4" />
							</a>
							<a
								href="https://discord.com/invite/mdBA9kMjFJ/"
								target="_blank"
								rel="noreferrer"
								aria-label="Discord"
								className="p-2 rounded-lg text-foreground/60 hover:text-foreground hover:bg-muted/50 transition-all duration-300"
							>
								<BsDiscord className="w-4 h-4" />
							</a>
						</div>
					</nav>

					{/* Desktop Actions */}
					<div className="hidden lg:flex items-center gap-2">
						<LanguageSelector currentLang={lang} path={path} />
						<a
							href={webAppUrl}
							target="_blank"
							rel="noreferrer"
							className="flex items-center gap-1.5 py-1.5 px-3 rounded-lg border border-border/70 text-sm font-medium text-foreground/70 hover:text-foreground hover:bg-muted/40 transition-colors duration-300"
							title="Open Web App"
						>
							<LuExternalLink className="w-3.5 h-3.5" />
							Web App
						</a>
						<a
							href="/download"
							className="flex items-center gap-2 py-1.5 px-3 rounded-lg bg-primary text-primary-foreground text-sm font-medium hover:bg-primary/90 transition-colors duration-300"
							title={studioName}
						>
							<LuDownload className="w-4 h-4" />
							{t("header.download")} Studio
						</a>
					</div>

					{/* Mobile Menu Button */}
					<button
						type="button"
						onClick={() => setMobileMenuOpen(true)}
						className="lg:hidden p-2 rounded-lg text-foreground hover:bg-muted/50 transition-colors duration-300"
						aria-label="Open menu"
						aria-controls="mobile-navigation"
						aria-expanded={mobileMenuOpen}
					>
						<Menu className="w-5 h-5" />
					</button>
				</div>
			</header>

			<MobileMenu
				open={mobileMenuOpen}
				onClose={() => setMobileMenuOpen(false)}
				t={t}
				currentLang={lang}
				path={path}
				stars={stars}
			/>
		</>
	);
}
