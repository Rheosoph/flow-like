export type InboundEmailAliasIssue = "format" | "reserved";

const RESERVED_ALIASES: ReadonlySet<string> = new Set([
	"abuse",
	"admin",
	"administrator",
	"billing",
	"bounce",
	"bounces",
	"dmarc",
	"ftp",
	"host-master",
	"hostmaster",
	"info",
	"is",
	"it",
	"mail",
	"mailer-daemon",
	"mailerdaemon",
	"marketing",
	"mis",
	"no-reply",
	"noc",
	"noreply",
	"postmaster",
	"root",
	"sales",
	"security",
	"ssl-admin",
	"ssladmin",
	"ssladministrator",
	"sslwebmaster",
	"support",
	"sysadmin",
	"webmaster",
	"www",
]);

export function normalizeInboundEmailAlias(value: string): string | null {
	return value.trim().toLowerCase() || null;
}

export function validateInboundEmailAlias(
	value: string,
): InboundEmailAliasIssue | undefined {
	const alias = normalizeInboundEmailAlias(value);
	if (alias === null) return undefined;
	if (!/^[a-z0-9][a-z0-9-]{1,62}[a-z0-9]$/.test(alias)) return "format";
	if (alias.startsWith("m-") || RESERVED_ALIASES.has(alias)) return "reserved";
	return undefined;
}
