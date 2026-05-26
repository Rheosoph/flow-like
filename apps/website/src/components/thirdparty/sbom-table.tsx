import { ExternalLink, FileText, Search, X } from "lucide-react";
import { useMemo, useState } from "react";

interface License {
	name: string;
	version: string;
	authors: string | null;
	repository: string | null;
	license: string | null;
	description: string | null;
	source: "rust" | "npm";
	licenseText?: string;
}

interface CargoLicense {
	name: string;
	version: string;
	authors: string | null;
	repository: string | null;
	license: string | null;
	description: string | null;
	license_text?: string;
}

interface NpmLicenseEntry {
	licenses: string;
	repository?: string;
	publisher?: string;
	email?: string;
	path?: string;
	licenseText?: string;
}

type NpmLicenses = Record<string, NpmLicenseEntry>;

interface SbomTableProps {
	cargoLicenses: CargoLicense[];
	npmLicenses: NpmLicenses;
}

interface FeaturedLibrary {
	name: string;
	displayName: string;
	description: string;
	url: string;
	category: string;
	sideloaded?: boolean;
}

function parseNpmLicenses(npmLicenses: NpmLicenses): License[] {
	return Object.entries(npmLicenses)
		.filter(([key]) => !key.includes("flow-like") && !key.includes("@flow-like"))
		.map(([key, value]) => {
			const match = key.match(/^(.+)@(\d+\.\d+\.\d+.*)$/);
			const name = match ? match[1] : key;
			const version = match ? match[2] : "unknown";
			return {
				name,
				version,
				authors: value.publisher || null,
				repository: value.repository || null,
				license: value.licenses || null,
				description: null,
				source: "npm" as const,
				licenseText: value.licenseText,
			};
		});
}

function parseCargoLicenses(cargoLicenses: CargoLicense[]): License[] {
	return cargoLicenses.map((l) => ({
		...l,
		source: "rust" as const,
		licenseText: l.license_text,
	}));
}

const FEATURED_LIBRARIES: FeaturedLibrary[] = [
	// Sideloaded (not in dependencies)
	{
		name: "llama.cpp",
		displayName: "llama.cpp",
		description:
			"LLM inference in C/C++ - powers local AI models in Flow-Like.",
		url: "https://github.com/ggerganov/llama.cpp",
		category: "AI/ML",
		sideloaded: true,
	},
	// Rust Core
	{
		name: "tauri",
		displayName: "Tauri",
		description:
			"Build smaller, faster, and more secure desktop and mobile applications with a web frontend.",
		url: "https://tauri.app",
		category: "Framework",
	},
	{
		name: "axum",
		displayName: "Axum",
		description:
			"Ergonomic and modular web framework built with Tokio, Tower, and Hyper.",
		url: "https://github.com/tokio-rs/axum",
		category: "Web Framework",
	},
	{
		name: "tokio",
		displayName: "Tokio",
		description:
			"Asynchronous runtime for Rust providing async I/O, networking, and task scheduling.",
		url: "https://tokio.rs",
		category: "Runtime",
	},
	{
		name: "tonic",
		displayName: "Tonic",
		description:
			"A native gRPC client & server implementation with async/await support.",
		url: "https://github.com/hyperium/tonic",
		category: "gRPC",
	},
	{
		name: "tracing",
		displayName: "Tracing",
		description:
			"Application-level tracing for Rust - structured, async-aware diagnostics.",
		url: "https://tracing.rs",
		category: "Observability",
	},
	{
		name: "serde",
		displayName: "Serde",
		description:
			"Framework for serializing and deserializing Rust data structures efficiently.",
		url: "https://serde.rs",
		category: "Serialization",
	},
	{
		name: "reqwest",
		displayName: "Reqwest",
		description: "An ergonomic, batteries-included HTTP client for Rust.",
		url: "https://github.com/seanmonstar/reqwest",
		category: "HTTP Client",
	},
	// Data & Analytics
	{
		name: "lancedb",
		displayName: "LanceDB",
		description:
			"Developer-friendly, serverless vector database for AI applications.",
		url: "https://lancedb.com",
		category: "Vector DB",
	},
	{
		name: "lance",
		displayName: "Lance",
		description: "Modern columnar data format optimized for ML workloads.",
		url: "https://github.com/lancedb/lance",
		category: "Data Format",
	},
	{
		name: "datafusion",
		displayName: "DataFusion",
		description:
			"Extensible query engine written in Rust that uses Apache Arrow.",
		url: "https://datafusion.apache.org",
		category: "Query Engine",
	},
	{
		name: "polars",
		displayName: "Polars",
		description: "Lightning-fast DataFrame library for Rust and Python.",
		url: "https://pola.rs",
		category: "Data Processing",
	},
	{
		name: "arrow",
		displayName: "Apache Arrow",
		description:
			"In-memory columnar data format for efficient analytic operations.",
		url: "https://arrow.apache.org",
		category: "Data Format",
	},
	{
		name: "parquet",
		displayName: "Apache Parquet",
		description:
			"Columnar storage file format optimized for big data processing.",
		url: "https://parquet.apache.org",
		category: "Data Format",
	},
	{
		name: "sqlx",
		displayName: "SQLx",
		description:
			"Async, pure Rust SQL toolkit with compile-time checked queries.",
		url: "https://github.com/launchbadge/sqlx",
		category: "Database",
	},
	// AI/ML
	{
		name: "fastembed",
		displayName: "FastEmbed",
		description: "Fast, lightweight embedding generation for semantic search.",
		url: "https://github.com/qdrant/fastembed",
		category: "AI/ML",
	},
	{
		name: "image",
		displayName: "Image",
		description: "Encoding and decoding images in Rust.",
		url: "https://github.com/image-rs/image",
		category: "Image Processing",
	},
	// Frontend
	{
		name: "react",
		displayName: "React",
		description: "A JavaScript library for building user interfaces.",
		url: "https://react.dev",
		category: "UI Library",
	},
	{
		name: "next",
		displayName: "Next.js",
		description:
			"The React framework for production - hybrid static & server rendering.",
		url: "https://nextjs.org",
		category: "Framework",
	},
	{
		name: "tailwindcss",
		displayName: "Tailwind CSS",
		description: "A utility-first CSS framework for rapid UI development.",
		url: "https://tailwindcss.com",
		category: "Styling",
	},
	{
		name: "astro",
		displayName: "Astro",
		description: "The web framework for content-driven websites.",
		url: "https://astro.build",
		category: "Framework",
	},
];

function FeaturedLibraries({ licenses }: { licenses: License[] }) {
	const featured = FEATURED_LIBRARIES.map((lib) => {
		const found = licenses.find(
			(l) => l.name.toLowerCase() === lib.name.toLowerCase(),
		);
		return { ...lib, license: found };
	}).filter((lib) => lib.license || lib.sideloaded);

	if (featured.length === 0) return null;

	return (
		<div className="mb-12">
			<h2 className="text-2xl font-semibold mb-6">Featured Libraries</h2>
			<p className="text-muted-foreground mb-6">
				Flow-Like is built on the shoulders of these amazing open source
				projects.
			</p>
			<div className="grid grid-cols-1 md:grid-cols-2 lg:grid-cols-4 gap-4">
				{featured.map((lib) => (
					<a
						key={lib.name}
						href={lib.url}
						target="_blank"
						rel="noreferrer"
						className="group p-4 rounded-lg border border-border/50 hover:border-primary/50 hover:bg-muted/30 transition-all"
					>
						<div className="flex items-start justify-between mb-2">
							<h3 className="font-semibold group-hover:text-primary transition-colors">
								{lib.displayName}
							</h3>
							<ExternalLink className="w-4 h-4 text-muted-foreground group-hover:text-primary transition-colors" />
						</div>
						<span className="inline-block text-xs px-2 py-0.5 rounded-full bg-primary/10 text-primary mb-2">
							{lib.category}
						</span>
						<p className="text-sm text-muted-foreground line-clamp-2">
							{lib.description}
						</p>
						{lib.license ? (
							<p className="text-xs text-muted-foreground/70 mt-2">
								v{lib.license.version} · {lib.license.license || "Unknown"}
							</p>
						) : lib.sideloaded ? (
							<p className="text-xs text-muted-foreground/70 mt-2">
								<span className="text-amber-500">Sideloaded</span> · MIT
							</p>
						) : null}
					</a>
				))}
			</div>
		</div>
	);
}

function LicenseBadge({ license }: { license: string | null }) {
	if (!license) return <span className="text-muted-foreground">Unknown</span>;

	const getColor = (lic: string) => {
		const l = lic.toLowerCase();
		if (l.includes("mit")) return "bg-green-500/10 text-green-500";
		if (l.includes("apache")) return "bg-blue-500/10 text-blue-500";
		if (l.includes("bsd")) return "bg-purple-500/10 text-purple-500";
		if (l.includes("isc")) return "bg-cyan-500/10 text-cyan-500";
		if (l.includes("mpl")) return "bg-orange-500/10 text-orange-500";
		return "bg-muted text-muted-foreground";
	};

	return (
		<span className={`text-xs px-2 py-0.5 rounded-full ${getColor(license)}`}>
			{license}
		</span>
	);
}

function SourceBadge({ source }: { source: "rust" | "npm" }) {
	const styles =
		source === "rust"
			? "bg-orange-500/10 text-orange-500"
			: "bg-yellow-500/10 text-yellow-500";

	return (
		<span className={`text-xs px-2 py-0.5 rounded-full ${styles}`}>
			{source === "rust" ? "🦀" : "📦"}
		</span>
	);
}

function LicenseTextModal({
	license,
	onClose,
}: {
	license: License;
	onClose: () => void;
}) {
	return (
		<div
			className="fixed inset-0 bg-black/50 z-50 flex items-center justify-center p-4"
			onClick={onClose}
			onKeyDown={(e) => e.key === "Escape" && onClose()}
		>
			<div
				className="bg-background border border-border rounded-xl max-w-4xl w-full max-h-[80vh] flex flex-col shadow-2xl"
				onClick={(e) => e.stopPropagation()}
			>
				<div className="flex items-center justify-between p-4 border-b border-border">
					<div>
						<h3 className="text-lg font-semibold">
							{license.name}{" "}
							<span className="text-muted-foreground font-normal">
								v{license.version}
							</span>
						</h3>
						<div className="flex items-center gap-2 mt-1">
							<LicenseBadge license={license.license} />
							<SourceBadge source={license.source} />
						</div>
					</div>
					<button
						type="button"
						onClick={onClose}
						className="p-2 hover:bg-muted rounded-lg transition-colors"
					>
						<X className="w-5 h-5" />
					</button>
				</div>
				<div className="flex-1 overflow-auto p-4">
					{license.licenseText ? (
						<pre className="whitespace-pre-wrap font-mono text-sm text-muted-foreground leading-relaxed">
							{license.licenseText}
						</pre>
					) : (
						<div className="text-center py-12 text-muted-foreground">
							<FileText className="w-12 h-12 mx-auto mb-4 opacity-50" />
							<p>License text not available for this package.</p>
							{license.repository && (
								<a
									href={license.repository}
									target="_blank"
									rel="noreferrer"
									className="inline-flex items-center gap-1 text-primary hover:underline mt-2"
								>
									View on repository <ExternalLink className="w-4 h-4" />
								</a>
							)}
						</div>
					)}
				</div>
			</div>
		</div>
	);
}

export function SbomTable({ cargoLicenses, npmLicenses }: SbomTableProps) {
	const [search, setSearch] = useState("");
	const [licenseFilter, setLicenseFilter] = useState<string>("all");
	const [sourceFilter, setSourceFilter] = useState<"all" | "rust" | "npm">(
		"all",
	);
	const [selectedLicense, setSelectedLicense] = useState<License | null>(null);

	const allLicenses = useMemo(() => {
		const rust = parseCargoLicenses(cargoLicenses);
		const npm = parseNpmLicenses(npmLicenses);
		return [...rust, ...npm];
	}, [cargoLicenses, npmLicenses]);

	const uniqueLicenses = useMemo(() => {
		const set = new Set<string>();
		for (const l of allLicenses) {
			if (l.license) set.add(l.license);
		}
		return Array.from(set).sort();
	}, [allLicenses]);

	const filtered = useMemo(() => {
		return allLicenses.filter((l) => {
			const matchesSearch =
				search === "" ||
				l.name.toLowerCase().includes(search.toLowerCase()) ||
				l.description?.toLowerCase().includes(search.toLowerCase()) ||
				l.authors?.toLowerCase().includes(search.toLowerCase());

			const matchesLicense =
				licenseFilter === "all" || l.license === licenseFilter;

			const matchesSource = sourceFilter === "all" || l.source === sourceFilter;

			return matchesSearch && matchesLicense && matchesSource;
		});
	}, [allLicenses, search, licenseFilter, sourceFilter]);

	const stats = useMemo(() => {
		const licenseCount: Record<string, number> = {};
		for (const l of allLicenses) {
			const lic = l.license || "Unknown";
			licenseCount[lic] = (licenseCount[lic] || 0) + 1;
		}
		const rustCount = allLicenses.filter((l) => l.source === "rust").length;
		const npmCount = allLicenses.filter((l) => l.source === "npm").length;
		return {
			total: allLicenses.length,
			rustCount,
			npmCount,
			licenseCount,
		};
	}, [allLicenses]);

	return (
		<div>
			<FeaturedLibraries licenses={allLicenses} />

			<div className="mb-8">
				<h2 className="text-2xl font-semibold mb-4">Complete SBOM</h2>
				<p className="text-muted-foreground mb-4">
					Full Software Bill of Materials containing {stats.total} dependencies
					({stats.rustCount} Rust crates, {stats.npmCount} npm packages).
				</p>

				<div className="flex flex-col sm:flex-row gap-4 mb-6">
					<div className="relative flex-1 min-w-0">
						<Search className="absolute left-3 top-1/2 -translate-y-1/2 w-4 h-4 text-muted-foreground" />
						<input
							type="text"
							placeholder="Search packages..."
							value={search}
							onChange={(e) => setSearch(e.target.value)}
							className="w-full pl-10 pr-10 py-2 rounded-lg border border-border bg-background focus:outline-none focus:ring-2 focus:ring-primary/50"
						/>
						{search && (
							<button
								type="button"
								onClick={() => setSearch("")}
								className="absolute right-3 top-1/2 -translate-y-1/2 text-muted-foreground hover:text-foreground"
							>
								<X className="w-4 h-4" />
							</button>
						)}
					</div>

					<select
						value={sourceFilter}
						onChange={(e) =>
							setSourceFilter(e.target.value as "all" | "rust" | "npm")
						}
						className="w-full sm:w-44 shrink-0 px-4 py-2 rounded-lg border border-border bg-background focus:outline-none focus:ring-2 focus:ring-primary/50"
					>
						<option value="all">All Sources ({stats.total})</option>
						<option value="rust">Rust ({stats.rustCount})</option>
						<option value="npm">npm ({stats.npmCount})</option>
					</select>

					<select
						value={licenseFilter}
						onChange={(e) => setLicenseFilter(e.target.value)}
						className="w-full sm:w-48 shrink-0 px-4 py-2 rounded-lg border border-border bg-background focus:outline-none focus:ring-2 focus:ring-primary/50 truncate"
					>
						<option value="all">All Licenses</option>
						{uniqueLicenses.map((lic) => (
							<option key={lic} value={lic}>
								{lic} ({stats.licenseCount[lic] || 0})
							</option>
						))}
					</select>
				</div>

				<div className="text-sm text-muted-foreground mb-4">
					Showing {filtered.length} of {stats.total} packages
				</div>
			</div>

			<div className="overflow-x-auto rounded-lg border border-border">
				<table className="w-full text-sm">
					<thead className="bg-muted/50">
						<tr>
							<th className="px-4 py-3 text-left font-semibold">Package</th>
							<th className="px-4 py-3 text-left font-semibold">Version</th>
							<th className="px-4 py-3 text-left font-semibold">Source</th>
							<th className="px-4 py-3 text-left font-semibold">License</th>
							<th className="px-4 py-3 text-left font-semibold hidden lg:table-cell">
								Description
							</th>
							<th className="px-4 py-3 text-center font-semibold w-16">Text</th>
						</tr>
					</thead>
					<tbody className="divide-y divide-border">
						{filtered.slice(0, 500).map((l, i) => (
							<tr
								key={`${l.name}-${l.version}-${l.source}-${i}`}
								className="hover:bg-muted/30 transition-colors"
							>
								<td className="px-4 py-3">
									<div className="flex flex-col">
										{l.repository ? (
											<a
												href={l.repository}
												target="_blank"
												rel="noreferrer"
												className="font-medium hover:text-primary transition-colors inline-flex items-center gap-1"
											>
												{l.name}
												<ExternalLink className="w-3 h-3" />
											</a>
										) : (
											<span className="font-medium">{l.name}</span>
										)}
										{l.authors && (
											<span className="text-xs text-muted-foreground truncate max-w-[200px]">
												{l.authors.split("|")[0]}
											</span>
										)}
									</div>
								</td>
								<td className="px-4 py-3 text-muted-foreground font-mono text-xs">
									{l.version}
								</td>
								<td className="px-4 py-3">
									<SourceBadge source={l.source} />
								</td>
								<td className="px-4 py-3">
									<LicenseBadge license={l.license} />
								</td>
								<td className="px-4 py-3 text-muted-foreground hidden lg:table-cell">
									<span className="line-clamp-1">{l.description || "—"}</span>
								</td>
								<td className="px-4 py-3 text-center">
									<button
										type="button"
										onClick={() => setSelectedLicense(l)}
										className={`p-1.5 rounded-md transition-colors ${
											l.licenseText
												? "hover:bg-primary/10 text-primary"
												: "hover:bg-muted text-muted-foreground"
										}`}
										title={
											l.licenseText
												? "View license text"
												: "License text not available"
										}
									>
										<FileText className="w-4 h-4" />
									</button>
								</td>
							</tr>
						))}
					</tbody>
				</table>
				{filtered.length > 500 && (
					<div className="px-4 py-3 text-center text-sm text-muted-foreground bg-muted/30">
						Showing first 500 results. Use search to narrow down.
					</div>
				)}
			</div>

			{selectedLicense && (
				<LicenseTextModal
					license={selectedLicense}
					onClose={() => setSelectedLicense(null)}
				/>
			)}
		</div>
	);
}
