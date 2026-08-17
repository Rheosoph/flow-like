---
title: Dates & Times
description: How Flow-Like represents the Date type, which inputs it parses, the format placeholders it accepts, and how dates are stored in tables
sidebar:
  order: 5
---

Flow-Like has one temporal type: **Date**. A Date value is an *instant* — a
point on the timeline — held in UTC. It is not a calendar day, and it does not
carry a timezone. Any offset in the input is folded into UTC when the value is
parsed; local time exists only where a value is displayed.

## The value itself

| Layer | Representation |
|-------|----------------|
| Pin and variable value (JSON on the wire) | RFC3339 string with a `Z` suffix — `"2026-08-17T12:00:00Z"` |
| Runtime | `chrono::DateTime<Utc>` |
| Struct schema / JSON Schema | `{"type": "string", "format": "date-time"}` (`"format": "date"` is also read as a Date) |
| Event, REST and MCP payload schema | `{"type": "string"}` |
| WASM node pin | pin type `date` or `datetime` |
| Table column | Arrow `Timestamp(Millisecond, "UTC")` |
| Legacy shape, still accepted on input | `{"secs_since_epoch": 1786968000, "nanos_since_epoch": 0}` |

Fractional seconds appear in the string only when they are non-zero, and then
with 3, 6, or 9 digits: `"2026-08-17T12:00:00.123456Z"`.

A new Date variable defaults to the moment it was created, written as an ISO
string. See [Variables](/studio/variables/) for defaults, exposure, and runtime
configuration.

:::note[Z versus +00:00]
The value carried between pins ends in `Z`. [Format DateTime](/nodes/utils/datetime/utils-datetime-format/)
with the `rfc3339` preset writes the same instant as `2026-08-17T12:00:00+00:00`.
Both are valid RFC3339 and both parse back identically — but if a downstream
system does a string comparison, pick one and stay with it.
:::

## Producing a Date

| Source | How |
|--------|-----|
| Current time | [Now](/nodes/utils/datetime/utils-datetime-now/) — always UTC |
| A string | [Parse DateTime](/nodes/utils/datetime/utils-datetime-parse/) |
| Any other value | Cast it to Date with [Try Transform](/nodes/utils/types/utils-types-try-transform/) |
| A struct field | [Break Struct](/nodes/structs/struct-break/) turns `format: date-time` fields into Date pins |
| An event payload | Date pins render as a `datetime-local` control in the event form |
| Arithmetic | [Add Duration](/nodes/utils/datetime/utils-datetime-duration/) on an existing Date |

## What Flow-Like parses

### Auto-detection

With the **Format** pin left empty, [Parse DateTime](/nodes/utils/datetime/utils-datetime-parse/)
tries these in order and takes the first that succeeds:

1. **RFC3339** — `2026-08-17T12:00:00Z`, `2026-08-17T12:00:00.123+02:00`. An
   offset is converted to UTC, so `12:00+02:00` becomes `10:00Z`.
2. **RFC2822** — `Mon, 17 Aug 2026 12:00:00 +0000`.
3. **An all-digit string** — read as Unix **seconds**.
4. **Common layouts**, in this order:

   ```
   %Y-%m-%d %H:%M:%S      %Y-%m-%dT%H:%M:%S
   %Y-%m-%d %H:%M:%S%.f   %Y-%m-%dT%H:%M:%S%.f
   %Y-%m-%d
   %d/%m/%Y   %m/%d/%Y   %d.%m.%Y   %Y/%m/%d   %d-%m-%Y   %m-%d-%Y
   ```

Two rules govern everything on that list. A layout with no offset is read as
**UTC**, not as local time — `2026-08-17 12:00:00` is 12:00 UTC. A layout with
no time component becomes **midnight UTC**.

:::caution[Ambiguous slash dates]
`%d/%m/%Y` is tried before `%m/%d/%Y`, so `03/04/2026` auto-detects as **3 April**,
not 4 March. When the source is a system you do not control, pass an explicit
format string instead of relying on the order.
:::

### With an explicit format

Give **Format** a [chrono format string](#format-placeholders) and the node
tries `NaiveDateTime` first, then `NaiveDate` (which lands on midnight). Both
results are interpreted as UTC.

:::caution[An offset in the format string is discarded]
On the explicit-format path the value is parsed as a *naive* local wall clock,
so `%z` / `%:z` in your format is consumed and then thrown away. Parsing
`2026-08-17T12:00:00+02:00` with `%Y-%m-%dT%H:%M:%S%:z` yields **12:00 UTC**,
while auto-detection yields the correct **10:00 UTC**. If the input carries an
offset, leave the format empty.
:::

### Casting other values

Casting a value to Date with [Try Transform](/nodes/utils/types/utils-types-try-transform/)
accepts more shapes than the parse node:

| Input | Result |
|-------|--------|
| RFC3339 string | Converted to UTC |
| `%Y-%m-%d`, `%d/%m/%Y`, `%m/%d/%Y`, `%Y-%m-%d %H:%M:%S` | Read as UTC |
| Integer greater than `946684800000` (the year 2000 in millis) | Unix **milliseconds** |
| Any smaller integer | Unix **seconds** |
| Float | Seconds, fractional part as nanoseconds |
| `{"secs_since_epoch": …, "nanos_since_epoch": …}` | The legacy shape |

:::caution[`"1786968000"` and `1786968000` are not the same input]
Parse DateTime reads a numeric *string* as seconds. Casting reads a *number* as
milliseconds once it passes the year-2000 threshold. Feed epoch values through
one path consistently, or a millisecond value that arrived as text lands 50,000
years in the future.
:::

Event forms and generated payload schemas use a third, stricter reader: RFC3339,
`%Y-%m-%d %H:%M:%S%.f %:z`, the four `%Y-%m-%d[T ]%H:%M:%S[%.f]` variants, and
plain `%Y-%m-%d`. Numbers follow the same millis/seconds threshold as casting.

### In the interface

Table and inspector views read dates back out of values the backend has already
flattened. A column *declared* temporal is parsed by magnitude — days, seconds,
millis, micros, or nanos. A column that is merely an `Int64` is only shown as a
date when its **name** promises an instant (`created_at`, `date_of_birth`,
`expires`) *and* the result lands between 1990 and 2100. That keeps `duration`,
`rating`, and `estimate` rendering as the numbers they are.

## Formatting

[Format DateTime](/nodes/utils/datetime/utils-datetime-format/) takes a Date and
a format string. Two names are presets (matched case-insensitively); anything
else is treated as a chrono pattern.

| Format pin | Output for `2026-08-17T12:00:00Z` |
|------------|-----------------------------------|
| `rfc3339` (default) | `2026-08-17T12:00:00+00:00` |
| `rfc2822` | `Mon, 17 Aug 2026 12:00:00 +0000` |
| `%Y-%m-%d %H:%M:%S` | `2026-08-17 12:00:00` |
| `%d.%m.%Y %H:%M` | `17.08.2026 12:00` |
| `%B %e, %Y` | `August 17, 2026` |

### Format placeholders

All output below is for `2026-08-17T12:00:00Z`, a Monday.

**Date**

| Placeholder | Meaning | Output |
|-------------|---------|--------|
| `%Y` | Year, zero-padded | `2026` |
| `%y` | Year within century | `26` |
| `%C` | Century | `20` |
| `%m` | Month number | `08` |
| `%b` | Month, abbreviated | `Aug` |
| `%B` | Month, full | `August` |
| `%d` | Day of month, zero-padded | `17` |
| `%e` | Day of month, space-padded | `17` |
| `%a` | Weekday, abbreviated | `Mon` |
| `%A` | Weekday, full | `Monday` |
| `%j` | Day of year | `229` |
| `%u` / `%w` | Weekday number (Mon=1 / Sun=0) | `1` / `1` |
| `%U` / `%W` | Week of year (Sunday- / Monday-based) | `33` / `33` |
| `%G` / `%V` | ISO week-year and ISO week number | `2026` / `34` |

**Time**

| Placeholder | Meaning | Output |
|-------------|---------|--------|
| `%H` | Hour, 24-hour, zero-padded | `12` |
| `%k` | Hour, 24-hour, space-padded | `12` |
| `%I` | Hour, 12-hour, zero-padded | `12` |
| `%l` | Hour, 12-hour, space-padded | `12` |
| `%p` / `%P` | `AM`/`PM` — upper and lower | `PM` / `pm` |
| `%M` | Minute | `00` |
| `%S` | Second | `00` |
| `%.3f` | Fractional seconds, dot included, fixed width | `.000` |
| `%3f` | Fractional seconds, no dot | `000` |
| `%.f` | Fractional seconds, only when non-zero | *(empty)* |
| `%s` | Unix timestamp in seconds | `1786968000` |

**Zone and composites**

| Placeholder | Meaning | Output |
|-------------|---------|--------|
| `%z` | Offset | `+0000` |
| `%:z` | Offset with colon | `+00:00` |
| `%Z` | Zone name | `UTC` |
| `%F` | `%Y-%m-%d` | `2026-08-17` |
| `%T` | `%H:%M:%S` | `12:00:00` |
| `%R` | `%H:%M` | `12:00` |
| `%D` / `%x` | `%m/%d/%y` | `08/17/26` |
| `%X` | Locale time | `12:00:00` |
| `%c` | Full timestamp | `Mon Aug 17 12:00:00 2026` |
| `%+` | ISO 8601 / RFC3339 | `2026-08-17T12:00:00+00:00` |
| `%%` | A literal percent sign | `%` |

:::caution[An unknown placeholder fails the run]
Formatting is strict: a typo like `%Q` is not passed through as text, it aborts
the run. Validate a format string once when you author the flow rather than
discovering it in production. Escape any literal percent as `%%`.
:::

Formatting always renders **UTC**. There is no node that converts a Date into
another timezone — if a user-facing string must be local, format it in the
interface layer, which knows the viewer's zone.

## Reading parts and doing arithmetic

| Node | Gives you |
|------|-----------|
| [To Date](/nodes/utils/datetime/utils-datetime-to-date/) | `year`, `month` (1–12), `day` (1–31), `weekday` (**0 = Monday**, 6 = Sunday), `day_of_year` (1–366) |
| [To Time](/nodes/utils/datetime/utils-datetime-to-time/) | `hour` (0–23), `minute`, `second`, `nanosecond` |
| [Add Duration](/nodes/utils/datetime/utils-datetime-duration/) | A new Date; days/hours/minutes/seconds are additive and may be negative. Overflow is an error, not a wrap |
| [DateTime Difference](/nodes/utils/datetime/utils-datetime-diff/) | `total_seconds` (signed), split `days`/`hours`/`minutes`/`seconds`, and a `human_readable` string. Has `Error` pins because it also accepts the legacy `secs_since_epoch` shape |

## How dates are stored in tables

App tables are LanceDB tables with an Arrow schema, so a Date has to become a
physical column type.

### Column types

| Type in the table designer | Arrow type | Holds |
|----------------------------|------------|-------|
| **Timestamp** | `Timestamp(Millisecond, "UTC")` | An instant — the physical form of a Flow-Like Date |
| **Date** | `Date32` | A calendar day with no time, stored as a day count since 1970-01-01 |

When you create a table through the API or an agent, the type name is one of
`timestamp`, `datetime`, `timestamp_ms`, `timestamp:ms:utc` (all the same), or
`date` / `date32`.

### Getting a Date into a column

The **first** write to a new table infers the schema. A string column whose
non-null values all parse as RFC3339 is promoted to `Timestamp(Millisecond, "UTC")`
— which is exactly what a Date pin produces, so dates land in a real timestamp
column without any declaration.

After that, the stored schema is authoritative and every later write is
serialized against it:

- A column that was inferred as text stays text forever, even once every later
  row is a valid RFC3339 string. One malformed value in the first batch decides
  the column. Declare the schema up front when the first batch is not trustworthy.
- Tables written before UTC-aware timestamps keep working. An RFC3339 value
  written into a timezone-less timestamp column is converted to a naive UTC wall
  clock, and a naive string written into a UTC column is read as UTC.
- Storage is **millisecond** resolution. Microseconds and nanoseconds present in
  the Date value are truncated on write.

### What comes back out

The read path matters, because the same column produces different JSON depending
on how you reach it:

| Read through | A `Timestamp` column returns | A `Date32` column returns |
|--------------|------------------------------|---------------------------|
| Table reads — list, filter, vector query | Epoch **milliseconds** as an integer: `1786881600000` | A **day count** as an integer: `20682` |
| A SQL query over the table | RFC3339 string: `"2026-08-17T12:00:00.000Z"` (timezone-less columns keep the suffix-less form `2026-08-17T12:00:00.000`) | `"2026-08-17"` |

A raw table read hands you a number, not a date string. Cast it back to a Date
before doing date work with it — the millis/seconds threshold described above
resolves it correctly. The table viewer does the same thing for you, which is
why an integer column named `created_at` still renders as a date.

### Filtering and updating rows

Filters are SQL fragments evaluated against the *physical* column, so a
timestamp needs a typed literal. A bare integer does not coerce and the update
fails:

```sql
-- fails: an integer literal is not a timestamp
created_at = 1786881600000

-- works
created_at = CAST(1786881600000 AS TIMESTAMP(3))
created_at >= TIMESTAMP '2026-08-17 12:00:00'
```

The rest of the fragment follows ordinary SQL rules: escape a quote in a string
literal by doubling it (`label = 'it''s a'`), and compare against `IS NULL`
rather than `= NULL`. Adding a timestamp column to an existing table works the
same way, through `CAST('2026-08-17' AS TIMESTAMP)`.

## Dates in SQL and DataFusion

| Node | Purpose |
|------|---------|
| [DateTime to SQL Timestamp](/nodes/data/datafusion/time/df-datetime-to-timestamp/) | Turns a Date into a `TIMESTAMP '2026-08-17 12:00:00.000000'` literal, plus the same instant in epoch microseconds |
| [Time Range Filter](/nodes/data/datafusion/time/df-time-range-filter/) | Builds a half-open `col >= … AND col < …` WHERE clause |
| [Time Bin Aggregation](/nodes/data/datafusion/aggregation/df-time-bin-aggregation/) | Groups rows into fixed intervals via `date_bin` |
| [Date Truncate Aggregation](/nodes/data/datafusion/aggregation/df-date-trunc-aggregation/) | Truncates to hour, day, month, and so on, then aggregates |

Time Range Filter accepts relative expressions as well as ISO strings, which
avoids a Now-plus-arithmetic chain for the common "last day" case:

| Expression | Meaning |
|------------|---------|
| `now` | The moment the node runs |
| `-24h`, `-7d`, `-30m`, `+1w` | Signed offset from now; units are `s`, `m`, `h`, `d`, `w` |
| `2026-08-17T12:00:00Z`, `2026-08-17 12:00:00`, `2026-08-17` | Absolute, read as UTC |

## Common pitfalls

| Symptom | Cause |
|---------|-------|
| A time is off by exactly the local offset | A string with no offset was parsed — those are UTC, never local |
| A time is off by the offset it explicitly carried | The offset was given to the explicit-format path, which discards it. Leave the format empty for RFC3339 input |
| A day and month are swapped | Auto-detection tries `%d/%m/%Y` before `%m/%d/%Y` |
| An epoch value lands in the far future | A millisecond value went through a path that reads seconds, or vice versa |
| A date column reads as plain text in a table | The first write to that table contained a value that was not RFC3339 |
| Sub-millisecond precision disappeared | Table storage is millisecond resolution |
| An update touches no rows, or errors on the filter | The timestamp literal was not cast — use `CAST(… AS TIMESTAMP(3))` |
| The run aborts inside a format node | An unrecognized `%` placeholder |

## See also

- [Variables](/studio/variables/) — declaring a Date variable, defaults, and runtime configuration
- [DateTime nodes](/nodes/utils/datetime/) — the full generated reference for the seven Date nodes
- [DataFusion & SQL Analytics](/topics/datascience/datafusion/) — querying tables that hold timestamps
- [Data Studio](/apps/data-studio/) — creating and inspecting the tables described above
