# Dunit static SVG runtime

A shared `no_std + alloc` SVG renderer for GUI assets. XML is parsed with
roxmltree, SVG data types and paths with svgtypes, and pixels with tiny-skia
using its no-std float implementation. No host conversion, browser, network,
font lookup or scripting engine is involved. Output is straight-alpha ARGB8888;
tiny-skia's premultiplied pixels are converted at the API boundary.

Supported static profile:

- SVG root with viewBox or dimensions; meet/slice/none aspect ratio and alignment.
- Groups, local `use` references, inherited paint, currentColor, display/visibility.
- Paths including relative commands, arcs, smooth/quadratic/cubic curves;
  rectangles with elliptical corners, circles, ellipses, lines and polygons.
- Anti-aliased fill/stroke, even-odd winding, caps, joins, numeric dash arrays
  (dash offset on the element defining the dash array), paint and group opacity.
- Solid colors and linear/radial gradients, stops, bounding-box/user-space units,
  gradient transforms and pad/repeat/reflect spread; local paint URL fallbacks
  and bounded local gradient inheritance.
- Presentation attributes, inline `style`, static class selectors with source-order
  cascade; paint-order for fill/stroke. Editor namespaces/metadata are inert.
- Single Gaussian blur over SourceGraphic, object/user filter regions and
  sRGB/linearRGB processing. Axis-aligned transforms only; sigma at most 32 device
  pixels, 1 MiPixel scratch surface and 64M convolution channel operations.

Unsupported: text/fonts, external stylesheets/complex CSS selectors, filter graphs, masks,
clip paths, nested SVG viewports/symbols, linked images,
SMIL/animation, scripts and external references. Unsupported elements or
properties return an error instead of claiming the document was fully rendered.
`use` only references local geometry/groups; its width/height are ignored as required
for those targets. Referencing svg/symbol viewports is unsupported. Neutral editor
presentation defaults are accepted; unsupported non-default effects are rejected.

Limits: 1 MiB XML, 4096 XML nodes/draw visits, depth 32, 16384 path segments,
128 stylesheet rules / 64 KiB stylesheet text, 256 stops per gradient, coordinate magnitudes through 10^7 and 12 MiPixels per
output. Nested opacity layers have a combined 24 MiPixel budget. DTD/entity
expansion and processing instructions are disabled. Cycles and invalid geometry
are rejected. Rendering returns no partial image on error.

Run the host regression suite with:

```
cargo test --manifest-path runtime/dunit-svg/Cargo.toml --target x86_64-unknown-linux-gnu
```

The same crate is linked into native Dunit GUI binaries and `svg_test`.
Dependency licenses shipped with the OS live in `assets/legal/svg-runtime`.
