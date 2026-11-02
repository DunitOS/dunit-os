# Virtual tree views (M7.6)

`View::tree_view` extends the shared virtual collection controller with a
validated hierarchy, controlled expansion, spatial navigation and disclosures.
The existing `View::tree()` method still builds the DUI layout tree; it is not
renamed or replaced by this constructor. No kernel or gui-v1 change is required.

```rust
let model = TreeModel::new(vec![
    TreeItem::new("workspace", "Workspace"),
    TreeItem::new("src", "Sources").under("workspace"),
    TreeItem::new("main", "main.rs").under("src"),
]).expect("valid application model");
View::tree_view("project", "Project", model.clone(), state.clone(),
    SelectionMode::Multiple, Msg::TreeState)
    .layout(Attrs {
        width: Sizing::Grow(1.),
        height: Sizing::Grow(1.),
        ..Default::default()
    })
```

## Model and controlled state

`TreeItem` contains a stable-key ListItem and an optional stable parent key.
Parents can appear after children in the supplied vector. Roots and siblings
follow input order; visible traversal is depth-first preorder. `TreeModel::new`
rejects empty/duplicate keys, missing parents, cycles (including disconnected
cycles), and nesting beyond 64 levels. It validates hidden branches too. Empty
models are valid. The immutable model shares validated graph data through Rc,
so cloning it into the next View does not clone every node or rebuild edges.
This cache contains model data, not per-row UI nodes.

`TreeState` owns `selection: ListSelection` and `expanded: BTreeSet<String>`.
The callback receives one complete proposed state, which the model accepts or
rejects. Selection and expansion have no hidden independent controller state.
Unchanged proposals do not dispatch messages. Expanded IDs must name existing
branches; selected/current/anchor IDs must name visible enabled rows. Invalid
view state fails validation rather than silently dropping user data.

For external collapse/removal/disable updates, `state.retain(&model)`:

- removes invalid/leaf expansion keys;
- removes hidden, removed or disabled selections;
- moves a hidden current/anchor to the nearest visible enabled ancestor;
- selects that ancestor if the hidden current was selected.

A removed node has no ancestry in the new model, so its current/anchor clears.
Collapsing retains expansion flags of hidden branches, allowing their shape to
return when an ancestor reopens. Reordering preserves stable-key selection and
expansion; the shared reveal logic follows the current row's new visible index.
A disabled node disables its whole subtree, including already expanded children.
The application may still change expansion programmatically while disabled.

## Input, scope and metadata

The tree is one Tab stop. Shared collection behavior handles Up/Down,
Home/End/PageUp/PageDown, single/multiple/range selection, Shift/Ctrl navigation,
Space and Ctrl+A. Ranges and Select All include only visible enabled rows.
No hidden descendant receives input or becomes a selection endpoint.

Unmodified Right expands a collapsed branch; on an expanded branch it selects
the first enabled direct child. Left collapses an expanded branch; otherwise
it selects the visible parent. At a root/leaf endpoint, or with no current row,
spatial commands are consumed without a callback. `Application::tree_command`
exposes Expand/Collapse/Toggle; plain Left/Right also use Application::navigate.

A primary Down in the disclosure toggles expansion while preserving selection,
except for normalization of hidden descendants. Clicking the label/body uses
normal selection and does not expand. Release does not redispatch. Disclosures
use the same geometry for painting and hit testing. Native pointer events have
no modifier bits; multiple/range pointer adapters may use the existing explicit
`list_pointer` modifiers, while native keyboard Shift/Ctrl works normally.
Double-click activation, range dragging, typeahead and lazy data loading are not
part of this slice.

Modal and popup scopes, focus restore, background shortcut blocking and disabled
input use the existing shared machinery. A tree inside a toolbar is rejected
like other non-command collection controls.

The root exposes role `tree`, supplied label and multiselectable metadata.
`tree_item(key)` reports a visible row's `treeitem` role, supplied label, parent,
1-based level/position, sibling count (including disabled siblings), expanded
state for branches, inherited disabled state, selected and current. Hidden rows
are absent from this view metadata. `tree_state()` reports controlled state.
These are descriptors for a future accessibility bridge, not an OS a11y service.

## Virtualization and DSS

ListGeometry, ScrollAxis, gutter/bar input, clipping, retained invalidation and
visible ListPaint snapshots are shared with lists. The tree does not build
layout boxes or View widgets for model rows. Visibility projection/validation
still visits model data; only rows intersecting viewport/ancestor/display clips
are retained as text paint rows and rasterized. This is rendering virtualization,
not a paged or asynchronous data source. `list_visible_rows` and `list_geometry`
on Renderer/ComponentWindow observe that bound for either collection kind.

DSS supplies the common list row height, selection/current/disabled colors,
font, padding, border/focus and scrollbar properties. Trees additionally use
`tree-indent`, `tree-toggle-width`, `tree-open-mark` and `tree-closed-mark`.
The two profiles provide indent and width; marks live in the shared DSS rule.
No appearance constants or manual hit tests are required in application code.
Long labels clip horizontally; rows have uniform height. Resize clamps scroll
and reveals current using the same controller as lists.

## Verification

Fourteen host scenarios cover graph validation/depth, arbitrary parent order,
sibling metadata, disabled inheritance, selection/expansion/navigation,
rejection, collapse promotion, model replacement, modal/popup isolation, empty
and leaf trees, 10,000 descendants, shared disclosure geometry, and retained
pixels/resize in both themes. Existing SDK suites run as regressions.

```sh
cargo test --manifest-path runtime/dunit-component/Cargo.toml --test tree
python3 tools/test_sdk_gui_fixture.py tree --accel tcg
```

One native showcase drives actual Tab/arrows/Shift, a root disclosure click,
wheel and window resize; it verifies nested levels, disabled skip, large-branch
virtualization and End reveal, collapse promotion/clamp and rejected expansion.
The marker contract includes clean GUI exit and the existing DF entry regression.
It captures hierarchy/selection, the final file and the collapsed workspace.
[Tables and sorting](tables.md) and controlled in-place renaming are implemented
on the same collection foundation. See the [M7.6 acceptance matrix](m7-6-completion.md)
for their contracts and final verification status.
