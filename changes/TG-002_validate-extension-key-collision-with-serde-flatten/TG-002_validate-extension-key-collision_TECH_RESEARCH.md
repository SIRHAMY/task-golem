# Tech Research: Validate Extension Key Collision with serde(flatten)

**ID:** TG-002
**Status:** Complete
**Created:** 2026-02-24
**PRD:** ./TG-002_validate-extension-key-collision_PRD.md
**Mode:** Light

## Overview

Research how to validate extension keys in a Rust struct that uses `#[serde(flatten)]` with a `BTreeMap<String, serde_json::Value>`. We need to ensure extension keys start with `x-` and don't collide with known Item field names. The key questions: what validation pattern to use, where to integrate it, and how to detect drift between a hard-coded field name list and the actual struct.

## Research Questions

- [x] What are common patterns for validating serde(flatten) map keys in Rust?
- [x] How do other Rust projects handle extension field validation with serde?
- [x] How to detect drift between a hard-coded known-field list and the actual struct at test time?
- [x] Where exactly in the codebase should validation be integrated?

---

## External Research

### Landscape Overview

The problem of validating keys in a `#[serde(flatten)]` map is well-known in the Rust/serde ecosystem. There is no built-in serde attribute for post-deserialization validation (a long-requested feature — see serde issues #642 and #939), so projects use workaround patterns. The most directly analogous real-world precedent is the OpenAPI specification's `x-` extension convention, which the `openapiv3` Rust crate implements using `serde(flatten)` with a custom `deserialize_with` function that filters keys by prefix.

### Common Patterns & Approaches

#### Pattern 1: Post-Deserialization Validation Function (Recommended)

**How it works:** Deserialize normally with `#[serde(flatten)]`, then call a validation method on the resulting struct. The method iterates the extensions map and checks each key.

**When to use:** When you want simplicity and control all deserialization call sites.

**Tradeoffs:**
- Pro: Simplest to implement; no custom Deserialize impl needed
- Pro: Validation logic is a regular Rust method, testable in isolation
- Pro: Easy to provide rich, actionable error messages
- Con: Requires discipline to call validation at every deserialization site
- Con: Struct can briefly exist in invalid state between deserialization and validation

**References:**
- [serde issue #642 - Finalizer attribute proposal](https://github.com/serde-rs/serde/issues/642) — long-standing request for post-deser validation
- [serde issue #939 - Validate attribute proposal](https://github.com/serde-rs/serde/issues/939)

#### Pattern 2: Custom `deserialize_with` on the Flattened Field

**How it works:** Use `#[serde(flatten, deserialize_with = "...")]` with a custom function that filters/validates keys during deserialization. The `openapiv3` crate does this with a `PredicateVisitor` that only accepts `x-`-prefixed keys.

**When to use:** When you want to filter/reject bad keys during deserialization itself.

**Tradeoffs:**
- Pro: Invalid keys never enter the struct
- Con: More complex (custom visitor, predicate infrastructure)
- Con: Harder to provide rich error messages
- Con: Only sees keys serde didn't route to struct fields

**References:**
- [`openapiv3` on GitHub](https://github.com/glademiller/openapiv3) — real-world `x-` extension handling
- [Rust forum: Deserialize flatten map with ignored fields](https://users.rust-lang.org/t/deserialize-flatten-map-with-ignored-fields/68908)

#### Pattern 3: `#[serde(try_from)]` Wrapper

**How it works:** Define an intermediate unchecked struct, use `#[serde(try_from = "UncheckedItem")]`, implement `TryFrom` with validation.

**When to use:** When you want deserialization to fail automatically without callers remembering to validate.

**Tradeoffs:**
- Pro: Invalid structs never constructed via deserialization
- Con: Requires duplicating struct definition
- Con: Does not catch programmatic construction issues

**References:**
- [DEV.to: Validate fields in serde with TryFrom](https://dev.to/equalma/validate-fields-and-types-in-serde-with-tryfrom-c2n)
- [serde: try_from container attribute](https://serde.rs/container-attrs.html)

#### Pattern 4: Serialize-and-Compare for Drift Detection

**How it works:** In a unit test, serialize a default Item to JSON, extract top-level keys, compare against the hard-coded known-field list. Test fails if they diverge.

**When to use:** To prevent the hard-coded field name list from drifting when struct fields are added/removed.

**Tradeoffs:**
- Pro: Simple, no extra dependencies, tests actual serialization behavior (including any `#[serde(rename)]`)
- Pro: Uses only existing dependencies (serde_json)
- Con: Only catches drift when tests run (not compile-time)

**References:**
- [serde issue #1110 - Get struct field names](https://github.com/serde-rs/serde/issues/1110)

### Technologies & Tools

| Tool / Crate | Purpose | Pros | Cons | Relevant |
|---|---|---|---|---|
| **serde (core)** | Serialization framework | Already a dependency | No built-in post-deser validation | All patterns |
| **serde_json** | JSON serialization | Already a dependency; `to_value()` useful for drift test | N/A | Drift detection |
| **openapiv3** | OpenAPI v3 parsing | Real-world precedent for `x-` + flatten | Reference only, not a dependency | Pattern 2 |

No new dependencies are needed. Everything is achievable with existing `serde` and `serde_json`.

### Common Pitfalls

| Pitfall | Why It's a Problem | How to Avoid |
|---------|-------------------|--------------|
| `serde(flatten)` silently routes known keys to struct fields | Post-deser validation can't detect known-field collisions from JSON — only catches programmatic construction errors | PRD already accounts for this; known-field collision check catches programmatic use |
| `serde(flatten)` uses `deserialize_map` internally | The `FIELDS` constant extraction trick doesn't work reliably | Use serialize-and-compare for drift detection instead |
| Forgetting to call validation at new deser sites | Invalid data slips through | Only 2 call sites (read_active, read_archive); manageable |
| `#[serde(rename)]` causing field name mismatch | Hard-coded list wouldn't match JSON keys | Not currently used on Item; drift test uses serialized keys to catch this |
| Case-sensitive key matching | `"Status"` (capital S) would enter extensions | PRD explicitly scopes as case-sensitive, matching serde behavior |

### Key Learnings

- The post-deserialization validation approach (Pattern 1) is the standard solution in the Rust ecosystem for this problem
- The `openapiv3` crate is the closest real-world precedent and uses the same `x-` convention from the OpenAPI spec
- No new dependencies are needed; serialize-and-compare is the established drift detection approach
- serde's lack of built-in post-deser validation is a known gap (issues #642, #939) — the workaround is well-understood

---

## Internal Research

### Existing Codebase State

The Item struct in `src/model/item.rs` uses `#[serde(flatten)]` on a `BTreeMap<String, serde_json::Value>` extensions field (line 48). Currently, the only `x-` prefix validation is in the CLI layer via `extensions::parse_dot_path()`. No post-deserialization validation exists at the model or store layer.

**Relevant files/modules:**

- `src/model/item.rs` (115 lines) — Item struct with all fields, `validate_title()` method, `apply_*` status transition methods, comprehensive serde round-trip tests
- `src/model/extensions.rs` (356 lines) — `parse_dot_path()` with `x-` prefix enforcement, dot-path parsing, nested extension manipulation
- `src/store/jsonl.rs` (376 lines) — `read_active()` (fail-fast, lines 18-58) and `read_archive()` (lenient, lines 61-117), JSONL line-by-line parsing after schema header
- `src/errors.rs` (92 lines) — `TgError` with `InvalidInput(String)` and `StorageCorruption(String)` variants
- `src/cli/commands/add.rs` (105 lines) — Uses `extensions::apply_sets()` at line 64
- `src/cli/commands/edit.rs` (125 lines) — Uses `extensions::apply_sets()` at line 103

**Existing patterns in use:**

- **Validation methods**: `Item::validate_title()` takes a value, returns `Result<(), TgError>` — the new `validate_extensions()` should follow this pattern
- **Error handling**: `TgError::InvalidInput` for user-facing validation, `TgError::StorageCorruption` for data integrity
- **Deserialization error handling**: Active store fails fast, archive skips with warning
- **Hard-coded constants with tests**: No proc macros or reflection; explicit testable implementations
- **Test structure**: Unit tests in `#[cfg(test)]` modules, integration tests in `/tests/` with `TestProject` harness, `make_test_item()` helpers

### Reusable Components

- `TgError::InvalidInput(String)` and `TgError::StorageCorruption(String)` — existing error types
- `make_test_item()` in `item.rs` tests — creates an Item with extensions, suitable for drift test
- `extensions::parse_dot_path()` — reference for error message style

### Constraints from Existing Code

- `serde(flatten)` must remain (PRD out-of-scope constraint)
- Active store uses `StorageCorruption` for hard errors; archive uses `eprintln!()` warnings and skips
- Item fields confirmed: `id`, `title`, `status`, `priority`, `description`, `tags`, `dependencies`, `created_at`, `updated_at`, `blocked_reason`, `blocked_from_status`, `claimed_by`, `claimed_at`
- No `#[serde(rename)]` attributes on Item fields — field names serialize as-is

---

## PRD Concerns

| PRD Assumption | Research Finding | Implication |
|----------------|------------------|-------------|
| Post-deser validation catches known-field collisions | `serde(flatten)` routes known field names to struct fields during deser, so they never reach extensions | The known-field collision check in `validate_extensions()` only catches programmatic construction errors, not JSON-level collisions — PRD already documents this correctly |
| Hard-coded field name list with drift test | Serialize-and-compare is the standard Rust approach; no compile-time alternative exists | PRD approach is validated; use `serde_json::to_value()` and compare keys |
| `validate_extensions()` on `Item` struct | Follows existing `validate_title()` pattern in the codebase | Natural fit; no design concerns |

No concerns raised — the PRD's approach is well-aligned with both external best practices and internal codebase patterns.

---

## Critical Areas

### Validation Check Ordering

**Why it's critical:** The `x-` prefix check and known-field collision check overlap — any key that matches a known field name also doesn't start with `x-`. The order matters only for error message clarity.

**Why it's easy to miss:** Both checks could fire for the same key. The implementation should check known-field collision first (more specific error) before the `x-` prefix check (more general error), or it doesn't matter since known fields don't start with `x-`.

**What to watch for:** Actually, since none of the known field names start with `x-`, both checks are mutually exclusive in practice: a known-field-name key (e.g., `"status"`) would fail the `x-` prefix check first. To give the more specific error message, check known-field collision before `x-` prefix. Alternatively, check `x-` prefix first and only check known-field collision for `x-`-prefixed keys — but no known field starts with `x-`, so this would never trigger the known-field check. The recommended order: check known-field collision first, then `x-` prefix.

---

## Deep Dives

_No deep dives conducted — light mode research was sufficient for this small, well-scoped change._

---

## Synthesis

### Open Questions

| Question | Why It Matters | Resolution |
|----------|----------------|------------|
| Should validation report all violations or fail on first? | Multiple invalid keys in one item could benefit from reporting all | PRD doesn't specify; fail-fast (first violation) is simpler and consistent with existing error handling patterns. If an item has multiple bad keys, the user fixes one at a time. |
| Which error to show when a key matches a known field AND lacks `x-` prefix? | Error message clarity | Check known-field collision first for a more specific error. In practice, no known field starts with `x-`, so the checks are effectively disjoint. |

### Recommended Approaches

#### Validation Pattern

| Approach | Pros | Cons | Best When |
|----------|------|------|-----------|
| Post-deser validation method (Pattern 1) | Simplest; matches PRD; follows existing codebase patterns; no new deps | Must remember to call at deser sites | You have few, well-defined deser call sites (we have 2) |
| Custom deserialize_with (Pattern 2) | Stronger invariant; keys filtered at deser time | More complex; harder error messages; PRD explicitly excludes custom deser | You have many deser sites or need absolute prevention |
| serde(try_from) wrapper (Pattern 3) | Automatic validation on deser | Struct duplication; doesn't catch programmatic construction | You need guarantee across arbitrary deser call sites |

**Initial recommendation:** Pattern 1 (post-deser validation method). It matches the PRD design, follows existing codebase patterns (`validate_title()`), requires no new dependencies, and is trivial to implement and test. With only 2 call sites, the "must remember to call" downside is negligible.

#### Drift Detection

| Approach | Pros | Cons | Best When |
|----------|------|------|-----------|
| Serialize-and-compare (Pattern 4) | Simple; uses existing deps; tests actual JSON keys | Test-time only, not compile-time | You have serde_json already (we do) |
| serde FIELDS extraction | Gets actual field names from generated code | Fragile with flatten; relies on serde internals | Struct doesn't use flatten |

**Initial recommendation:** Serialize-and-compare. Create a test Item, serialize to JSON with `serde_json::to_value()`, extract non-`x-`-prefixed keys, assert they equal the `KNOWN_FIELDS` constant. Simple, reliable, uses only existing dependencies.

### Key References

| Reference | Type | Why It's Useful |
|-----------|------|-----------------|
| [serde: attr-flatten docs](https://serde.rs/attr-flatten.html) | Official docs | Documents flatten behavior and constraints |
| [serde issue #642](https://github.com/serde-rs/serde/issues/642) | Issue | Context on why post-deser validation isn't built into serde |
| [serde issue #1110](https://github.com/serde-rs/serde/issues/1110) | Issue | Discussion of extracting field names from serde structs |
| [openapiv3 crate](https://github.com/glademiller/openapiv3) | Code | Real-world precedent for `x-` extension keys with serde(flatten) |
| [OpenAPI Specification v3.1.0](https://spec.openapis.org/oas/v3.1.0.html) | Standard | Origin of the `x-` extension convention |
| [DEV.to: serde TryFrom validation](https://dev.to/equalma/validate-fields-and-types-in-serde-with-tryfrom-c2n) | Tutorial | Alternative pattern (Pattern 3) for reference |

---

## Research Log

| Date | Activity | Outcome |
|------|----------|---------|
| 2026-02-24 | Light external research (patterns, pitfalls, standards) | 4 patterns identified; Pattern 1 recommended; no new deps needed |
| 2026-02-24 | Light internal research (codebase exploration) | Integration points identified; existing patterns confirm PRD approach |
| 2026-02-24 | PRD analysis and synthesis | No concerns; PRD is well-aligned with research findings |

## Assumptions

- **Autonomous research**: Conducted without human interview. Mode defaulted to light given small size, low complexity assessments.
- **Fail-fast on first violation**: Chose fail-on-first-error over collect-all-errors for simplicity and consistency with existing patterns.
- **Check order**: Recommend checking known-field collision before `x-` prefix for more specific error messages, though in practice the checks are disjoint (no known field starts with `x-`).
