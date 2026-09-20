import { expect, test } from "bun:test";
import { readFileSync } from "node:fs";
import { resolve } from "node:path";

const databaseUrl = process.env.FLOW_LIKE_PAYMENT_SCHEMA_TEST_DATABASE_URL;
const migrationName = "20260920120000_payments_foundations";

function sql(statement: string): string {
	const result = Bun.spawnSync(
		["psql", "-X", "-q", "-t", "-A", "-v", "ON_ERROR_STOP=1", databaseUrl!],
		{ stdin: Buffer.from(statement), stdout: "pipe", stderr: "pipe" },
	);
	if (result.exitCode !== 0) {
		throw new Error(
			result.stderr.toString() ||
				`PostgreSQL fixture failed: ${result.exitCode}`,
		);
	}
	return result.stdout.toString().trim();
}

const legacyFixture = `
CREATE TABLE "User" (id TEXT PRIMARY KEY);
CREATE TABLE "App" (id TEXT PRIMARY KEY);
CREATE TABLE "WasmPackage" (id TEXT PRIMARY KEY);
CREATE TABLE "AppDiscount" (id TEXT PRIMARY KEY);
CREATE TABLE "JoinQueue" (id TEXT PRIMARY KEY);
CREATE TABLE "AppPurchase" (
  id TEXT PRIMARY KEY,
  "userId" TEXT REFERENCES "User"(id) ON DELETE CASCADE,
  "appId" TEXT REFERENCES "App"(id) ON DELETE CASCADE,
  "discountId" TEXT REFERENCES "AppDiscount"(id) ON DELETE SET NULL,
  "stripeSessionId" TEXT NOT NULL
);
CREATE TABLE "WasmPackagePurchase" (
  id TEXT PRIMARY KEY,
  "userId" TEXT REFERENCES "User"(id) ON DELETE CASCADE,
  "packageId" TEXT REFERENCES "WasmPackage"(id) ON DELETE CASCADE,
  "stripeSessionId" TEXT NOT NULL
);
INSERT INTO "User" VALUES ('buyer');
INSERT INTO "App" VALUES ('app');
INSERT INTO "WasmPackage" VALUES ('package');
INSERT INTO "AppDiscount" VALUES ('discount');
INSERT INTO "AppPurchase" VALUES
  ('sale-a', 'buyer', 'app', 'discount', 'cs_duplicate'),
  ('sale-b', 'buyer', 'app', 'discount', 'cs_duplicate');
INSERT INTO "WasmPackagePurchase" VALUES
  ('package-a', 'buyer', 'package', 'cs_duplicate'),
  ('package-b', 'buyer', 'package', 'cs_duplicate');
`;

for (const tree of ["migrations", "migrations-dsql"]) {
	test.skipIf(!databaseUrl)(
		`${tree}: payment migration preserves existing money and scopes new identities`,
		() => {
			const schema = `payment_schema_${process.pid}_${Date.now()}_${tree.replaceAll("-", "_")}`;
			const prefix = `SET search_path TO "${schema}";\n`;
			const migration = readFileSync(
				resolve(
					import.meta.dir,
					"../prisma",
					tree,
					migrationName,
					"migration.sql",
				),
				"utf8",
			).replaceAll("INDEX ASYNC", "INDEX");
			sql(`CREATE SCHEMA "${schema}"`);
			try {
				sql(prefix + legacyFixture + migration);
				sql(`${prefix}
INSERT INTO "PaymentOrder" (id,kind,"userId","itemId","payeeUserId","connectedAccountId","platformAccountId",livemode,"chargeType",amount,currency,"applicationFeeAmount","feeBps",snapshot,"expiresAt","nextCheckAt","createdAt","updatedAt")
VALUES ('historical','APP','buyer','app','seller','acct_seller','acct_platform',false,'DESTINATION',1000,'eur',100,1000,'{}',1,1,1,1);
`);
				const platformMigration = readFileSync(
					resolve(
						import.meta.dir,
						"../prisma",
						tree,
						"20260920130000_platform_owned_payments/migration.sql",
					),
					"utf8",
				);
				sql(prefix + platformMigration);
				expect(
					sql(
						`${prefix} SELECT "connectedAccountId" || ':' || "chargeType" FROM "PaymentOrder" WHERE id='historical';`,
					),
				).toBe("acct_seller:DESTINATION");
				sql(`${prefix}
INSERT INTO "PaymentOrder" (id,kind,"userId","itemId","payeeUserId","connectedAccountId","platformAccountId",livemode,"chargeType",amount,currency,"applicationFeeAmount","feeBps",snapshot,"expiresAt","nextCheckAt","createdAt","updatedAt")
VALUES ('platform','APP','buyer','app','admin',NULL,'acct_platform',false,'PLATFORM',1000,'eur',0,0,'{}',1,1,1,1);
INSERT INTO "PaymentRequest" (id,"runId","nodeId",nonce,"requestDigest","appId","payerUserId","payeeUserId","connectedAccountId","platformAccountId",livemode,amount,currency,"applicationFeeAmount","feeBps","productName",description,snapshot,"expiresAt","nextCheckAt","createdAt","updatedAt")
VALUES ('platform-request','run','node','nonce','digest','app','buyer','admin',NULL,'acct_platform',false,1000,'eur',0,0,'Product','','{}',1,1,1,1);
`);
				expect(
					sql(
						`${prefix} SELECT count(*) FROM "PaymentOrder" WHERE "connectedAccountId" IS NULL; SELECT count(*) FROM "PaymentRequest" WHERE "connectedAccountId" IS NULL;`,
					),
				).toBe("1\n1");
				sql(
					`${prefix}
DELETE FROM "User";
DELETE FROM "App";
DELETE FROM "WasmPackage";
DELETE FROM "AppDiscount";
`,
				);
				expect(
					sql(
						`${prefix} SELECT count(*) FROM "AppPurchase"; SELECT count(*) FROM "WasmPackagePurchase";`,
					),
				).toBe("2\n2");
				expect(
					sql(
						`${prefix} SELECT count(*) FROM "AppPurchase" WHERE "discountId" = 'discount' AND "chargeType" IS NULL;`,
					),
				).toBe("2");

				// One command can recur on a different Stripe scope, never twice on the same scope.
				sql(`${prefix}
INSERT INTO "StripeOperation" (id, "sourceType", "sourceId", operation, "platformAccountId", "scopeKey", livemode, "requestParams", "requestDigest", "idempotencyKey", "firstAttemptAt", "lastAttemptAt", "nextAttemptAt", "createdAt", "updatedAt")
SELECT 'operation-' || scope, 'ORDER', 'order', 'create', 'platform', scope, false, '{}', 'digest', 'same-key', 1, 1, 1, 1, 1
FROM (VALUES ('acct_a'), ('acct_b')) AS scopes(scope);
`);
				expect(() =>
					sql(
						`${prefix} INSERT INTO "StripeOperation" SELECT 'duplicate-operation', "sourceType", "sourceId", operation, "platformAccountId", "scopeKey", "connectedAccountId", livemode, "requestParams", "requestDigest", "idempotencyKey", status, response, "stripeObjectId", "requestId", error, "firstAttemptAt", "lastAttemptAt", "nextAttemptAt", attempts, revision, "leaseOwner", "leaseExpiresAt", "createdAt", "updatedAt" FROM "StripeOperation" LIMIT 1;`,
					),
				).toThrow();

				// Different purchase sources survive independently; duplicate delivery cannot add a grant.
				sql(`${prefix}
INSERT INTO "AccessGrant" (id, "userId", "itemKind", "itemId", "sourceType", "sourceId", "createdAt", "updatedAt")
VALUES ('grant-a', 'buyer', 'APP', 'app', 'PURCHASE', 'sale-a', 1, 1), ('grant-b', 'buyer', 'APP', 'app', 'PURCHASE', 'sale-b', 1, 1);
UPDATE "AccessGrant" SET status = 'REVOKED' WHERE "sourceId" = 'sale-b';
INSERT INTO "AccessGrant" (id, "userId", "itemKind", "itemId", "sourceType", "sourceId", "createdAt", "updatedAt")
VALUES ('grant-replay', 'buyer', 'APP', 'app', 'PURCHASE', 'sale-a', 1, 1) ON CONFLICT DO NOTHING;
`);
				expect(
					sql(
						`${prefix} SELECT count(*) FROM "AccessGrant" WHERE status = 'ACTIVE';`,
					),
				).toBe("1");
				expect(
					sql(`${prefix} SELECT count(*) FROM "PaymentLegacyArchive";`),
				).toBe("0");
			} finally {
				sql(`DROP SCHEMA "${schema}" CASCADE`);
			}
		},
	);
}
