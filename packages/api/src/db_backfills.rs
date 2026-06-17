use sea_orm::{ConnectionTrait, DatabaseConnection, DbBackend, DbErr, Statement};

pub async fn run_startup_backfills(db: &DatabaseConnection) -> Result<(), DbErr> {
    backfill_legacy_user_defaults(db).await?;
    backfill_legacy_technical_user_creators(db).await?;
    Ok(())
}

async fn backfill_legacy_user_defaults(db: &DatabaseConnection) -> Result<(), DbErr> {
    let backend = db.get_database_backend();
    let sql = match backend {
        DbBackend::Postgres => {
            r#"UPDATE "User"
SET
    "permission" = COALESCE("permission", 0),
    "tutorialCompleted" = COALESCE("tutorialCompleted", FALSE),
    "status" = COALESCE("status", 'ACTIVE'::"UserStatus"),
    "tier" = COALESCE("tier", 'FREE'::"UserTier"),
    "totalSize" = COALESCE("totalSize", 0),
    "totalLLMPrice" = COALESCE("totalLLMPrice", 0),
    "totalEmbeddingPrice" = COALESCE("totalEmbeddingPrice", 0),
    "createdAt" = COALESCE("createdAt", NOW()),
    "updatedAt" = COALESCE("updatedAt", NOW())
WHERE "permission" IS NULL
   OR "tutorialCompleted" IS NULL
   OR "status" IS NULL
   OR "tier" IS NULL
   OR "totalSize" IS NULL
   OR "totalLLMPrice" IS NULL
   OR "totalEmbeddingPrice" IS NULL
   OR "createdAt" IS NULL
   OR "updatedAt" IS NULL"#
        }
        _ => {
            r#"UPDATE "User"
SET
    "permission" = COALESCE("permission", 0),
    "tutorialCompleted" = COALESCE("tutorialCompleted", FALSE),
    "status" = COALESCE("status", 'ACTIVE'),
    "tier" = COALESCE("tier", 'FREE'),
    "totalSize" = COALESCE("totalSize", 0),
    "totalLLMPrice" = COALESCE("totalLLMPrice", 0),
    "totalEmbeddingPrice" = COALESCE("totalEmbeddingPrice", 0),
    "createdAt" = COALESCE("createdAt", CURRENT_TIMESTAMP),
    "updatedAt" = COALESCE("updatedAt", CURRENT_TIMESTAMP)
WHERE "permission" IS NULL
   OR "tutorialCompleted" IS NULL
   OR "status" IS NULL
   OR "tier" IS NULL
   OR "totalSize" IS NULL
   OR "totalLLMPrice" IS NULL
   OR "totalEmbeddingPrice" IS NULL
   OR "createdAt" IS NULL
   OR "updatedAt" IS NULL"#
        }
    };

    db.execute(Statement::from_string(backend, sql.to_string()))
        .await?;
    Ok(())
}

async fn backfill_legacy_technical_user_creators(db: &DatabaseConnection) -> Result<(), DbErr> {
    let backend = db.get_database_backend();
    let now_expr = match backend {
        DbBackend::Postgres => "NOW()",
        _ => "CURRENT_TIMESTAMP",
    };
    let sql = format!(
        r#"UPDATE "TechnicalUser"
SET
    "creatorUserId" = (
        SELECT m."userId"
        FROM "Membership" m
        JOIN "App" a ON a."id" = m."appId"
        WHERE a."id" = "TechnicalUser"."appId"
          AND a."ownerRoleId" IS NOT NULL
          AND m."roleId" = a."ownerRoleId"
        LIMIT 1
    ),
    "creatorMembershipId" = COALESCE(
        "creatorMembershipId",
        (
            SELECT m."id"
            FROM "Membership" m
            JOIN "App" a ON a."id" = m."appId"
            WHERE a."id" = "TechnicalUser"."appId"
              AND a."ownerRoleId" IS NOT NULL
              AND m."roleId" = a."ownerRoleId"
            LIMIT 1
        )
    ),
    "updatedAt" = {now_expr}
WHERE "creatorUserId" IS NULL
  AND EXISTS (
      SELECT 1
      FROM "Membership" m
      JOIN "App" a ON a."id" = m."appId"
      WHERE a."id" = "TechnicalUser"."appId"
        AND a."ownerRoleId" IS NOT NULL
        AND m."roleId" = a."ownerRoleId"
  )"#
    );

    db.execute(Statement::from_string(backend, sql)).await?;
    Ok(())
}
