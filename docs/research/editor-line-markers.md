# Editor line markers: Git gutter bars and overview-ruler marks in mature editors

Researched 2026-10-27 for an upstream GPUI Kit PR that adds (1) Git change markers in the code editor gutter and (2) markers on the vertical scrollbar track. Sources are pinned to these commits:

| Project | Commit | License |
| --- | --- | --- |
| VS Code / Monaco (`microsoft/vscode`) | `4bda7217fc83e8e838b1edd9ebaf21bb9b498c7e` | MIT |
| IntelliJ Platform (`JetBrains/intellij-community`) | `43722b268dbc6359441135e1895a63d0fbb15fca` | Apache-2.0 |
| Zed (`zed-industries/zed`) | `ec86047d67acd6fddd81392f44ed6ebcc6e7f531` | `crates/editor` and `crates/buffer_diff` are GPL-3.0-or-later. This report describes their approach and API shape only and quotes no code. |
| CodeMirror 6 (`codemirror/view`, `codemirror/merge`) | `fbff59ba004d80d8c914f64c42586387b08706ac`, `3bc269331394691dcce9784ba8c93b318d15535f` | MIT |
| GPUI Kit (`longbridge/gpui-kit`) | crates.io `gpui-base` 0.8.0, and `main` at `64e6aaffbcd3760fc9d000f86655495c4978931c` | Apache-2.0 |

## Headline

Monaco models both surfaces as optional facets of a single tracked decoration. VS Code's quick diff sets the gutter facet (`linesDecorationsClassName`) and the ruler facet (`overviewRuler`, `OverviewRulerLane.Left`) on the same decoration for each hunk. The other mature editors split the work by surface:

- **IntelliJ** could put both on one `RangeHighlighter`, but its VCS renderer deliberately uses separate highlighters for the gutter and for the error stripe.
- **Zed** has two separate code paths, each with its own settings.
- **CodeMirror 6** has gutters only.

So Kit's two-API plan conflicts with Monaco's data model, but it matches Kit's own precedent (`TextDecoration` and `RangeDecoration` are already separate collections) and it matches IntelliJ and Zed. Everything else in the plan can follow Monaco directly:

- tracked ranges that the owner replaces after each re-diff;
- whole-line coverage;
- an empty range at a line boundary for a deletion;
- Monaco's `OverviewRulerLane` {Left, Center, Right, Full}, with SCM in Left, find in Center and diagnostics in Right. Zed uses the same three columns in the same order.

One Kit-specific blocker: Kit's `normalize` rejects empty ranges, and its edit transform drops ranges that become empty. A deleted-lines marker therefore needs its own normalization and edit rule.

---

## 1. Monaco / VS Code

### 1.1 Which `IModelDecorationOptions` field targets which surface

All of these fields live on one options bag, `IModelDecorationOptions` ([model.ts](https://github.com/microsoft/vscode/blob/4bda7217fc83e8e838b1edd9ebaf21bb9b498c7e/src/vs/editor/common/model.ts), [typedoc](https://microsoft.github.io/monaco-editor/typedoc/interfaces/editor_editor_api.editor.IModelDecorationOptions.html)). A single decoration can set any combination of them.

| Field | Surface | Notes from the source docs |
| --- | --- | --- |
| `linesDecorationsClassName` (+ `linesDecorationsTooltip`) | The **lines-decorations** column, between the line numbers and the text | "rendered in the lines decorations with this CSS class name". It renders on every line of the range. |
| `firstLineDecorationClassName` | Same column, first line only | Folding chevrons use it ([foldingDecorations.ts](https://github.com/microsoft/vscode/blob/4bda7217fc83e8e838b1edd9ebaf21bb9b498c7e/src/vs/editor/contrib/folding/browser/foldingDecorations.ts)). |
| `glyphMarginClassName` (+ `glyphMargin: {position: GlyphMarginLane, persistLane}`, `glyphMarginHoverMessage`) | The **glyph margin**, the left-most column (breakpoints, icons) | The width is `lineHeight × lane count`. The option "defaults to true in vscode and to false in monaco-editor". |
| `lineNumberClassName` | The line number itself | |
| `marginClassName` | The whole margin width | |
| `overviewRuler: {color, darkColor?, position: OverviewRulerLane}` | The **overview ruler** on the vertical scrollbar track | `OverviewRulerLane { Left = 1, Center = 2, Right = 4, Full = 7 }` is a bitmask ([typedoc](https://microsoft.github.io/monaco-editor/typedoc/enums/editor_editor_api.editor.OverviewRulerLane.html), [IModelDecorationOverviewRulerOptions](https://microsoft.github.io/monaco-editor/typedoc/interfaces/editor_editor_api.editor.IModelDecorationOverviewRulerOptions.html)). |
| `minimap: {color, position: MinimapPosition.Inline \| Gutter}` | The minimap | Kit has no minimap. |
| `isWholeLine` | Any surface | "Should the decoration expand to encompass a whole line." |
| `stickiness: TrackedRangeStickiness` | Edit tracking | Defaults to `AlwaysGrowsWhenTypingAtEdges`. The other values are `NeverGrows…`, `GrowsOnlyWhenTypingBefore` and `GrowsOnlyWhenTypingAfter`. |
| `zIndex` | Stacking on the same line | |

The owner holds an `IEditorDecorationsCollection` ([editorCommon.ts](https://github.com/microsoft/vscode/blob/4bda7217fc83e8e838b1edd9ebaf21bb9b498c7e/src/vs/editor/common/editorCommon.ts), [typedoc](https://microsoft.github.io/monaco-editor/typedoc/interfaces/editor_editor_api.editor.IEditorDecorationsCollection.html)) with these members: `set`, `append`, `clear`, `getRanges`, `getRange(i)`, `has`, `length`, and `onDidChange`, which fires "when decorations change in the editor, but the change is not caused by us setting or clearing the collection".

The VS Code **extension** API has the same one-object shape. A single `TextEditorDecorationType` carries `gutterIconPath`, `overviewRulerColor`, `overviewRulerLane`, `isWholeLine` and `rangeBehavior` ([vscode.d.ts](https://github.com/microsoft/vscode/blob/4bda7217fc83e8e838b1edd9ebaf21bb9b498c7e/src/vscode-dts/vscode.d.ts), [docs](https://code.visualstudio.com/api/references/vscode-api#DecorationRenderOptions)).

Monaco does have one separate, untracked ruler API, but it is internal: `ICodeEditor.createOverviewRuler()` returns an `IOverviewRuler` with `setZones(OverviewRulerZone[])`. A zone is `startLineNumber`, `endLineNumber`, `heightInLines` and `color`, re-set by line number each time. The diff editor uses it for its own side ruler ([editorBrowser.ts](https://github.com/microsoft/vscode/blob/4bda7217fc83e8e838b1edd9ebaf21bb9b498c7e/src/vs/editor/browser/editorBrowser.ts), [overviewZoneManager.ts](https://github.com/microsoft/vscode/blob/4bda7217fc83e8e838b1edd9ebaf21bb9b498c7e/src/vs/editor/common/viewModel/overviewZoneManager.ts), whose `MINIMUM_HEIGHT` is 4).

### 1.2 Margin layout

`EditorLayoutInfoComputer` ([editorOptions.ts L2940-2977](https://github.com/microsoft/vscode/blob/4bda7217fc83e8e838b1edd9ebaf21bb9b498c7e/src/vs/editor/common/config/editorOptions.ts#L2940-L2977)) lays the margin out from left to right:

```
glyphMarginLeft = 0
lineNumbersLeft = glyphMarginLeft + glyphMarginWidth      // lineHeight × lanes, 0 if off
decorationsLeft = lineNumbersLeft + lineNumbersWidth
contentLeft     = decorationsLeft + lineDecorationsWidth  // default 10px, +16px when folding controls are shown
```

So the order is **glyph margin | line numbers | lines decorations (which include the folding chevrons) | text**. The option doc says: "Line decorations are placed between line numbers and the editor content… Defaults to 10" (the value can also be given as `"1.3ch"`).

`linesDecorations.ts` renders each `linesDecorationsClassName` as an absolutely positioned div with `left: decorationsLeft` and `width: decorationsWidth`. Folding renders its chevron in the same column through `firstLineDecorationClassName`, with `margin-left: 2px` and the icon centred ([folding.css](https://github.com/microsoft/vscode/blob/4bda7217fc83e8e838b1edd9ebaf21bb9b498c7e/src/vs/editor/contrib/folding/browser/folding.css)). The SCM bar takes `margin-left: 5px` in that column (section 1.3). The bar and the chevron therefore share one 26px column: the bar sits near its left edge and the chevron in its middle.

### 1.3 Quick diff (dirty diff) decorations

Source: [`quickDiffDecorator.ts`](https://github.com/microsoft/vscode/blob/4bda7217fc83e8e838b1edd9ebaf21bb9b498c7e/src/vs/workbench/contrib/scm/browser/quickDiffDecorator.ts), with styles in [`media/dirtydiffDecorator.css`](https://github.com/microsoft/vscode/blob/4bda7217fc83e8e838b1edd9ebaf21bb9b498c7e/src/vs/workbench/contrib/scm/browser/media/dirtydiffDecorator.css) and colours in [`common/quickDiff.ts`](https://github.com/microsoft/vscode/blob/4bda7217fc83e8e838b1edd9ebaf21bb9b498c7e/src/vs/workbench/contrib/scm/common/quickDiff.ts).

**Each hunk becomes one decoration that carries every surface as a facet.** `QuickDiffDecorator.createDecoration` (L34-62) builds a single `IModelDecorationOptions` as follows:

- `description: 'dirty-diff-decoration'` and `isWholeLine` (true for added and modified, false for deleted);
- if the gutter is enabled, `linesDecorationsClassName = "dirty-diff-glyph " + className` and `linesDecorationsTooltip` ("Added lines" / "Changed lines" / "Removed lines");
- if the overview ruler is enabled, `overviewRuler = { color: themeColorFromId(...), position: OverviewRulerLane.Left }`;
- if the minimap is enabled, `minimap = { color, position: MinimapPosition.Gutter }`.

`stickiness` is not set, so the default `AlwaysGrowsWhenTypingAtEdges` applies.

The setting `scm.diffDecorations` takes `all | gutter | overview | minimap | none` (default `all`) and simply decides which facets get set ([scm.contribution.ts](https://github.com/microsoft/vscode/blob/4bda7217fc83e8e838b1edd9ebaf21bb9b498c7e/src/vs/workbench/contrib/scm/browser/scm.contribution.ts)). Class variants are `primary` / `secondary` (the second quick-diff provider) and `pattern` (a hatched bar when `scm.diffDecorationsGutterPattern` is on).

**Ranges (L150-190).** The model's `IChange` uses `modifiedEndLineNumber == 0` for a deletion. For a deletion, `modifiedStartLineNumber` is the line *before* the gap: [`toLineChanges`](https://github.com/microsoft/vscode/blob/4bda7217fc83e8e838b1edd9ebaf21bb9b498c7e/src/vs/editor/browser/widget/diffEditor/diffEditorWidget.ts#L869) sets it to `modified.startLineNumber - 1`.

- **Add / Modify:** `{startLine, col 1} → {endLine, col 1}` with `isWholeLine: true`.
- **Delete:** an **empty range at the end of the line above the gap**, `{startLine, col MAX} → {startLine, col MAX}`, with `isWholeLine: false`.
  - A deletion at the top of the file has `startLine = 0`. Decoration validation ([`_validateRangeRelaxedNoAllocations`](https://github.com/microsoft/vscode/blob/4bda7217fc83e8e838b1edd9ebaf21bb9b498c7e/src/vs/editor/common/model/textModel.ts#L925)) clamps that to (1, 1). The marker then sits on line 1's bottom edge instead of above line 1.

All hunks go into one `createDecorationsCollection` that is replaced with `.set(decorations)` on every model change. The model re-diffs on content change through a 200 ms `ThrottledDelayer` ([quickDiffModel.ts L108](https://github.com/microsoft/vscode/blob/4bda7217fc83e8e838b1edd9ebaf21bb9b498c7e/src/vs/workbench/contrib/scm/browser/quickDiffModel.ts#L108)). Between re-diffs, the model's tracked ranges move the bars.

**Shapes (CSS):**

- `.dirty-diff-glyph` has `margin-left: 5px`. Added and modified lines get a solid `border-left` whose width is `scm.diffDecorationsGutterWidth`: 1-5, default **3px**, written into a dynamic stylesheet.
- `.dirty-diff-deleted:after` is a **right-pointing CSS triangle**, `width: 4px; height: 0; border-top/bottom: 4px solid transparent; border-left: 4px solid <deleted colour>`. It is positioned at `bottom: -4px` on the anchor line, so it is 8px tall and centred on the boundary between the anchor line and the next one.
- `scm.diffDecorationsGutterVisibility: always | hover` sets the markers' opacity to 0 until the margin is hovered.
- `scm.diffDecorationsGutterAction: diff | none` makes a click open the inline peek diff.

**Theme colour ids** (`common/quickDiff.ts` L25-61):

| Id | Default |
| --- | --- |
| `editorGutter.addedBackground` | dark `#487E02`, light `#48985D` |
| `editorGutter.modifiedBackground` | dark `#1B81A8`, light `#2090D3` |
| `editorGutter.deletedBackground` | defaults to `editorError.foreground` |
| `editorGutter.{added,modified,deleted}SecondaryBackground` | darkened or lightened variants of the above |
| `editorOverviewRuler.{added,modified,deleted}Foreground` | the gutter colour at 60% opacity (`transparent(…, 0.6)`) |
| `minimapGutter.{added,modified,deleted}Background` | |

### 1.4 How the overview ruler is painted

Source: [`decorationsOverviewRuler.ts`](https://github.com/microsoft/vscode/blob/4bda7217fc83e8e838b1edd9ebaf21bb9b498c7e/src/vs/editor/browser/viewParts/overviewRuler/decorationsOverviewRuler.ts).

- **Layer and position.**
  - It is a separate `<canvas class="decorationsOverviewRuler">`. `view.ts` inserts it into the scrollable element's DOM *before* the vertical scrollbar node (`getOverviewRulerLayoutInfo` returns `{parent, insertBefore: verticalScrollbar.domNode}`, [scrollableElement.ts L334](https://github.com/microsoft/vscode/blob/4bda7217fc83e8e838b1edd9ebaf21bb9b498c7e/src/vs/base/browser/ui/scrollbar/scrollableElement.ts#L334)). The scrollbar slider is therefore painted **on top of** the markers.
  - The slider is translucent by default: `scrollbarSlider.background` is `#797979` at 40% opacity.
  - Layout gives the ruler `width = verticalScrollbarSize` (default 14), `right: 0`, and `top = arrow size` ([editorOptions.ts L3061](https://github.com/microsoft/vscode/blob/4bda7217fc83e8e838b1edd9ebaf21bb9b498c7e/src/vs/editor/common/config/editorOptions.ts#L3061)).
- **Always visible.** The scrollbar's fade (`.monaco-scrollable-element > .invisible { opacity: 0 }`) applies to the scrollbar element only. The canvas is a sibling, so the markers stay visible while the slider fades. (This is inferred from the DOM structure and CSS, not from a doc statement.) `overviewRulerLanes: 0` hides the ruler.
- **Lanes.** `overviewRulerLanes` defaults to 3, with a range of 0-3. `_initLanes` splits the canvas width into thirds (1px left offset) and maps the bitmask, so `Left | Center` spans two thirds and `Full` spans all three.
  - By convention SCM uses **Left** (above), find matches use **Center** ([findDecorations.ts](https://github.com/microsoft/vscode/blob/4bda7217fc83e8e838b1edd9ebaf21bb9b498c7e/src/vs/editor/contrib/find/browser/findDecorations.ts)), and diagnostics use **Right**, with `NeverGrowsWhenTypingAtEdges` ([markerDecorationsService.ts L324-329](https://github.com/microsoft/vscode/blob/4bda7217fc83e8e838b1edd9ebaf21bb9b498c7e/src/vs/editor/common/services/markerDecorationsService.ts#L324-L329)).
  - Cursors are drawn as 2px lines across the Full lane.
- **Geometry.**
  - `y = verticalOffset(line) × canvasHeight / scrollHeight`. The scroll height includes `scrollBeyondLastLine`, so the ruler is proportional to scrollable height, not to line count.
  - `MIN_DECORATION_HEIGHT = 6` (× pixel ratio). A shorter marker is grown around its centre and clamped inside the canvas.
  - Decorations are grouped by colour, and runs in the same lane are merged when they touch (`y1 <= prevY2 + 1`).
  - An optional 1px border is drawn (`overviewRulerBorder`, colour `editorOverviewRuler.border`).
  - The ruler repaints only when the sorted decoration list or the cursor rows change.

---

## 2. JetBrains IntelliJ Platform

### 2.1 One primitive with per-surface facets

[`RangeHighlighter`](https://github.com/JetBrains/intellij-community/blob/43722b268dbc6359441135e1895a63d0fbb15fca/platform/editor-ui-api/src/com/intellij/openapi/editor/markup/RangeHighlighter.java) "represents a range of text in the document which has specific markup (special text attributes, line marker, gutter icon, error stripe marker or line separator)". It is created with `MarkupModel.addRangeHighlighter(start, end, layer, attrs, HighlighterTargetArea.EXACT_RANGE | LINES_IN_RANGE)` or with `addLineHighlighter`. It has these facets:

| Setter | Surface |
| --- | --- |
| `setLineMarkerRenderer(LineMarkerRenderer)` | The line-marker area. The javadoc says: "Line markers are drawn over the folding area and are used, for example, to highlight modified lines in files under source control." |
| `setGutterIconRenderer(GutterIconRenderer)` | The icon area, "to the left of the folding area". |
| `setErrorStripeMarkColor(Color)`, `setErrorStripeTooltip(Object)`, `setThinErrorStripeMark(boolean)` | The **error stripe**, IntelliJ's overview ruler. A thin mark has "reduced width (like the markers used to highlight changed lines)". The colour can also come from `TextAttributes.getErrorStripeColor()`. |
| `setLineSeparatorColor` / `setCustomRenderer` | Other surfaces. |

`LineMarkerRenderer.paint(editor, g, r)` receives a rectangle that spans the highlighter's lines ([source](https://github.com/JetBrains/intellij-community/blob/43722b268dbc6359441135e1895a63d0fbb15fca/platform/editor-ui-api/src/com/intellij/openapi/editor/markup/LineMarkerRenderer.java)).

- `LineMarkerRendererEx.getPosition()` returns `LEFT | RIGHT | CUSTOM` ("CUSTOM - over whole gutter area") ([source](https://github.com/JetBrains/intellij-community/blob/43722b268dbc6359441135e1895a63d0fbb15fca/platform/platform-impl/src/com/intellij/openapi/editor/markup/LineMarkerRendererEx.java)).
- `ActiveGutterRenderer` adds `canDoAction`, `doAction(editor, MouseEvent)`, `getTooltipText` and `calcBounds` ([source](https://github.com/JetBrains/intellij-community/blob/43722b268dbc6359441135e1895a63d0fbb15fca/platform/editor-ui-api/src/com/intellij/openapi/editor/markup/ActiveGutterRenderer.java)).

Gutter order ([EditorGutterLayout.java](https://github.com/JetBrains/intellij-community/blob/43722b268dbc6359441135e1895a63d0fbb15fca/platform/platform-impl/src/com/intellij/openapi/editor/impl/EditorGutterLayout.java)):

- **Classic UI:** line numbers | annotations | left free painters | icons | right free painters | folding.
- **New UI:** annotations | line numbers | … icons | right free painters | folding | **extra right free painters**.

In the new UI the VCS bar paints in that last area, just before the text (`getExtraLineMarkerFreePaintersAreaOffset`, see 2.2).

### 2.2 How VCS change markers are drawn: separate highlighters per surface

[`LineStatusMarkerRenderer.kt`](https://github.com/JetBrains/intellij-community/blob/43722b268dbc6359441135e1895a63d0fbb15fca/platform/diff-impl/src/com/intellij/openapi/vcs/ex/LineStatusMarkerRenderer.kt) keeps the two surfaces apart, even though one `RangeHighlighter` could carry both:

- **Gutter: one highlighter for the whole document** (`createGutterHighlighter`, L136). It is a single `RangeHighlighter` over `0..document.textLength` with `LINES_IN_RANGE` and greedy flags on both sides. Its `LineMarkerRenderer` (`LineStatusGutterMarkerRenderer`, position `CUSTOM`) paints *all* change ranges itself in `paint()`, from the tracker's current `List<Range>`. The active variant ([ActiveLineStatusGutterMarkerRenderer.kt](https://github.com/JetBrains/intellij-community/blob/43722b268dbc6359441135e1895a63d0fbb15fca/platform/diff-impl/src/com/intellij/openapi/vcs/ex/ActiveLineStatusGutterMarkerRenderer.kt)) hit-tests the ranges under the mouse to open the change popup.
- **Error stripe: one highlighter per change range** (`updateErrorStripeHighlighters`, L186, and `createErrorStripeHighlighter`, L232). Each one covers the range's lines, sets `setThinErrorStripeMark(true)`, and uses `DiffStripeTextAttributes(type)`, whose `getErrorStripeColor()` returns the type's colour. A highlighter is reused when its offsets and type are unchanged; otherwise it is disposed and recreated.
- `shouldPaintErrorStripeMarkers()` is an overridable switch for the stripe alone.
- Both surfaces update through a 100 ms `DebouncedUpdates` queue.

**Anchoring.** The change ranges are line numbers held by the [`DocumentTracker`](https://github.com/JetBrains/intellij-community/blob/43722b268dbc6359441135e1895a63d0fbb15fca/platform/diff-impl/src/com/intellij/openapi/vcs/ex/DocumentTracker.kt), which shifts and updates its blocks synchronously in `documentChanged` (L404). The gutter renderer reads them at paint time. The error-stripe highlighters are `RangeMarker`s, which are "automatically adjusted as the document text is modified", with optional `greedyToLeft` / `greedyToRight` growth and, for an empty marker, `setStickingToRight` ([RangeMarker.java](https://github.com/JetBrains/intellij-community/blob/43722b268dbc6359441135e1895a63d0fbb15fca/platform/core-api/src/com/intellij/openapi/editor/RangeMarker.java)).

**Shapes** ([LineStatusMarkerDrawUtil.java](https://github.com/JetBrains/intellij-community/blob/43722b268dbc6359441135e1895a63d0fbb15fca/platform/diff-impl/src/com/intellij/openapi/diff/LineStatusMarkerDrawUtil.java)):

- **Bar width.** In the new UI it is the registry value `gutter.vcs.changes.width`, default **4** (range 4-6), scaled with editor zoom and placed in the extra-right free-painter area with 1-3px of left padding. Hovered blocks grow 3px to the left. In the classic UI the bar runs from the free-painter offset to the folding outline's centre.
- **Rect.** In the new UI the rect is rounded, with a radius equal to the width and 1px inset top and bottom. In the classic UI it is a plain fill plus an optional border (`BORDER_LINES_COLOR`).
- **Deleted block.** A range with `y1 == y2` lies on the boundary at the top of the line after the deletion.
  - In the classic UI it is a **right-pointing triangle**, half-height 4px (8px tall), centred on that boundary.
  - In the new UI it is a **rounded pill** `2×5−2 = 8px` tall, centred on the boundary and as wide as the bar.
  - The hit area extends `lineHeight/3` above and below the boundary.
- **Colours** ([LineStatusMarkerColorScheme.kt](https://github.com/JetBrains/intellij-community/blob/43722b268dbc6359441135e1895a63d0fbb15fca/platform/diff-impl/src/com/intellij/openapi/diff/LineStatusMarkerColorScheme.kt)):
  - Gutter: `EditorColors.ADDED_LINES_COLOR`, `MODIFIED_LINES_COLOR`, `DELETED_LINES_COLOR` and `WHITESPACES_MODIFIED_LINES_COLOR`.
  - Ignored-change outlines: `IGNORED_*_BORDER_COLOR`.
  - Error stripe: `DiffColors.DIFF_INSERTED`, `DIFF_MODIFIED` and `DIFF_DELETED`, through `.errorStripeColor`.

### 2.3 How the error stripe is painted

The stripe is the editor's own vertical scrollbar UI. `EditorMarkupModelImpl.MyErrorPanel` extends `ButtonlessScrollBarUI` and paints the marks in `paintTrack` into a cached image, then paints a **translucent thumb** over them (`isThumbTranslucent() = true`, `alwaysPaintThumb() = true`) ([EditorMarkupModelImpl.kt](https://github.com/JetBrains/intellij-community/blob/43722b268dbc6359441135e1895a63d0fbb15fca/platform/platform-impl/src/com/intellij/openapi/editor/impl/EditorMarkupModelImpl.kt)).

- **Thickness and columns.** The thickness is `Editor.scrollBarWidth (14) + thinGap (2) + minMarkHeight`.
  - Thin marks (VCS) go in a **left column** as wide as `minMarkHeight`.
  - Wide marks (errors, warnings) go in a 14px column to its right (`drawSpot`, L1220). The sides swap when the scrollbar is mirrored.
- **Minimum mark height.** It comes from `DaemonCodeAnalyzerSettings.errorStripeMarkMinHeight`, default **2**, scaled. Thin marks shorter than 6px are stretched slightly.
- **Overlaps.** When marks overlap, the higher layer wins.

---

## 3. Zed (GPL: approach and API shape only)

### 3.1 Git diff hunks in the gutter

- **Source.** The hunks come from the buffer's `BufferDiff`. Each `DiffHunk` holds a `buffer_range: Range<Anchor>`, and anchors follow edits; the diff itself is recomputed asynchronously. `git.gutter_debounce` (default 0 ms) debounces gutter refreshes ([default.json](https://github.com/zed-industries/zed/blob/ec86047d67acd6fddd81392f44ed6ebcc6e7f531/assets/settings/default.json), [buffer_diff.rs](https://github.com/zed-industries/zed/blob/ec86047d67acd6fddd81392f44ed6ebcc6e7f531/crates/buffer_diff/src/buffer_diff.rs)).
- **Per frame.** The editor converts the hunks for the visible rows into display rows (`display_diff_hunks_for_rows` in [editor/src/git.rs](https://github.com/zed-industries/zed/blob/ec86047d67acd6fddd81392f44ed6ebcc6e7f531/crates/editor/src/git.rs)). It produces one of two variants:
  - **Folded**: the hunk starts inside a fold. It is drawn as one line-tall bar on the fold's row, always in the *modified* colour.
  - **Unfolded**: carries the display row range and the added/modified/deleted status.
- **Painting** ([editor/src/element.rs](https://github.com/zed-industries/zed/blob/ec86047d67acd6fddd81392f44ed6ebcc6e7f531/crates/editor/src/element.rs): `layout_gutter_diff_hunks`, `paint_gutter_diff_hunks`, `diff_hunk_bounds`):
  - The strip sits at the **far left edge of the gutter**, inside its left padding, before the runnable/breakpoint buttons and the line numbers.
  - Its width defaults to `floor(0.275 × line height)`. `gutter.git_gutter_width` sets a custom pixel width.
  - Each unfolded hunk gets a mouse-blocking hitbox; clicking toggles the inline diff.
- **Deleted hunk** (an empty row range). It is anchored at the start of the row *after* the deletion. It is drawn as a fully rounded pill one line height tall, **centred on that row's top edge**, about `0.35 × line height` wide, and partly outside the gutter's left edge.
- **Style and colours.**
  - Colours are the theme's `version_control_added`, `version_control_modified` and `version_control_deleted`, blended over the editor background.
  - `git.hunk_style` (`staged_hollow` | `unstaged_hollow`) draws one of staged or unstaged hunks as a 1px outline over a 30% fill.
  - `git.git_gutter: tracked_files | hide` turns the gutter display on or off.

### 3.2 Scrollbar markers

`paint_scrollbars` paints the track, then markers, then the thumb on top (`element.rs` around L6152). There are two kinds of markers:

- **Fast markers** are cursors. They are recomputed every frame and drawn as 2px lines across the full track width (`LINE_MARKER_HEIGHT`).
- **Slow markers** are computed on a background thread and cached in the editor's `scrollbar_marker_state` with a dirty flag (`refresh_slow_scrollbar_markers`). They come in fixed **columns**: the track minus a 1px border is split into thirds.

| Column | Category | Setting (`scrollbar.*`) |
| --- | --- | --- |
| 0 | git diff hunks (added/modified/deleted colours) | `git_diff` (default true) |
| 1 | buffer search results, selected-text occurrences, selected-symbol occurrences (`status.info`, faded) | `search_results`, `selected_text`, `selected_symbol` |
| 2 | diagnostics by severity, the most severe painted last | `diagnostics: all \| information \| warning \| error \| none` |
| full width | cursors | `cursors` |

Column markers have a minimum height of **5px** (`MIN_MARKER_HEIGHT`). Consecutive same-colour ranges within 1px are merged (`marker_quads_for_ranges`).

With `scrollbar.show: "auto"` (the default), the scrollbar is shown whenever any enabled category has content: diff hunks, search highlights, selection or symbol highlights, diagnostics, or cursors out of view (`ShowScrollbar::Auto`, L1520). Zed keeps the markers visible by keeping the whole scrollbar visible, not by drawing a separate layer.

### 3.3 Shape

Zed has no public decoration type with surface facets. The editor element reads each source directly (diff hunks, keyed background highlights, diagnostics) and has **one code path per surface**, with independent settings for each: `git.git_gutter` and `gutter.git_gutter_width` for the gutter, and `scrollbar.git_diff` and the others for the scrollbar.

---

## 4. CodeMirror 6

Source: [`@codemirror/view` gutter.ts](https://github.com/codemirror/view/blob/fbff59ba004d80d8c914f64c42586387b08706ac/src/gutter.ts) and the [reference](https://codemirror.net/docs/ref/#view.gutter).

- `gutter(config)` defines **one gutter column per extension**, ordered by extension priority. Its config fields:
  - `class` and `renderEmptyElements`;
  - `markers: (view) => RangeSet<GutterMarker> | RangeSet<GutterMarker>[]`;
  - `lineMarker: (view, line, otherMarkers) => GutterMarker | null`, for computed per-line markers such as line numbers;
  - `widgetMarker`, for markers next to block widgets;
  - `lineMarkerChange`, `initialSpacer`/`updateSpacer`, `domEventHandlers`;
  - `side: "before" | "after"`.
- `gutters({fixed})` makes the gutters sticky during horizontal scrolling.
- `GutterMarker extends RangeValue` and offers `eq`, `toDOM?`, `elementClass` and `destroy`. Its prototype defaults are `point = true`, `startSide = endSide = -1` and `mapMode = MapMode.TrackBefore`. A marker is attached to the line containing its position and maps through edits as a point that stays before text inserted at that position.
- `gutterLineClass: Facet<RangeSet<GutterMarker>>` adds a class to *every* gutter element of a line. Markers given to it should only set `elementClass`. `gutterWidgetClass` does the same next to widgets, and `lineNumberMarkers` feeds the line-number gutter.
- **Anchoring.** Markers live in a `RangeSet`, which "can efficiently be mapped though document changes" (`RangeSet.map(changes)`). The owner keeps the set in a `StateField` or computes it in a view plugin.
  - The official Git-style example is `@codemirror/merge`. Its `changeGutter` is a `gutter({class: "cm-changeGutter", markers})` whose markers a view plugin rebuilds for the viewport from the chunk list on each doc, viewport or chunk change. The chunks themselves are updated with every transaction ([deco.ts](https://github.com/codemirror/merge/blob/3bc269331394691dcce9784ba8c93b318d15535f/src/deco.ts)).
  - The marker is a singleton `GutterMarker` with only `elementClass = "cm-changedLineGutter"`, placed at each changed line's start. The theme gives the gutter `width: 3px; padding-left: 1px` and colours the class by side ([theme.ts](https://github.com/codemirror/merge/blob/3bc269331394691dcce9784ba8c93b318d15535f/src/theme.ts)).
  - In the unified view, deleted chunks are block widgets, and their gutter mark comes from `widgetMarker` returning `cm-deletedLineGutter` ([unified.ts](https://github.com/codemirror/merge/blob/3bc269331394691dcce9784ba8c93b318d15535f/src/unified.ts)).
- **No scrollbar markers in core.** On the forum (May 2023) the maintainer answered "No, not yet." He said annotating the native scrollbar is hard because scrollbars vary across platforms, an overlay a few pixels wide on the editor's right edge "should be very doable", and the search extension does not eagerly collect all matches ([discuss thread](https://discuss.codemirror.net/t/show-location-of-matched-search-query-in-scrollbar/6463)). CodeMirror 5 had `addon/scroll/annotatescrollbar.js` (`cm.annotateScrollbar`), which v6 never ported.

---

## 5. Side by side

| | Monaco / VS Code | IntelliJ | Zed | CodeMirror 6 |
| --- | --- | --- | --- | --- |
| Gutter API | `linesDecorationsClassName` facet on a decoration | `LineMarkerRenderer` facet on a `RangeHighlighter` | none public: the element reads diff hunks | `gutter({markers})` + `GutterMarker` |
| Overview API | `overviewRuler {color, lane}` facet on the same decoration | error-stripe facet on a `RangeHighlighter` | none public: fixed categories | none in core |
| VCS uses one object or several | **one decoration per hunk with both facets** | **separate**: 1 gutter highlighter for the whole document + N stripe highlighters | **separate** code paths, shared source | gutter only |
| Anchoring | tracked model ranges, replaced via `collection.set` after each 200 ms re-diff | tracker line ranges shifted on every edit; stripe uses `RangeMarker`s; 100 ms debounce | buffer `Anchor`s, async re-diff | `RangeSet` mapped through changes |
| Deleted hunk anchor | empty range at the **end of the line above** (top of file clamps to line 1) | empty line range, **top edge of the line below** | empty row range, **top edge of the row below** | block widget (`widgetMarker`) |
| Deleted marker | 4×8px triangle centred on the boundary | 8px triangle (classic) / 8px pill (new UI) | pill one line tall, centred on the boundary | class on the widget's gutter cell |
| Bar width | 3px (1-5), 5px after the line numbers | 4px (4-6) new UI | 0.275 × line height | 3px |
| Bar position | between line numbers and text, sharing the column with fold chevrons | new UI: after folding, next to the text | far left of the gutter | its own gutter column |
| Overview layer | canvas under a translucent slider, not faded with it | the scrollbar track, under a translucent thumb | track, markers, then thumb; `auto` keeps the bar visible when markers exist | n/a |
| Lanes | Left / Center / Right / Full; SCM Left, find Center, diagnostics Right | thin column (VCS) + wide column | columns 0 git, 1 search/highlights, 2 diagnostics; cursors full width | n/a |
| Min marker height | 6px | 2px (thin marks stretched below 6px) | 5px (cursor lines 2px) | n/a |

---

## 6. Synthesis for a Kit API

### 6.1 What Kit has today

Kit's decorations (`gpui-base-0.8.0/src/input/editor/decorations.rs`, identical on `main` at `64e6aaff`):

- **Two separate typed collections.**
  - `create_decorations_collection(Vec<TextDecoration>) -> TextDecorationCollection` holds `HighlightStyle` runs. Its doc links Monaco's `createDecorationsCollection` and `IEditorDecorationsCollection`.
  - `create_range_decorations_collection(Vec<RangeDecoration>) -> RangeDecorationCollection` holds geometric fill or frame decorations with an optional `Hsla`, where `None` means the editor foreground.
  - Kit has therefore already split Monaco's single options bag by paint surface.
- **Handles.** A handle is a weak reference to the state plus an id, offering `set`, `append`, `clear` and `get_ranges`. `RangeDecorationCollection` also has `dispose`. Operations after a dispose, or after the editor is dropped, do nothing.
- **Ranges.** Ranges are half-open UTF-8 byte offsets, and they are **tracked**: `adjust_for_edit` runs on every edit, including undo, redo and formatting, with Monaco's `NeverGrowsWhenTypingAtEdges` semantics.
- **Empty ranges.** **`normalize` rejects empty ranges, and `adjust_for_edit` drops any range an edit empties.** An empty anchor that someone types at would also become reversed and be dropped.
- **Folds.** Painting queries `intersecting(visible_spans)`, an interval index that skips folded-away text. A decoration that lies entirely inside a fold therefore disappears.

The gutter is laid out by `layout_line_numbers` and `layout_fold_icons` in `input/base/element.rs` on `main`. From left to right: padding | line numbers | an 18px fold-icon hitbox (when folding is on) | the 6px `LINE_NUMBER_RIGHT_MARGIN` | text.

The scrollbar can be replaced since #3644: `InputBaseState::set_scrollbar_renderer(Option<EditorScrollbarRenderer>)` receives an `EditorScrollbarContext: ScrollbarHandle`. Since #3616, Kit's scrollbar fades after idle.

### 6.2 What "following Monaco" most naturally gives Kit

**The literal Monaco design would put the facets on one decoration**, for example `with_gutter(...)` and `with_overview_ruler(color, lane)` on a whole-line decoration in a single collection. Choosing two APIs instead keeps Kit's precedent of one typed collection per paint surface, and it can still borrow everything else from Monaco:

1. **Two collections that mirror `RangeDecorationCollection` exactly.** They live on `InputBaseState<EditorMode>`, with the same handle type, `set`/`append`/`clear`/`dispose`/`get_ranges`, edit tracking and ownership semantics:
   - `create_gutter_decorations_collection(Vec<GutterDecoration>) -> GutterDecorationCollection`;
   - `create_overview_ruler_decorations_collection(Vec<OverviewRulerDecoration>) -> OverviewRulerDecorationCollection`.

   The doc comments can link Monaco's `linesDecorationsClassName`, `overviewRuler` and `OverviewRulerLane` the way `decorations.rs` already links Monaco's types.
   - "Overview ruler" is Monaco's name for the scrollbar markers. It names a surface that still makes sense when an application supplies its own scrollbar.
   - "Gutter" is clearer than Monaco's "lines decorations" and is what IntelliJ, Zed and CodeMirror call the area.
2. **Whole-line coverage, as with `isWholeLine`.** A non-empty byte range marks every visual line it intersects, using the half-open intersection the interval index already implements. An application turns Git line ranges into `line_start(a)..line_start(b + 1)`, or `..text.len()` at the end of the file. This is the same as Monaco's `{a, col 1} → {b, col 1}` with `isWholeLine`.
3. **A deleted marker is an empty range at a line boundary.** All four editors represent a deletion as an empty range.
   - Anchor it at the **start of the line after the deletion**, or at `text.len()` when the deletion is at the end of the file, and paint it centred on that line's top edge. IntelliJ and Zed do this.
   - Monaco's anchor at the end of the line above clamps wrongly at the top of the file.
   - Geometry can follow from whether the range is empty, as IntelliJ's `paintSimpleRange` does: a non-empty range is a bar, an empty range is a boundary marker. That avoids a style enum.
   - The gutter collection needs **its own `normalize` that keeps empty ranges** (clipped to a char boundary) **and an edit rule for points**. The point should stay before text inserted at its offset, as CodeMirror's `startSide = -1` and IntelliJ's default `stickingToRight = false` do, and a range that collapses should survive, as Monaco's collapsed decorations do. The owner's next `set` replaces it anyway.
4. **Lanes: copy Monaco's `OverviewRulerLane { Left, Center, Right, Full }` and its conventions.** Git goes in Left, search in Center and diagnostics in Right. Zed's columns 0, 1 and 2 hold exactly these categories in the same order, and IntelliJ also puts VCS in its own left column, so this is the strongest point of agreement in the research.
5. **Ruler painting, as Monaco does it.**
   - Use a layer beneath the scrollbar element, whether that is Base's or a renderer's, over the vertical track bounds.
   - Make positions proportional to content height, including wrap rows.
   - Use a minimum marker height: Monaco 6px, Zed 5px, IntelliJ 2px.
   - Merge touching runs of the same colour.
   - Keep the markers visible when the thumb fades.

   Painting beneath the element means custom scrollbars from `set_scrollbar_renderer` get markers without having to know about them. The alternative is to put the markers into `EditorScrollbarContext`. Zed's alternative of keeping the scrollbar visible while markers exist conflicts with #3616's fade.
6. **Gutter placement.** No position is universal: Monaco puts the bar between the line numbers and the fold chevron, IntelliJ's new UI between folding and the text, and Zed at the far left. For Kit, the Monaco analogue is a 3-4px bar between the line numbers and the 18px fold hitbox; the IntelliJ analogue is the 6px margin before the text. Monaco always reserves `lineDecorationsWidth`, so Kit should reserve the strip while a gutter collection exists rather than only while it is non-empty, so that text does not shift as the diff changes.
7. **Colours.** Kit's precedent is an optional `Hsla` with an editor fallback, but there is no neutral fallback for added, modified or deleted lines. Either require a colour, or add theme tokens named after VS Code's `editorGutter.{added,modified,deleted}Background` and `editorOverviewRuler.*Foreground`. VS Code's default ruler colour is the gutter colour at 60% opacity.
8. **Usage contract to document.** All four editors track ranges between recomputations and replace the whole set after each one: VS Code with a 200 ms throttle, IntelliJ with a 100 ms debounce, and Zed with an async re-diff. `set()` after each re-diff is therefore the intended use, and tracked ranges keep markers aligned in between.
9. **Folds.** Zed draws a hunk hidden in a fold on the fold's row, in the modified colour. Kit's `intersecting` currently hides such a hunk. The PR should either map it to the fold's header row or document that it is hidden.

Out of scope for a first PR:

- clicks and hover on markers (VS Code's peek diff, IntelliJ's `ActiveGutterRenderer`, Zed's hitboxes);
- hollow and filled variants for staged hunks (Zed's `hunk_style`, VS Code's `pattern`);
- a minimap facet.

Each can come later without changing the collection shape.

### 6.3 Where two separate APIs agree or conflict with the mature editors

**Conflicts:**

- Monaco's data model, the VS Code extension API and VS Code's own quick diff all use **one decoration per hunk carrying both facets**. With Kit's two APIs, an application calls `set` on two collections with the same ranges, and Kit tracks two copies.
- VS Code's `scm.diffDecorations: all | gutter | overview` toggles facets inside one decoration. In Kit's design that becomes "create or don't create that collection", which is simpler, but the API stops looking like Monaco's at the type level.

**Agrees:**

- **Kit's own design.** `TextDecoration` and `RangeDecoration` are already separate collections, one per paint kind. A gutter collection and an overview collection extend that pattern consistently.
- **IntelliJ.** `RangeHighlighter` *can* carry a line marker and an error-stripe mark together, but its VCS renderer deliberately uses one whole-document gutter highlighter and per-range stripe highlighters. It has a stripe-only switch, `shouldPaintErrorStripeMarkers`.
- **Zed.** Separate paths and separate settings per surface.
- **Many sources appear on only one surface.** The ruler shows find matches, selection and symbol occurrences, diagnostics and cursors, none of which has a gutter bar. Gutter items such as fold chevrons have no ruler mark. Even in Monaco, only SCM sets both facets on one decoration. Find and diagnostics use the ruler with inline styles and no gutter facet.

**Net:** two APIs is a defensible, common choice. To still "follow Monaco", reuse its vocabulary (overview ruler, `OverviewRulerLane` with its lane conventions, whole-line coverage, collapsed ranges for deletions) and its tracked-collection contract. The one thing to state in the PR is that Kit splits Monaco's facets into per-surface collections, as it already did for text styles versus range geometry.

---

## Sources

VS Code / Monaco (commit `4bda7217`):
- `src/vs/editor/common/model.ts`: `IModelDecorationOptions`, `OverviewRulerLane`, `GlyphMarginLane`, `TrackedRangeStickiness`. https://github.com/microsoft/vscode/blob/4bda7217fc83e8e838b1edd9ebaf21bb9b498c7e/src/vs/editor/common/model.ts
- `src/vs/editor/common/editorCommon.ts`: `IEditorDecorationsCollection`. https://github.com/microsoft/vscode/blob/4bda7217fc83e8e838b1edd9ebaf21bb9b498c7e/src/vs/editor/common/editorCommon.ts
- `src/vs/workbench/contrib/scm/browser/quickDiffDecorator.ts`. https://github.com/microsoft/vscode/blob/4bda7217fc83e8e838b1edd9ebaf21bb9b498c7e/src/vs/workbench/contrib/scm/browser/quickDiffDecorator.ts
- `src/vs/workbench/contrib/scm/browser/media/dirtydiffDecorator.css`. https://github.com/microsoft/vscode/blob/4bda7217fc83e8e838b1edd9ebaf21bb9b498c7e/src/vs/workbench/contrib/scm/browser/media/dirtydiffDecorator.css
- `src/vs/workbench/contrib/scm/common/quickDiff.ts` (colour ids, `getChangeType`). https://github.com/microsoft/vscode/blob/4bda7217fc83e8e838b1edd9ebaf21bb9b498c7e/src/vs/workbench/contrib/scm/common/quickDiff.ts
- `src/vs/workbench/contrib/scm/browser/quickDiffModel.ts` (200 ms throttle). https://github.com/microsoft/vscode/blob/4bda7217fc83e8e838b1edd9ebaf21bb9b498c7e/src/vs/workbench/contrib/scm/browser/quickDiffModel.ts
- `src/vs/workbench/contrib/scm/browser/scm.contribution.ts` (`scm.diffDecorations*` settings). https://github.com/microsoft/vscode/blob/4bda7217fc83e8e838b1edd9ebaf21bb9b498c7e/src/vs/workbench/contrib/scm/browser/scm.contribution.ts
- `src/vs/editor/browser/widget/diffEditor/diffEditorWidget.ts` (`toLineChanges`). https://github.com/microsoft/vscode/blob/4bda7217fc83e8e838b1edd9ebaf21bb9b498c7e/src/vs/editor/browser/widget/diffEditor/diffEditorWidget.ts
- `src/vs/editor/common/model/textModel.ts` (`_validateRangeRelaxedNoAllocations`). https://github.com/microsoft/vscode/blob/4bda7217fc83e8e838b1edd9ebaf21bb9b498c7e/src/vs/editor/common/model/textModel.ts
- `src/vs/editor/common/config/editorOptions.ts` (layout, `lineDecorationsWidth`, `overviewRulerLanes`, scrollbar sizes). https://github.com/microsoft/vscode/blob/4bda7217fc83e8e838b1edd9ebaf21bb9b498c7e/src/vs/editor/common/config/editorOptions.ts
- `src/vs/editor/browser/viewParts/linesDecorations/linesDecorations.ts`. https://github.com/microsoft/vscode/blob/4bda7217fc83e8e838b1edd9ebaf21bb9b498c7e/src/vs/editor/browser/viewParts/linesDecorations/linesDecorations.ts
- `src/vs/editor/browser/viewParts/overviewRuler/decorationsOverviewRuler.ts`. https://github.com/microsoft/vscode/blob/4bda7217fc83e8e838b1edd9ebaf21bb9b498c7e/src/vs/editor/browser/viewParts/overviewRuler/decorationsOverviewRuler.ts
- `src/vs/editor/common/viewModel/overviewZoneManager.ts`. https://github.com/microsoft/vscode/blob/4bda7217fc83e8e838b1edd9ebaf21bb9b498c7e/src/vs/editor/common/viewModel/overviewZoneManager.ts
- `src/vs/editor/browser/view.ts` and `src/vs/base/browser/ui/scrollbar/scrollableElement.ts` (ruler inserted before the scrollbar). https://github.com/microsoft/vscode/blob/4bda7217fc83e8e838b1edd9ebaf21bb9b498c7e/src/vs/editor/browser/view.ts and https://github.com/microsoft/vscode/blob/4bda7217fc83e8e838b1edd9ebaf21bb9b498c7e/src/vs/base/browser/ui/scrollbar/scrollableElement.ts
- `src/vs/editor/contrib/find/browser/findDecorations.ts` (Center lane) and `src/vs/editor/common/services/markerDecorationsService.ts` (Right lane). https://github.com/microsoft/vscode/blob/4bda7217fc83e8e838b1edd9ebaf21bb9b498c7e/src/vs/editor/contrib/find/browser/findDecorations.ts and https://github.com/microsoft/vscode/blob/4bda7217fc83e8e838b1edd9ebaf21bb9b498c7e/src/vs/editor/common/services/markerDecorationsService.ts
- `src/vs/editor/contrib/folding/browser/foldingDecorations.ts`, `folding.css`. https://github.com/microsoft/vscode/blob/4bda7217fc83e8e838b1edd9ebaf21bb9b498c7e/src/vs/editor/contrib/folding/browser/foldingDecorations.ts
- Monaco typedoc: https://microsoft.github.io/monaco-editor/typedoc/interfaces/editor_editor_api.editor.IModelDecorationOptions.html, https://microsoft.github.io/monaco-editor/typedoc/enums/editor_editor_api.editor.OverviewRulerLane.html, https://microsoft.github.io/monaco-editor/typedoc/interfaces/editor_editor_api.editor.IEditorDecorationsCollection.html
- VS Code extension API `DecorationRenderOptions`: https://code.visualstudio.com/api/references/vscode-api#DecorationRenderOptions

IntelliJ Platform (commit `43722b26`):
- `platform/editor-ui-api/src/com/intellij/openapi/editor/markup/RangeHighlighter.java`, `LineMarkerRenderer.java`, `ActiveGutterRenderer.java`. https://github.com/JetBrains/intellij-community/blob/43722b268dbc6359441135e1895a63d0fbb15fca/platform/editor-ui-api/src/com/intellij/openapi/editor/markup/RangeHighlighter.java
- `platform/platform-impl/src/com/intellij/openapi/editor/markup/LineMarkerRendererEx.java`. https://github.com/JetBrains/intellij-community/blob/43722b268dbc6359441135e1895a63d0fbb15fca/platform/platform-impl/src/com/intellij/openapi/editor/markup/LineMarkerRendererEx.java
- `platform/core-api/src/com/intellij/openapi/editor/RangeMarker.java`. https://github.com/JetBrains/intellij-community/blob/43722b268dbc6359441135e1895a63d0fbb15fca/platform/core-api/src/com/intellij/openapi/editor/RangeMarker.java
- `platform/diff-impl/src/com/intellij/openapi/vcs/ex/LineStatusMarkerRenderer.kt`, `LineStatusGutterMarkerRenderer.kt`, `ActiveLineStatusGutterMarkerRenderer.kt`, `DocumentTracker.kt`. https://github.com/JetBrains/intellij-community/blob/43722b268dbc6359441135e1895a63d0fbb15fca/platform/diff-impl/src/com/intellij/openapi/vcs/ex/LineStatusMarkerRenderer.kt
- `platform/diff-impl/src/com/intellij/openapi/diff/LineStatusMarkerDrawUtil.java`, `LineStatusMarkerColorScheme.kt`. https://github.com/JetBrains/intellij-community/blob/43722b268dbc6359441135e1895a63d0fbb15fca/platform/diff-impl/src/com/intellij/openapi/diff/LineStatusMarkerDrawUtil.java
- `platform/platform-impl/src/com/intellij/openapi/editor/impl/EditorMarkupModelImpl.kt` (error stripe), `EditorGutterLayout.java`, `EditorGutterComponentImpl.java`. https://github.com/JetBrains/intellij-community/blob/43722b268dbc6359441135e1895a63d0fbb15fca/platform/platform-impl/src/com/intellij/openapi/editor/impl/EditorMarkupModelImpl.kt
- `platform/analysis-impl/src/com/intellij/codeInsight/daemon/DaemonCodeAnalyzerSettings.java` (`errorStripeMarkMinHeight = 2`). https://github.com/JetBrains/intellij-community/blob/43722b268dbc6359441135e1895a63d0fbb15fca/platform/analysis-impl/src/com/intellij/codeInsight/daemon/DaemonCodeAnalyzerSettings.java

Zed (commit `ec86047d`, GPL-3.0-or-later, read for approach only):
- `assets/settings/default.json` (`scrollbar.*`, `gutter.git_gutter_width`, `git.git_gutter`, `git.gutter_debounce`, `git.hunk_style`). https://github.com/zed-industries/zed/blob/ec86047d67acd6fddd81392f44ed6ebcc6e7f531/assets/settings/default.json
- `crates/editor/src/element.rs` (gutter hunks, scrollbar markers). https://github.com/zed-industries/zed/blob/ec86047d67acd6fddd81392f44ed6ebcc6e7f531/crates/editor/src/element.rs
- `crates/editor/src/git.rs` (`DisplayDiffHunk`, `display_diff_hunks_for_rows`). https://github.com/zed-industries/zed/blob/ec86047d67acd6fddd81392f44ed6ebcc6e7f531/crates/editor/src/git.rs
- `crates/buffer_diff/src/buffer_diff.rs` (`DiffHunk.buffer_range: Range<Anchor>`). https://github.com/zed-industries/zed/blob/ec86047d67acd6fddd81392f44ed6ebcc6e7f531/crates/buffer_diff/src/buffer_diff.rs
- Settings docs: https://zed.dev/docs/configuring-zed

CodeMirror 6:
- `@codemirror/view` `src/gutter.ts`. https://github.com/codemirror/view/blob/fbff59ba004d80d8c914f64c42586387b08706ac/src/gutter.ts
- Reference manual: https://codemirror.net/docs/ref/#view.gutter
- `@codemirror/merge` `src/deco.ts`, `src/theme.ts`, `src/unified.ts`. https://github.com/codemirror/merge/blob/3bc269331394691dcce9784ba8c93b318d15535f/src/deco.ts
- Forum, "show location of matched search query in scrollbar?" (2023-05): https://discuss.codemirror.net/t/show-location-of-matched-search-query-in-scrollbar/6463

GPUI Kit:
- `gpui-base` 0.8.0 `src/input/editor/decorations.rs` (local registry copy; identical on `main` `64e6aaff`).
- `crates/base/src/input/base/element.rs` on `main` (`layout_line_numbers`, `layout_fold_icons`, `EditorScrollbar`). https://github.com/longbridge/gpui-kit/blob/64e6aaffbcd3760fc9d000f86655495c4978931c/crates/base/src/input/base/element.rs
- #3644 "input: Let an editor supply its own scrollbar" (`crates/base/src/input/scrollbar_presentation.rs`). https://github.com/longbridge/gpui-kit/blob/64e6aaffbcd3760fc9d000f86655495c4978931c/crates/base/src/input/scrollbar_presentation.rs
