# API Development

## Security Pattern

Always check comparable endpoints before implementing new ones. Two layers of access control are required:

1. **Macro guard** at the handler entry:
```rust
ensure_permission!(user, &app_id, &state, RolePermissions::Read);
```

2. **SQL-level enforcement** — filter by user/org in queries, never rely on the macro alone. The macro checks project access; the query must scope the data.

## OpenAPI

Every endpoint needs `utoipa` annotations. Descriptions target end users, not developers:
```rust
#[utoipa::path(
    get,
    path = "/api/v1/resource/{id}",
    params(("id" = String, Path, description = "Resource identifier")),
    responses(
        (status = 200, description = "Returns the resource details", body = ResourceResponse),
        (status = 404, description = "Resource not found")
    ),
    tag = "Resources"
)]
```

## Performance

- Cache frequently-accessed data (user profiles, permissions, config).
- Avoid N+1 queries — prefer joins or batch fetches.
- Consider endpoint call frequency when deciding optimization effort.
- Use database indices for common filter/sort columns.
