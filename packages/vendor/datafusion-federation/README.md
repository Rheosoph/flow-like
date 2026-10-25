# DataFusion Federation

[![crates.io](https://img.shields.io/crates/v/datafusion-federation.svg)](https://crates.io/crates/datafusion-federation)
[![docs.rs](https://docs.rs/datafusion-federation/badge.svg)](https://docs.rs/datafusion-federation)

DataFusion Federation allows
[DataFusion](https://github.com/apache/arrow-datafusion) to execute (part of) a
query plan by a remote execution engine.

                                        ┌────────────────┐
                   ┌────────────┐       │ Remote DBMS(s) │
    SQL Query ───> │ DataFusion │  ───> │  ( execution   │
                   └────────────┘       │ happens here ) │
                                        └────────────────┘

The goal is to allow resolving queries across remote query engines while
pushing down as much compute as possible to the remote database(s). This allows
execution to happen as close to the storage as possible. This concept is
referred to as 'query federation'.

> [!TIP]
> This repository implements the federation framework itself. If you want to
> connect to a specific database, check out the compatible providers available
> in
> [datafusion-contrib/datafusion-table-providers](https://github.com/datafusion-contrib/datafusion-table-providers/).

## Usage

Check out the [examples](./datafusion-federation/examples/) to get a feel for
how it works.

For a complete step-by-step example of how federation works, you can check the
example [here](./datafusion-federation/examples/df-csv-advanced.rs).

## Potential use-cases:

- Querying across SQLite, MySQL, PostgreSQL, ...
- Pushing down SQL or [Substrait](https://substrait.io/) plans.
- DataFusion -> Flight SQL -> DataFusion
- ..

## Design concept

Say you have a query plan as follows:

                   ┌────────────┐
                   │    Join    │
                   └────────────┘
                          ▲
                  ┌───────┴────────┐
           ┌────────────┐   ┌────────────┐
           │   Scan A   │   │    Join    │
           └────────────┘   └────────────┘
                                   ▲
                           ┌───────┴────────┐
                    ┌────────────┐   ┌────────────┐
                    │   Scan B   │   │   Scan C   │
                    └────────────┘   └────────────┘

DataFusion Federation will identify the largest possible sub-plans that
can be executed by an external database:

                   ┌────────────┐      Optimizer recognizes
                   │    Join    │      that B and C are
                   └────────────┘      available in an
                          ▲            external database
           ┌──────────────┴────────┐
           │       ┌ ─  ─ ─ ─  ─ ─ ┴ ─ ── ─ ─ ─  ─ ─┐
    ┌────────────┐          ┌────────────┐          │
    │   Scan A   │ │        │    Join    │
    └────────────┘          └────────────┘          │
                   │               ▲
                           ┌───────┴────────┐       │
                    ┌────────────┐   ┌────────────┐ │
                   ││   Scan B   │   │   Scan C   │
                    └────────────┘   └────────────┘ │
                    ─ ── ─ ─ ── ─ ─ ─ ─  ─ ─ ─ ── ─ ┘

The sub-plans are cut out and replaced by an opaque federation node in the plan:

                   ┌────────────┐
                   │    Join    │
                   └────────────┘    Rewritten Plan
                          ▲
                 ┌────────┴───────────┐
                 │                    │
          ┌────────────┐    ┏━━━━━━━━━━━━━━━━━━┓
          │   Scan A   │    ┃     Scan B+C     ┃
          └────────────┘    ┃  (TableProvider  ┃
                            ┃ that can execute ┃
                            ┃ sub-plan in an   ┃
                            ┃external database)┃
                            ┗━━━━━━━━━━━━━━━━━━┛

Different databases may have different query languages and execution
capabilities. To accommodate for this, we allow each 'federation provider' to
self-determine what part of a sub-plan it will actually federate. This is done
by letting each federation provider define its own optimizer rule. When a
sub-plan is 'cut out' of the overall plan, it is first passed the federation
provider's optimizer rule. This optimizer rule determines the part of the plan
that is cut out, based on the execution capabilities of the database it
represents.

## Implementation

A remote database is represented by the `FederationProvider` trait. To identify
table scans that are available in the same database, they implement
`FederatedTableSource` trait. This trait allows lookup of the corresponding
`FederationProvider`.

Identifying sub-plans to federate is done by the `FederationOptimizerRule`.
This rule needs to be registered in your DataFusion SessionState. One easy way
to do this is using `default_session_state`. To do its job, the
`FederationOptimizerRule` currently requires that all TableProviders that need
to be federated are `FederatedTableProviderAdaptor`s. The
`FederatedTableProviderAdaptor` also has a fallback mechanism that allows
implementations to fallback to a 'vanilla' TableProvider in case the
`FederationOptimizerRule` isn't registered.

The `FederationProvider` can provide a `compute_context`. This allows it to
differentiate between multiple remote execution context of the same type. For
example two different mysql instances, database schemas, access level, etc. The
`FederationProvider` also returns the `Optimizer` that is allows it to
self-determine what part of a sub-plan it can federate.

The `sql` module implements a generic `FederationProvider` for SQL execution
engines. A specific SQL engine implements the `SQLExecutor` trait for its
engine specific execution. There are a number of compatible providers available
in
[datafusion-contrib/datafusion-table-providers](https://github.com/datafusion-contrib/datafusion-table-providers/).

## Status

The project is in alpha status. Contributions welcome; land a PR = commit
access.

- [Docs (release)](https://docs.rs/datafusion-federation)
- [Docs (main)](https://datafusion-contrib.github.io/datafusion-federation/)
