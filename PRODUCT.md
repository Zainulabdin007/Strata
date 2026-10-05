# Strata Product Definition

Strata is a Rust storage engine focused on extremely compact storage of massive structured event datasets.

## Core philosophy

> Minimize bits per logical value subject to user-defined ingest, decode, query, durability, and memory constraints.

## What Strata is not

- a PostgreSQL clone
- a SQL database
- an ORM
- a generic web backend
- a wrapper around Zstd
- a finance-specific database

## Design intent

Strata should understand schemas and data statistics, then choose specialized encoding pipelines per column and per block.

## Target workloads (eventually)

- market events
- telemetry
- logs
- clickstreams
- IoT data
- transactions
- ordered event streams

## Relationship to Apex

Apex will later use Strata as its persistent market-event store, but Strata must remain completely usable without Apex.

## Operating principles

1. **Correctness first.**
2. **Optimization second.**
3. **Measurement always.**
4. **Do not fabricate benchmark results.**
