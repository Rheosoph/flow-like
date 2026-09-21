import { parseArgs } from "node:util";

// bun packages/api/scripts/payments-inventory.ts --database-url "$DATABASE_URL"
// All inspection runs in one read-only transaction. No historical payee is inferred.
const { values } = parseArgs({
	args: Bun.argv.slice(2),
	options: { "database-url": { type: "string" }, help: { type: "boolean" } },
	strict: true,
});
if (values.help) {
	console.log(
		"Usage: bun payments-inventory.ts [--database-url URL]\nDefaults to DATABASE_URL. Prints a read-only migration inventory as JSON.",
	);
	process.exit(0);
}
const databaseUrl = values["database-url"] ?? process.env.DATABASE_URL;
if (!databaseUrl) throw new Error("Set DATABASE_URL or pass --database-url");

const statement = `
BEGIN READ ONLY;
SET LOCAL statement_timeout = '60s';
WITH purchases AS (
  SELECT 'APP' AS kind,p.id,p."userId",p."appId" AS "itemId",p."stripeSessionId",p."stripePaymentIntentId",p.status,p.currency,p."pricePaid",p."createdAt"
  FROM "AppPurchase" p
  WHERE COALESCE(to_jsonb(p)->>'chargeType','legacy_platform') IN ('legacy_platform','LEGACY_PLATFORM')
  UNION ALL
  SELECT 'PACKAGE',p.id,p."userId",p."packageId",p."stripeSessionId",p."stripePaymentIntentId",p.status,p.currency,p."pricePaid",p."createdAt"
  FROM "WasmPackagePurchase" p
  WHERE COALESCE(to_jsonb(p)->>'chargeType','legacy_platform') IN ('legacy_platform','LEGACY_PLATFORM')
), priced AS (
  SELECT 'APP' AS kind,a.id,a.price::TEXT AS price,a.visibility,
    (SELECT COUNT(*) FROM "Membership" m WHERE m."appId"=a.id AND m."roleId"=a."ownerRoleId") AS "currentOwnerCount"
  FROM "App" a WHERE a.price>0
  UNION ALL
  SELECT 'PACKAGE',p.id,p.price::TEXT,p.visibility,
    (SELECT COUNT(*) FROM "WasmPackageUser" u WHERE u."packageId"=p.id AND MOD(u.permission,2)=1)
  FROM "WasmPackage" p WHERE p.price>0
), duplicate_ids AS (
  SELECT 'checkout_session' AS object,"stripeSessionId" AS id,COUNT(*) AS count,
    jsonb_agg(jsonb_build_object('kind',kind,'purchaseId',id,'userId',"userId",'itemId',"itemId",'status',status) ORDER BY kind,id) AS rows
  FROM purchases WHERE "stripeSessionId"<>'' GROUP BY "stripeSessionId" HAVING COUNT(*)>1
  UNION ALL
  SELECT 'payment_intent',"stripePaymentIntentId",COUNT(*),
    jsonb_agg(jsonb_build_object('kind',kind,'purchaseId',id,'userId',"userId",'itemId',"itemId",'status',status) ORDER BY kind,id)
  FROM purchases WHERE "stripePaymentIntentId" IS NOT NULL AND "stripePaymentIntentId"<>'' GROUP BY "stripePaymentIntentId" HAVING COUNT(*)>1
), missing_access AS (
  SELECT p.kind,p.id AS "purchaseId",p."userId",p."itemId",p.status,p.currency,p."pricePaid"::TEXT AS "pricePaid"
  FROM purchases p WHERE p.status IN ('COMPLETED','PARTIALLY_REFUNDED') AND (
    (p.kind='APP' AND NOT EXISTS(SELECT 1 FROM "Membership" m WHERE m."userId"=p."userId" AND m."appId"=p."itemId")) OR
    (p.kind='PACKAGE' AND NOT EXISTS(SELECT 1 FROM "WasmPackageUser" u WHERE u."userId"=p."userId" AND u."packageId"=p."itemId"))
  )
), totals AS (
  SELECT kind,LOWER(currency) AS currency,status,COUNT(*) AS "rowCount",SUM("pricePaid")::TEXT AS "recordedGrossMinor"
  FROM purchases GROUP BY kind,LOWER(currency),status
)
SELECT jsonb_build_object(
  'generatedAt',CURRENT_TIMESTAMP,
  'readOnly',true,
  'revenueAssignment','unassigned; current ownership is not historical payee evidence',
  'totalsCaution','Recorded row totals include duplicate rows and are not reconciled Stripe revenue.',
  'missingAccessCaution','Missing access can reflect revocation or deletion and requires source review.',
  'sampleLimit',1000,
  'pricedListingCount',(SELECT COUNT(*) FROM priced),
  'pricedListings',COALESCE((SELECT jsonb_agg(to_jsonb(x)) FROM (SELECT * FROM priced ORDER BY kind,id LIMIT 1000) x),'[]'::jsonb),
  'ambiguousOwnerCount',(SELECT COUNT(*) FROM priced WHERE "currentOwnerCount"<>1),
  'ambiguousOwners',COALESCE((SELECT jsonb_agg(to_jsonb(x)) FROM (SELECT * FROM priced WHERE "currentOwnerCount"<>1 ORDER BY kind,id LIMIT 1000) x),'[]'::jsonb),
  'duplicateObjectCount',(SELECT COUNT(*) FROM duplicate_ids),
  'duplicateObjects',COALESCE((SELECT jsonb_agg(to_jsonb(x)) FROM (SELECT * FROM duplicate_ids ORDER BY object,id LIMIT 1000) x),'[]'::jsonb),
  'paidRowsWithoutAccessCount',(SELECT COUNT(*) FROM missing_access),
  'paidRowsWithoutAccess',COALESCE((SELECT jsonb_agg(to_jsonb(x)) FROM (SELECT * FROM missing_access ORDER BY kind,"purchaseId" LIMIT 1000) x),'[]'::jsonb),
  'currencyTotals',COALESCE((SELECT jsonb_agg(to_jsonb(x)) FROM (SELECT * FROM totals ORDER BY kind,currency,status) x),'[]'::jsonb)
);
COMMIT;
`;
const result = Bun.spawnSync(
	["psql", "-X", "-q", "-A", "-t", "-v", "ON_ERROR_STOP=1", databaseUrl],
	{ stdin: Buffer.from(statement), stdout: "pipe", stderr: "pipe" },
);
if (result.exitCode !== 0) {
	throw new Error(result.stderr.toString() || "Payment inventory failed");
}
console.log(JSON.stringify(JSON.parse(result.stdout.toString()), null, 2));
