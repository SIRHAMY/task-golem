# Change Design: Canonical UUIDv7 identity and atomic graph application

**Contract:** hamy-change-design/v6
**Change:** 2026-08-29_tg-graph-store-pg-workflows
**Version:** 1.0
**Status:** Ready
**Foundation version:** 3.0
**Foundation fingerprint:** bccd4078bedbeba0cd7160ad4f35076c794e96cd527fea2b4bcd44396b7e373e
**RFC version:** 2.0
**RFC fingerprint:** c1b5994147a42f73db575631f14409e6a15518fb861f54854cd1ebbc062dbf2d

## Scope

This Design fixes Task Golem's canonical item identity and one generic, create-only graph application operation. It covers the public library/in-process request and result types, exact ID validation and lookup, request-local reference resolution, proposed-graph validation, structured diagnostics, the store lock and JSONL commit boundary, event behavior, and derived cache invalidation.

It does not design Phase Golem behavior, metadata query indexing, archived dependency completion rules, broader item CRUD, migration or compatibility, a network API, or workflow semantics. It consumes the RFC's archived-dependency eligibility without elaborating its representation or shared evaluation path. It does not persist application keys, request digests, apply receipts, or request-local references.

## Design

### Canonical identity

TG keeps `Item.id` as a string on the existing JSONL wire and model shape, but every newly created value is a canonical textual UUIDv7. Canonical means exactly 36 lowercase ASCII characters in the `8-4-4-4-12` hyphenated form. Parsing must confirm the UUID version nibble is `7` and the RFC variant bits are `10`. UUID parsers that accept braces, URNs, compact forms, uppercase, or other alternate spellings are not lookup validation.

Generation uses the UUID library's UUIDv7 generator. It checks the active IDs, archived IDs, and IDs already allocated in the current request before accepting each value. A collision is regenerated up to a bounded retry limit, after which the operation fails before writing; the caller never supplies a new item ID.

The ID module exposes canonical generation, validation, and exact resolution. Resolution first validates the complete canonical input, then performs an exact match in the requested active/archive scope. It never adds `tg-`, strips a prefix, performs prefix matching, accepts shortened or legacy IDs, or searches mixed formats. A well-formed but absent ID is `item_not_found`; a malformed, non-v7, prefixed, shortened, or non-canonical ID is `invalid_id`. The `id_prefix` and `id_len` configuration fields and the prefix-based generator export are removed rather than ignored.

### Apply contract

The library and in-process entry point is a single `Store::apply_graph(GraphApplyRequest)` operation. The public library surface re-exports `GraphApplyRequest`, `GraphApplyItem`, `GraphRef`, and `GraphApplyResult` alongside canonical ID generation, validation, and resolution. The CLI `tg apply` reads one JSON `GraphApplyRequest` from stdin and, with structured output enabled, emits the same result or error contract. New items contain no caller-supplied durable IDs; only `existing` edge targets carry durable UUIDs:

```json
{
  "items": [
    {
      "ref": "root",
      "title": "Root",
      "description": null,
      "priority": 0,
      "tags": [],
      "parent": null,
      "dependencies": [],
      "extensions": {"x-owner": "opaque value"}
    }
  ]
}
```

`items` must contain at least one item. `GraphApplyItem` has one unique, non-empty, single-line `ref`, the generic create fields `title`, `description`, `priority`, `tags`, `parent`, `dependencies`, and `extensions`, and no `id`, timestamps, status, claim, or blocked-state fields. TG creates every item as `todo`, unclaimed, and unblocked. It captures one UTC timestamp for the application and uses it for every new item's `created_at` and `updated_at`. Existing `Item` title and extension validation remains authoritative; extension keys remain `x-` keys and are persisted as opaque flattened metadata.

Every edge target is explicitly tagged, so a local symbol cannot be mistaken for an ID:

```json
{"local": "other-node"}
{"existing": "018f2b1c-4d5e-7abc-8123-456789abcdef"}
```

`local` must name one item in this request, exactly as written. `existing` must be a full canonical UUIDv7 and must exist in durable TG state. Local references have request scope only. They are not serialized into an `Item`, are not accepted by later lookup, and do not survive a successful call.

Success is a `GraphApplyResult` with a stable operation/outcome envelope and a complete, lexicographically ordered mapping from every request reference to its new UUID:

```json
{
  "operation": "graph_apply",
  "outcome": "created",
  "count": 1,
  "mapping": {
    "root": "018f2b1c-4d5e-7abc-8123-456789abcdef"
  }
}
```

The mapping contains only newly created items. Existing targets are anchors, not affected items. Repeating an identical valid request is non-idempotent: each successful call returns a different mapping and creates a disjoint graph. There is no update mode in this operation. Changing an existing item, including its edges, remains an item CRUD operation.

### Generate, resolve, validate

After request deserialization has rejected unknown or structurally impossible fields, the operation admits a readable, internally consistent active/archive snapshot, then runs these semantic phases inside one write-lock callback. Snapshot admission rejects duplicate or non-canonical durable IDs before the phases begin; it does not allocate or write anything.

1. Generate one fresh UUIDv7 per request item in request order and reserve each allocation locally against the admitted known-ID set.
2. Resolve every `local` target through the request mapping and every `existing` target through exact durable lookup. Preserve resolved dependency order, but reject duplicate dependencies after resolution. Reject an empty graph, duplicate item refs, missing local refs, missing existing IDs, and self-references. A parent has one target; it follows the same reference rules.
3. Construct the proposed active graph in memory and validate all item state, parent edges, dependency edges, and both complete graph invariants before committing. A failure in any phase discards the local allocations and proposed items.

Parent targets must be active items or proposed new items; archived parent targets remain invalid under the existing parent invariant. Dependency targets may be active items, archived items with preserved completion evidence, or proposed new items. Existing active items participate in proposed parent and dependency cycle checks, including their pre-existing edges. Archived dependency targets are terminal anchors for this validation and do not add active graph edges. Existing items and their events are read-only throughout.

The cycle checks use the proposed graph, not one-edge-at-a-time checks. Parent and dependency graphs remain independent. Each cycle is normalized by rotating it to its lexicographically smallest member, and normalized cycles are sorted before diagnostics are emitted. This makes cycles that cross an existing anchor deterministic without relying on hash iteration order.

### Diagnostics and ordering

Graph failures use a typed `graph_apply` error with `category` and an ordered `diagnostics` array. Each diagnostic contains a stable snake-case `code`, a source path such as `items[2].dependencies[0]`, and structured reference/detail fields. Request-local failures identify local refs and source indexes rather than exposing discarded generated UUIDs. The categories and core codes are:

- `invalid_request`: `empty_graph`, `duplicate_reference`, `missing_reference`, `invalid_id`, `self_reference`, `duplicate_dependency`, `invalid_item`
- `invalid_graph`: `parent_cycle`, `dependency_cycle`
- `storage_corruption`: `duplicate_durable_id`, malformed durable state, or an invalid existing graph anchor
- `persistence_failure`: the atomic task-store write failed

Validation diagnostics are sorted by source path, then code, then canonicalized reference/detail values. Cycle diagnostics use the normalized cycle order above. Storage and persistence failures contain one stable diagnostic and never return a partial mapping. JSON CLI errors include `operation`, `outcome: "error"`, `category`, `diagnostics`, and the existing exit-code field. Library and in-process callers receive the same typed categories and details without parsing human text.

### Atomic store behavior

`apply_graph` holds `Store::with_lock` from snapshot load through commit. It does not call item-level create functions or perform per-item writes. It builds a proposed active vector, then calls the existing JSONL `write_atomic` path exactly once after all validation succeeds. That path writes a temporary `tasks.jsonl`, fsyncs it, and atomically renames it into place. No durable mutation occurs before that rename.

Graph creation emits no event: the current event model records status transitions and notes, not item creation. Successful apply therefore changes only `tasks.jsonl`; `archive.jsonl`, both event files, and existing anchor records remain unchanged. Invalid requests, storage validation failures, and an injected pre-rename persistence failure leave all of those durable files unchanged.

The SQLite cache remains derived state and is not written as part of graph application. A successful task-file rename changes its freshness stamp; the next cache open detects staleness and rebuilds from the authoritative JSONL under the cache module's existing lock-scoped read and atomic cache replacement flow. Failed applies do not change the stamp and do not rebuild or mutate the cache.

## Verification

- Generate a large sample of IDs and prove every value is lowercase canonical `8-4-4-4-12`, UUIDv7, RFC-variant, unique, and time-orderable; prove exact active and archive lookup succeeds while prefixed, shortened, uppercase, alternate-format, malformed, non-v7, and mixed-format inputs return `invalid_id`.
- Apply a valid multi-level request using local parent/dependency refs, an active existing anchor, archived dependency evidence, tags, and opaque extensions; prove the result maps every local ref exactly once, all stored edges contain UUIDs, and all new items are `todo` and unclaimed.
- Reject duplicate refs, duplicate dependencies, missing local/existing refs, self-references, malformed item fields, duplicate durable IDs, parent cycles, and dependency cycles; compare active, archive, event, and cache state before and after each rejection.
- Inject failure at the JSONL atomic-write seam and prove the call returns `persistence_failure` with no partial items, edges, metadata, events, archive changes, or cache changes.
- Submit one valid request twice and prove both calls succeed, their mappings are disjoint, and each graph is complete; prove no application key, request digest, receipt, or request-reference lookup is accepted or persisted.
- Include existing active graph anchors in cycle fixtures and prove their item bytes/meaning, edges, timestamps, events, and status remain unchanged after a successful apply; prove an attempted existing-item update is rejected and item CRUD remains the separate update path.
- Verify the structured library, in-process, and `tg apply` success/error shapes have identical fields and deterministic diagnostic ordering, and verify a successful task write makes the derived cache stale without directly mutating it.
