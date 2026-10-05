# Strata Query Language (not SQL)

Strata ships a **typed** query language for scanning datasets with projection,
predicate pushdown, and block skipping. It is intentionally not SQL and not a
general database dialect.

## CLI

```bash
strata query --dataset /path/to/dataset --query 'select ts, price where symbol == AAPL limit 10'
strata query --dataset /path/to/dataset --file query.sq --explain
strata query --dataset /path/to/dataset --query 'select *' --full-scan
```

Flags:

| Flag | Meaning |
| --- | --- |
| `--dataset` | Dataset root (required) |
| `--query` | Inline query text |
| `--file` | Read query text from a file |
| `--explain` | Print plan + stats (still runs the query) |
| `--full-scan` | Disable index block skipping |
| `--format` | `text` (default) or `json` |
| `--max-print` | Cap printed rows in text mode (default 50) |

Rebuild indexes first for best pruning:

```bash
strata index rebuild /path/to/dataset
```

## Grammar

```text
query      := SELECT select_list [ WHERE predicate { AND predicate } ] [ LIMIT uint ]
select_list := * | ident { , ident }
predicate  := ident cmp literal
            | ident BETWEEN literal AND literal
            | ident IN ( literal { , literal } )
cmp        := == | = | != | < | <= | > | >=
literal    := null | true | false | number | ident | "string" | 'string'
```

- Keywords (`select`, `where`, `and`, `between`, `in`, `limit`) are **case-insensitive**.
- Field names are **case-sensitive** and must match the schema.
- Bare words in literal position are strings (`AAPL`, `buy`).
- Only **AND** combinations are supported (no `OR` / `NOT` in v1).
- `#` starts a line comment.

## Examples

```text
# Conceptual market query
select ts, price, size
where symbol == AAPL
  and ts between 1700000000000000000 and 1700000001000000000
limit 1000

# Categorical IN + numeric range
select seq, symbol, price
where symbol in (AAPL, MSFT, GOOG)
  and price between 10000 and 20000

# Sequence point / range
select *
where seq between 1000 and 2000

# Full projection
select *
where side == buy
limit 25
```

## Execution model

1. **Validate** select list + predicates against the dataset schema.
2. **Plan** decode columns = projection ∪ predicate fields.
3. **Pushdown / skip** using indexes when present:
   - categorical `==` / `IN` → posting lists
   - timestamp ranges → time index / min-max prune
   - unsigned/sequence ranges → zone maps / min-max prune
   - numeric ranges → block min/max
4. **Stream** matching blocks:
   - decompress block frame
   - **decode only required columns**
   - apply row filters
   - project output columns
   - yield [`RecordBatch`] until `LIMIT` or end

Results are never fully materialized unless the caller collects the stream.

## What is not included

- SQL parsers / joins / group-by / order-by
- `OR` / nested boolean expressions
- Updating or deleting rows
