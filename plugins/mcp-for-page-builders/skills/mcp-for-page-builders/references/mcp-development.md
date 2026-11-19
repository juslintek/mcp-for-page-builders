# MCP architecture and API development

Use this when changing WordPress/Elementor tools. Read focused implementations and tests; `src/tools/mod.rs` registers tools and `src/lib.rs` wires the protocol. Keep discovery read-only, validate trust boundaries, and follow current error/result conventions. Planning, compilation, runtime behavior, and deployed behavior are separate evidence levels.

## Native API and mutation path

Use a native ability or REST interface only after checking registration, availability, permission, input/output schema, target version, and actual behavior. A schema accepted by the bridge does not prove that Elementor's runtime transformer exists or emits the expected markup. The Core MCP module may be absent, an ability may return `WP_Error`, and the bridge can call only abilities it exposes. Internal compiler code is not a public typed API.

Keep inspection and planning read-only. Validate the canonical site, document revision, operation ID, resource revisions, identifiers, and full plan before mutation. `apply_elementor_plan` accepts a complete canonical `document_patch`, optional revision-bound resource patches, retained-element explanations, source, and requested checks. Writes can partially commit before verification fails. Read the journal and saved state, then use the returned restore pointer. Document and every resource need their own before snapshot and readback.

| Tool | Input and result | Boundary |
|---|---|---|
| `inspect_elementor_capabilities` | No arguments; capabilities, runtime versions/routes, style schema. | Read-only. Check availability and permission on the connected site. |
| `get_widget_schema` | Required `widget_type`; uses native `element_schema` when bridge capabilities are reachable, otherwise `SchemaRegistry`. | Registry fallback is not the connected site's live schema. |
| `elementor_runtime` | `capabilities`, `runtime`, `style_schema`, or `widget_schema`; the last action requires `widget_type`. | Read-only bridge inspection; distinct from the MCP tool named `get_widget_schema`. |
| `css_to_elementor` | `css`, `widget_type`, optional `target` (`classic` or `atomic`). | Read-only conversion. Atomic props are styles, not content settings. |
| `plan_css_migration` | `page_id`, `operation_id`, target, CSS text or source URL, optional selector/target/widget. | Read-only proposal; CSSOM/CDP does not prove complete cascade capture. |
| `convert_elements` | `page_id`, `operation_id`, direction, optional `element_ids`. | Read-only plan. Selected IDs must be valid, unique, and non-overlapping; output still contains the full document. |
| `elementor_design_system` | `page_id`, `operation_id`, optional resource patches. | Read-only plan; classes and variables are collection-scoped. Validates the complete batch before page or resource reads. |
| `extract_elementor_patterns` | `page_id`, `operation_id`. | Read-only proposal; produces portable templates, not synchronized Components. |
| `apply_elementor_plan` | Site, page, expected revision, operation ID, optional full document/resource patches, retained items, source, checks. | Mutation boundary; partial writes remain possible. |
| `restore_elementor_snapshot` | Generic document revision or native editor-state fingerprint. | Mutating recovery, guarded by revision/fingerprint and readback. |
| `laya_decide_elementor` | Bounded task, state, question, unique options. | Local read-only recommendation; cannot validate or authorize writes. |

For a document edit, fetch the complete tree from `get_page`, inspect its revision and live schemas, review the read-only plan and all residual/retained entries, then submit the full tree. `document_patch` is not a partial merge patch; an empty array clears the tree. Selected `element_ids` convert disjoint subtrees while preserving the rest of the complete tree; invalid, duplicate, missing, or parent/child-overlapping selections fail before planning. If a selected subtree cannot be represented, no executable patch is returned. Resource-only plans bind to the inspected source-document revision. A combined document/resource plan uses the checked-save receipt revision, not a later foreign GET. Each resource `after` is its complete value and must carry the inspected revision and `before` snapshot.

Existing resource kinds are `class`, `variable`, `kit`, `page_settings`, `template`, and `post_metadata`. Write IDs are scoped: classes/variables require `collection`, kit requires `settings`, page settings use a positive numeric document ID, and templates use a positive numeric ID or `new` / canonical `new_pattern_N` with N at least 2. Component discovery requires an action, an available and permitted ability, and a nonempty object output schema with properties. Discovery does not imply an executable sync adapter. Synchronized Components remain unavailable until that adapter is checked. Protected generic option writes reject Elementor-managed prefixes. Unsupported experiments must fail before writing until a checked adapter exists.

## Serialization and conversion

Keep content settings, Atomic style props, dynamic bindings, resource records, and generated CSS as distinct typed boundaries. Use canonical editor exports, native parsers, resolvers, and a real guest render. Check controls, breakpoints, states, child restrictions, and transformer availability against the active target. Classic Containers use their actual `css_classes` field; do not reuse Atomic class serialization.

Atomic local typed classes include ordinary semantic CSS strings and node-local style IDs, not only global references. Keep local style IDs unique per node and bind them to matching typed classes. Pattern signatures alpha-normalize only valid bound local style IDs and definition keys; they retain global class IDs, declarations, state/breakpoint/class ordering, raw custom CSS, custom logic, and unsafe references. Ordinary object-key order is canonicalized. Navigator titles and style labels are normalized. Pattern extraction remains a portable-template proposal and does not sync later edits.

Preserve responsive variants, pseudo-states, descendants, cascade order, source provenance, classes, attributes, links, media, and dynamic bindings. Version-gate links, media, attributes, and dynamic values. If any logic cannot be represented, retain the whole affected subtree. Classic URL flags follow PHP `!empty` semantics for supported scalars. Atomic links do not represent active `nofollow` or nonempty `custom_attributes`; retain those nodes. Unknown objects and arrays are not booleans.

Atomic native `styles` maps use definition insertion order because order can determine the winning declaration. Bridge fingerprints preserve order only for verified native style maps whose keys match each definition ID; ordinary object fields remain canonical. `native-style-order-v1` fingerprints protect new editor-guard and resource-plan requests. Historical non-style fingerprints remain compatible; historical receipts containing ordered native styles are rejected rather than replayed under an ambiguous hash.

## Components and version gates

Do not treat route presence, plugin activation, `manage_options`, or a generic ability response as proof that a Components action is allowed. Core 4.2.3 exposes Components routes only when `e_components` and `e_atomic-widgets` are active. Its list is shallow and capped at 100, does not return component trees, and the overridable-props GET can realign legacy metadata, so it is not a harmless probe. `Components_Access_Controller` requires active Pro for create and allows edit with active or expired Pro.

Core 4.3.3 source includes native create/update/rename/archive/publish actions with source copying and ID remapping. Its ZIP source is not runtime proof: verify the exact Core/Pro pair, registered ability, schema, entitlement, and behavior before using it. No checked native Components adapter or sync acceptance is recorded. Until then, treat templates as copies.

CSS planning can use CSSOM rules and CDP matched/computed styles at mapped widths. Imports, cross-origin failures, unsupported contexts, descendant/state selectors, specificity/order conflicts, or unobserved declarations remain residuals. CSSOM declaration counts can differ from literal source counts when shorthand properties expand. Report the method and both counts when relevant. A parsed stylesheet does not prove all cascade inputs were observed.

## Persistence and recovery

Saved props and generated CSS readback alone do not prove a native save. Compare-and-swap verification must invalidate the affected document's native HTML cache with the document cache API, refresh and warm its guest render, and compare the paired snapshot. An admin/native empty document may have no cache leaf; exempt only exact empty document data paired with an empty native render. Do not add a broad success fallback. Capture Atomic cache booleans and artifact bytes before returning. Preserve cold native CSS status (`null` or empty string) separately from a declared file. Avoid global CSS flushes.

Restore from a full paired snapshot, including document/resource state, generated CSS, locks, foreign metadata, title, and artifact files. Reject stale fingerprints or foreign changes; do not refresh stored fingerprints from arbitrary later reads. Return exact partial-failure IDs and restore outcomes.

Native fixtures must isolate unrelated state process-locally and clean up only their own artifacts. Do not toggle global print method, flush whole caches, or restore whole cache trees. Verify protected-state equality and report failed checks. A CLI process-local filter does not guard a separate browser HTTP request; name the actual isolation scope. Capture original derived-option values before changing them; hashes alone cannot safely restore unknown values.

## Current evidence

The latest frozen bridge source is `d308e3cf5f654fb7aa1d558b99e382873d40d430559c919ca2d06920cd7d1463`, byte-identical in the isolated clone. Final process-configuration and isolation checks passed, including required environment validation, local URL parsing, and DDEV project/container identity checks. Native reruns passed: Atomic CSS 2/2 (one native case and one pure configuration validator, 41.53s), serialization 3/3 (two native cases and one pure validator, 25.02s), and combined resource transactions 4/4 (60.52s). The combined cases cover ordered Atomic style replay/conflict and revision-aware restore after a committed write returned a simulated 502. These counts cover only the named targets, not all integration tests or installed-MCP acceptance.

Source planner checks passed for selected subtree conversion (27 planning tests, one tool test) and complete design-system batch validation (27 planning tests, five tool tests). These checks do not establish native conversion or pattern acceptance. Selected conversion and pattern source are currently undergoing native checks; final results are pending. The design-system planner rejects unsupported or incomplete batches, duplicate or invalid IDs, missing revisions, and requested writes when native resource transactions are unavailable, before reading page context or resource state.

The final targeted `cargo check --locked --test atomic_css --test native_elementor_serialization --test resource_transactions`, rustfmt, and pure configuration validator passed. Earlier `--lib` and `--tests` checks passed at a separate source snapshot; they do not combine into a final full-branch check. Fresh connected MCP metadata lists 83 tools; the candidate plan, conversion, and Laya interfaces are not installed or reconnected there. The separate Atomic CSS artifact-readiness integration and full installed-MCP acceptance remain pending. Do not claim broad integration coverage or deployed acceptance from these focused results.

The final native reruns confirmed the isolated clone identity and WordPress 7.1, Core 4.2.3, Pro 4.2.2. Protected post, revision, and postmeta aggregates were unchanged, and the fixture pages were removed. An initial full run generated CSS for existing posts 2 and 22414; the final rerun kept that three-file set stable. Do not claim every pre-existing CSS file was historically unchanged. The aggregate Elementor option count changed 855→861→867 and remains unclassified; no blanket restore or global flush was performed. The live target remains WordPress 7.1, Core 4.2.3, Pro 4.2.2. A post-save through Yoast 26.9 had exact intermediate readback, but `_edit_lock` drift changed the full fingerprint after reload. Stale restore returned 409 without overwriting. Keep `_edit_lock` in full comparisons. Wrong/replayed/late token, foreign metadata, timeout, and crash checks remain pending. The editor guard and persistence harness now use strict local native-test configuration, configured admin identity, exact fixture cleanup, and one session token for both browser cookies; scoped Rust formatting and compilation checks passed. Their native tests have not run at this source state.

Laya is advisory. Recorded English/Lithuanian inference and worker lifecycle checks do not establish a confidence guarantee or SLA. Inference timing excludes some startup, import, and hashing costs. Deterministic validation and user intent decide writes.

## Focused verification

Use the smallest real check for the change: a focused Rust check, the repository PHP self-check, or an isolated clone-native fixture with confirmed clone identity and owned IDs. Never skip missing infrastructure or mock away native operations. Read the complete result before updating status. See [Karjera review](karjera-review.md) for the page's live recovery and migration boundaries.

References: [Elementor MCP requirements](https://elementor.com/help/how-to-build-and-edit-your-site-using-elementor-mcp/), [Atomic data structure](https://developers.elementor.com/docs/data-structure/atomic-elements/), [Components](https://elementor.com/help/components-2/), [Pro changelog](https://elementor.com/pro/changelog/), [Laya MLX](https://github.com/mizorewww/laya-mlx).
