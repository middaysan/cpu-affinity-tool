# Local epaint hinting compatibility patch

This is the published epaint 0.36.2 crate from crates.io, licensed under
MIT OR Apache-2.0. The original crates.io archive checksum is
`a0ec0308dc23130fc1a7d4623a8534c2ed069c6dc84e258ee4f3de99615aa1a3`. The original registry metadata, Cargo manifests and source
are retained. The license texts are copied from the same egui 0.36.2 distribution.

Only `src/text/font.rs` changes: compare two default variation locations as
equivalent before reconfiguring the existing hinting instance. Skrifa 0.44.0
stores an all-zero location as an empty slice; comparing its raw coordinates
with Inter's explicit zero axes repeated the same configuration for new glyphs.
Nondefault coordinates, size and target invalidation retain their behavior.
The patch adds no glyph cache, timer, worker or diagnostic instrumentation.

Two local unit tests cover default equivalence and nondefault invalidation.
The application also characterizes unchanged atlas pixels at three DPI scales
with regular, medium, semibold, Latin, Cyrillic and fallback glyphs.

Upstream: https://github.com/emilk/egui/tree/0.36.2/crates/epaint
Remove this copy when an upstream dependency provides the same behavior and
the local regression/pixel/CPU19 acceptance checks pass against it.
The root `[patch.crates-io]` selects this copy for both platform entrypoints; Linux keeps its Glow renderer. The registry cache is not modified, and the standalone registry lockfile/build stamp is omitted.
