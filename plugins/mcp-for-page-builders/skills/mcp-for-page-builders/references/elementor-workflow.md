# Elementor native authoring and review

Use this when authoring a page, converting Classic content or CSS, or reusing Elementor designs. Work from the connected site's document, kit, registered abilities, schemas, runtime versions, and permissions. Elementor's published MCP guide currently lists WordPress 6.8+ and Core/Pro 4.3+; that is a requirement for those features, not evidence they exist on an older target.

## Discover and choose a boundary

Confirm the site and page/template ID, fetch the complete document and revision, then inspect page settings, generated CSS, kit, breakpoints, relevant resources, and guest output. `get_element_tree` is a flattened view, not the full tree. Inspect `inspect_elementor_capabilities`, `elementor_runtime`, and relevant tool schemas. For each ability, verify registration, availability, permission, version, and input/output schema. An ability inventory does not prove its transformer or editor behavior is usable.

Prefer verified native controls and registered widgets, then verified Classes, Variables, and bindings. Use synchronized Components only when a real native adapter is available and authorized; portable templates copy content and do not sync later edits. Keep Classic where its widget or behavior has no verified Atomic mapping. Classic containers and Atomic elements have different class serialization. Atomic is a native element/property/style model, not a synonym for flexbox. Preserve unsupported nodes and their full subtrees.

## Plan and convert

Validate content settings, style props, responsive values, states, descendants, classes, attributes, links, media IDs/URLs, and dynamic bindings against the connected schema and native parser. `css_to_elementor`, `plan_css_migration`, `convert_elements`, `elementor_design_system`, and `extract_elementor_patterns` produce proposals; they do not write. Atomic `props` are style values, not widget content settings. Review every rejected or retained item and keep rules outside the verified native boundary in scoped residual CSS.

`convert_elements` can target selected, disjoint subtrees with `element_ids`; it still returns a complete canonical document proposal preserving the other nodes. It validates the full tree and every requested ID before planning. Empty, duplicate, missing, overlapping parent/child selections are invalid. If any selected subtree cannot be represented safely, no executable document patch is returned. Without `element_ids`, conversion considers the whole document. Neither form proves external selectors or stylesheets were captured.

Before a write, review the complete tree, current revision, source provenance, warnings, and retained subtrees. `document_patch` is the full canonical tree, not a partial JSON patch. Bind every resource patch to its inspected revision and complete `before` value. Keep known original global class IDs when checking deletion; local style IDs are unique per node and must match that node's typed classes. Do not remove old CSS until native save/reload and guest rendering pass.

## Save, inspect, and recover

Acceptance needs native save, editor reload, intended tree/settings, relevant UI interactions, and a fresh guest render. A 200 response, saved props, or DOM presence alone is insufficient. After document compare-and-swap, invalidate only that document's native HTML cache, refresh and warm its guest render, then compare paired state. Allow a missing cache leaf only for the exact empty-data plus empty-render case. Preserve cold CSS status (`null`, empty, or a declared file) distinctly; avoid global CSS cache flushes.

Restore only from the returned operation pointer and expected current revision/fingerprint. Compare the full paired state, including resources, generated CSS, locks, foreign metadata, title, and artifact files. Do not refresh a stored fingerprint from an arbitrary later read. Report partial writes and exact restore results.

For visual review, use paired Figma frames or exports at their native dimensions. Measure height, computed styles, boxes, overflow, interaction states, and a pixel-diff bounding box or percentage. Do not resize or crop the source reference; if a secondary crop helps inspect detail, retain the full-frame comparison and measurements. The current Karjera live recovery passed 24/24 checks at eight widths, but raw differences were 18.1752% desktop and 20.5656% mobile, so it is not pixel parity. See [Karjera review](karjera-review.md) for current live, isolated-clone, and migration status.

Report evidence by stage: source checks, an isolated native fixture, an actual page save, an installed-MCP connection, an exact-version upgrade, and a deployed release establish different facts. Do not promote evidence from one stage to another.

References: [Elementor MCP guide](https://elementor.com/help/how-to-build-and-edit-your-site-using-elementor-mcp/), [Atomic data structure](https://developers.elementor.com/docs/data-structure/atomic-elements/), [Components](https://elementor.com/help/components-2/), [Pro changelog](https://elementor.com/pro/changelog/).
