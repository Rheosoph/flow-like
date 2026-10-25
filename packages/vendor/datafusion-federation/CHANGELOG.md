# Changelog

All notable changes to this project will be documented in this file.

The format is based on [Keep a Changelog](https://keepachangelog.com/en/1.0.0/),
and this project adheres to [Semantic Versioning](https://semver.org/spec/v2.0.0.html).

## [Unreleased]

## [0.5.3](https://github.com/datafusion-contrib/datafusion-federation/compare/v0.5.2...v0.5.3) - 2026-03-30

### Other

- Fix `Explain Analyze` ([#168](https://github.com/datafusion-contrib/datafusion-federation/pull/168))

## [0.5.2](https://github.com/datafusion-contrib/datafusion-federation/compare/v0.5.1...v0.5.2) - 2026-03-24

### Other

- Upgrade to DataFusion 53 ([#164](https://github.com/datafusion-contrib/datafusion-federation/pull/164))

## [0.5.1](https://github.com/datafusion-contrib/datafusion-federation/compare/v0.5.0...v0.5.1) - 2026-03-04

### Added

- *(sql)* add SQL query rewriter hook to SQLTable ([#163](https://github.com/datafusion-contrib/datafusion-federation/pull/163))

### Fixed

- *(sql)* apply table-scan rewrite before EXPLAIN display ([#161](https://github.com/datafusion-contrib/datafusion-federation/pull/161))

## [0.5.0](https://github.com/datafusion-contrib/datafusion-federation/compare/v0.4.14...v0.5.0) - 2026-02-26

### Other

- Pushdown physical filters to `SQLExecutor` ([#160](https://github.com/datafusion-contrib/datafusion-federation/pull/160))

## [0.4.14](https://github.com/datafusion-contrib/datafusion-federation/compare/v0.4.13...v0.4.14) - 2026-01-14

### Other

- update Cargo.toml dependencies

## [0.4.13](https://github.com/datafusion-contrib/datafusion-federation/compare/v0.4.12...v0.4.13) - 2026-01-13

### Other

- update Cargo.toml dependencies

## [0.4.12](https://github.com/datafusion-contrib/datafusion-federation/compare/v0.4.11...v0.4.12) - 2025-12-04

### Other

- Fix conversion of empty `RecordBatch` ([#154](https://github.com/datafusion-contrib/datafusion-federation/pull/154))

## [0.4.11](https://github.com/datafusion-contrib/datafusion-federation/compare/v0.4.10...v0.4.11) - 2025-11-24

### Other

- Upgrade datafusion to 51, arrow to 57 ([#151](https://github.com/datafusion-contrib/datafusion-federation/pull/151))

## [0.4.10](https://github.com/datafusion-contrib/datafusion-federation/compare/v0.4.9...v0.4.10) - 2025-09-18

### Other

- Upgrade datafusion to version 50 ([#149](https://github.com/datafusion-contrib/datafusion-federation/pull/149))

## [0.4.9](https://github.com/datafusion-contrib/datafusion-federation/compare/v0.4.8...v0.4.9) - 2025-08-19

### Other

- Cargo clippy 1.89 ([#146](https://github.com/datafusion-contrib/datafusion-federation/pull/146))

## [0.4.8](https://github.com/datafusion-contrib/datafusion-federation/compare/v0.4.7...v0.4.8) - 2025-08-05

### Other

- Add metrics to `SchemaCastScanExec` and `VirtualExecutionPlan` ([#143](https://github.com/datafusion-contrib/datafusion-federation/pull/143))

## [0.4.7](https://github.com/datafusion-contrib/datafusion-federation/compare/v0.4.6...v0.4.7) - 2025-07-29

### Other

- Upgrade datafusion to version 49 ([#140](https://github.com/datafusion-contrib/datafusion-federation/pull/140))

## [0.4.6](https://github.com/datafusion-contrib/datafusion-federation/compare/v0.4.5...v0.4.6) - 2025-07-21

### Other

- Make VirtualExecutionPlan public ([#138](https://github.com/datafusion-contrib/datafusion-federation/pull/138))

## [0.4.5](https://github.com/datafusion-contrib/datafusion-federation/compare/v0.4.4...v0.4.5) - 2025-07-09

### Other

- Add ability to set statistics in the SQLExecutor ([#134](https://github.com/datafusion-contrib/datafusion-federation/pull/134))

## [0.4.4](https://github.com/datafusion-contrib/datafusion-federation/compare/v0.4.3...v0.4.4) - 2025-06-30

### Other

- Minor clippy fixes introduced in rust 1.88 ([#132](https://github.com/datafusion-contrib/datafusion-federation/pull/132))

## [0.4.3](https://github.com/datafusion-contrib/datafusion-federation/compare/v0.4.2...v0.4.3) - 2025-06-22

### Other

- Update DataFusion to 48 ([#130](https://github.com/datafusion-contrib/datafusion-federation/pull/130))

## [0.4.2](https://github.com/datafusion-contrib/datafusion-federation/compare/v0.4.1...v0.4.2) - 2025-04-21

### Other

- Fix errors in the `sql` feature ([#124](https://github.com/datafusion-contrib/datafusion-federation/pull/124))

## [0.4.1](https://github.com/datafusion-contrib/datafusion-federation/compare/v0.4.0...v0.4.1) - 2025-04-21

### Other

- update Cargo.toml dependencies

## [0.4.0](https://github.com/datafusion-contrib/datafusion-federation/compare/v0.3.7...v0.4.0) - 2025-04-14

### Other

- Refactor the SQL implementation to include the `SQLTable` trait and add support for parameterized views. ([#117](https://github.com/datafusion-contrib/datafusion-federation/pull/117))

## [0.3.7](https://github.com/datafusion-contrib/datafusion-federation/compare/v0.3.6...v0.3.7) - 2025-04-03

### Other

- update to datafusion 46 ([#115](https://github.com/datafusion-contrib/datafusion-federation/pull/115))

## [0.3.6](https://github.com/datafusion-contrib/datafusion-federation/compare/v0.3.5...v0.3.6) - 2025-02-19

### Other

- update Cargo.toml dependencies

## [0.3.5](https://github.com/datafusion-contrib/datafusion-federation/compare/datafusion-federation-v0.3.4...datafusion-federation-v0.3.5) - 2025-01-20

### Other

- Use the Dialect and Unparser constructor when using the plan_to_sql function. (#105)

## [0.3.4](https://github.com/datafusion-contrib/datafusion-federation/compare/datafusion-federation-v0.3.3...datafusion-federation-v0.3.4) - 2025-01-12

### Other

- upgrade datafusion to 44 (#103)

## [0.3.3](https://github.com/datafusion-contrib/datafusion-federation/compare/datafusion-federation-v0.3.2...datafusion-federation-v0.3.3) - 2025-01-04

### Fixed

- handle `LogicalPlan::Limit` separately to preserve skip and offset in `rewrite_table_scans` (#101)

## [0.3.2](https://github.com/datafusion-contrib/datafusion-federation/compare/datafusion-federation-v0.3.1...datafusion-federation-v0.3.2) - 2024-12-05

### Other

- Release plz action: install required dependencies ([#85](https://github.com/datafusion-contrib/datafusion-federation/pull/85))
