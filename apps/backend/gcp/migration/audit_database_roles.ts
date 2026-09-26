// Repository-side resolution of the import migrate.ts makes. The image does
// not carry this file: its Dockerfile copies apps/backend/shared/audit_database_roles.ts
// to this path, next to node_modules, the way the compose db-init image does.
export { provisionAuditDatabaseRoles } from "../../shared/audit_database_roles";
