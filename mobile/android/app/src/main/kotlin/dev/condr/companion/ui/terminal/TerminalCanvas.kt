package dev.condr.companion.ui.terminal

import android.graphics.Paint
import android.graphics.Typeface
import androidx.compose.foundation.Canvas
import androidx.compose.foundation.gestures.detectDragGesturesAfterLongPress
import androidx.compose.foundation.gestures.detectTapGestures
import androidx.compose.foundation.gestures.detectVerticalDragGestures
import androidx.compose.foundation.layout.BoxWithConstraints
import androidx.compose.foundation.layout.fillMaxSize
import androidx.compose.runtime.Composable
import androidx.compose.runtime.LaunchedEffect
import androidx.compose.runtime.getValue
import androidx.compose.runtime.remember
import androidx.compose.runtime.rememberUpdatedState
import androidx.compose.ui.Modifier
import androidx.compose.ui.geometry.Offset
import androidx.compose.ui.graphics.drawscope.drawIntoCanvas
import androidx.compose.ui.graphics.nativeCanvas
import androidx.compose.ui.hapticfeedback.HapticFeedbackType
import androidx.compose.ui.input.pointer.pointerInput
import androidx.compose.ui.platform.LocalContext
import androidx.compose.ui.platform.LocalDensity
import androidx.compose.ui.platform.LocalHapticFeedback
import androidx.core.content.res.ResourcesCompat
import dev.condr.companion.R
import kotlin.math.abs
import kotlin.math.floor
import uniffi.condr_mobile.ScreenCell
import uniffi.condr_mobile.TerminalScreen

/** A cell's place on the grid. */
data class Cell(val row: UShort, val column: UShort)

/** What the terminal's touches mean (ADR 0041); the page hands them to the Session. */
class TerminalTouches(
  /** A tap that hit a selection, a link, a mouse-tracking program, or nothing: in that order. */
  val tap: (Cell, ScreenCell?) -> Unit,
  val longPress: (Cell) -> Unit,
  val drag: (from: Cell, to: Cell) -> Unit,
  val released: (Offset) -> Unit,
  /** Wheel motion at the finger, positive towards what came before. */
  val wheel: (Cell, Int) -> Unit,
  /** A swipe started. */
  val swiped: () -> Unit,
)

/**
 * The grid, drawn from the Server's cells with the framework `Paint` (ADR 0048): a run of one style
 * at a time, every cell its own width, the selection already in its colours, the cursor last. The
 * whole viewport is redrawn on each frame.
 */
// ponytail: whole-viewport redraw per frame; damage by row when a measurement asks for it.
@Composable
fun TerminalCanvas(
  screen: TerminalScreen?,
  fontSizeSp: Float,
  touches: TerminalTouches,
  modifier: Modifier = Modifier,
  onGrid: (rows: Int, columns: Int) -> Unit,
) {
  val context = LocalContext.current
  val density = LocalDensity.current
  val haptics = LocalHapticFeedback.current
  val faces = remember {
    val regular = ResourcesCompat.getFont(context, R.font.jetbrains_mono) ?: Typeface.MONOSPACE
    val italic =
      ResourcesCompat.getFont(context, R.font.jetbrains_mono_italic) ?: Typeface.MONOSPACE
    Faces(regular, italic)
  }
  val textSize = with(density) { fontSizeSp.sp.toPx() }
  val metrics = remember(textSize) { CellMetrics(faces.regular, textSize) }
  // The gestures outlive a composition: they act on the screen and handlers of the latest one.
  val latestScreen by rememberUpdatedState(screen)
  val latestMetrics by rememberUpdatedState(metrics)
  val latestTouches by rememberUpdatedState(touches)
  BoxWithConstraints(modifier) {
    val width = with(density) { maxWidth.toPx() }
    val height = with(density) { maxHeight.toPx() }
    val columns = floor(width / metrics.width).toInt().coerceAtLeast(1)
    val rows = floor(height / metrics.height).toInt().coerceAtLeast(1)
    LaunchedEffect(rows, columns) { onGrid(rows, columns) }

    fun cell(at: Offset): Cell? {
      val screen = latestScreen ?: return null
      val metrics = latestMetrics
      val row = floor(at.y / metrics.height).toInt().coerceIn(0, screen.rows.toInt() - 1)
      val column = floor(at.x / metrics.width).toInt().coerceIn(0, screen.columns.toInt() - 1)
      return Cell(row.toUShort(), column.toUShort())
    }

    Canvas(
      Modifier.fillMaxSize()
        .pointerInput(Unit) {
          detectTapGestures { at ->
            val hit = cell(at) ?: return@detectTapGestures
            val screen = latestScreen ?: return@detectTapGestures
            val index = hit.row.toInt() * screen.columns.toInt() + hit.column.toInt()
            latestTouches.tap(hit, screen.cells.getOrNull(index))
          }
        }
        .pointerInput(Unit) {
          var anchor: Cell? = null
          var reached: Cell? = null
          var finger = Offset.Zero
          detectDragGesturesAfterLongPress(
            onDragStart = { at ->
              val hit = cell(at) ?: return@detectDragGesturesAfterLongPress
              anchor = hit
              reached = hit
              finger = at
              haptics.performHapticFeedback(HapticFeedbackType.LongPress)
              latestTouches.longPress(hit)
            },
            onDrag = { change, _ ->
              val from = anchor ?: return@detectDragGesturesAfterLongPress
              finger = change.position
              val to = cell(change.position) ?: return@detectDragGesturesAfterLongPress
              if (to != reached) {
                reached = to
                latestTouches.drag(from, to)
              }
            },
            onDragEnd = {
              anchor = null
              latestTouches.released(finger)
            },
            onDragCancel = { anchor = null },
          )
        }
        .pointerInput(Unit) {
          // A swipe is wheel motion at the finger, a line per cell height, a finger moving
          // down showing what came before.
          var dragged = 0f
          detectVerticalDragGestures(
            onDragStart = {
              dragged = 0f
              latestTouches.swiped()
            },
            onDragEnd = { dragged = 0f },
            onDragCancel = { dragged = 0f },
          ) { change, delta ->
            dragged += delta
            val height = latestMetrics.height
            val lines = (abs(dragged) / height).toInt() * if (dragged < 0) -1 else 1
            if (lines != 0) {
              dragged -= lines * height
              cell(change.position)?.let { latestTouches.wheel(it, lines) }
            }
          }
        }
    ) {
      val screen = screen ?: return@Canvas
      drawIntoCanvas { canvas ->
        Renderer(faces, metrics, textSize).draw(canvas.nativeCanvas, screen, rows, columns)
      }
    }
  }
}

private class Faces(val regular: Typeface, val italic: Typeface)

/** Draws `screen` onto `canvas` at `rows` × `columns`, the renderer's one entry point. */
fun drawTerminal(
  canvas: android.graphics.Canvas,
  screen: TerminalScreen,
  rows: Int,
  columns: Int,
  regular: Typeface,
  italic: Typeface,
  metrics: CellMetrics,
  textSize: Float,
) = Renderer(Faces(regular, italic), metrics, textSize).draw(canvas, screen, rows, columns)

/** A cell's size for the font: the width of "W" and the font's line height. */
class CellMetrics(face: Typeface, textSize: Float) {
  val width: Float
  val height: Float
  val ascent: Float

  init {
    val paint =
      Paint(Paint.ANTI_ALIAS_FLAG).apply {
        typeface = face
        this.textSize = textSize
        fontFeatureSettings = TerminalFont.FEATURES
      }
    width = paint.measureText("W")
    val metrics = paint.fontMetrics
    height = metrics.descent - metrics.ascent
    ascent = -metrics.ascent
  }
}

private class Renderer(
  private val faces: Faces,
  private val metrics: CellMetrics,
  textSize: Float,
) {
  private val fill = Paint()
  private val text =
    Paint(Paint.ANTI_ALIAS_FLAG or Paint.SUBPIXEL_TEXT_FLAG).apply {
      this.textSize = textSize
      fontFeatureSettings = TerminalFont.FEATURES
    }
  private val builder = StringBuilder()

  fun draw(canvas: android.graphics.Canvas, screen: TerminalScreen, rows: Int, columns: Int) {
    val width = screen.columns.toInt()
    val visibleRows = minOf(screen.rows.toInt(), rows)
    val visibleColumns = minOf(width, columns)
    val cells = screen.cells
    for (row in 0 until visibleRows) {
      val top = row * metrics.height
      val start = row * width
      // Backgrounds first, as runs of one colour.
      var column = 0
      while (column < visibleColumns) {
        val index = start + column
        if (index >= cells.size) break
        val color = cells[index].background
        var end = column + 1
        while (
          end < visibleColumns && start + end < cells.size && cells[start + end].background == color
        ) end++
        if (color != screen.background) {
          fill.color = argb(color)
          canvas.drawRect(
            column * metrics.width,
            top,
            end * metrics.width,
            top + metrics.height,
            fill,
          )
        }
        column = end
      }
      // Then the text, as runs of one style; a cell that is not plain ASCII is drawn on its own
      // so the font's fallback for it cannot shift the cells after it.
      column = 0
      while (column < visibleColumns) {
        val index = start + column
        if (index >= cells.size) break
        val first = cells[index]
        if (first.text.isEmpty() || first.text == " ") {
          column++
          continue
        }
        var end = column + 1
        val ascii = first.text.length == 1 && first.text[0].code < 0x7f
        if (ascii) {
          while (end < visibleColumns && start + end < cells.size) {
            val next = cells[start + end]
            if (!sameStyle(first, next) || next.text.length != 1 || next.text[0].code >= 0x7f) break
            end++
          }
        }
        builder.setLength(0)
        for (i in column until end) builder.append(cells[start + i].text.ifEmpty { " " })
        style(first)
        canvas.drawText(
          builder,
          0,
          builder.length,
          column * metrics.width,
          top + metrics.ascent,
          text,
        )
        column = end
      }
    }
    val cursor = screen.cursor
    if (
      cursor != null && cursor.row.toInt() < visibleRows && cursor.column.toInt() < visibleColumns
    ) {
      val x = cursor.column.toInt() * metrics.width
      val y = cursor.row.toInt() * metrics.height
      fill.color = argb(cursor.color)
      canvas.drawRect(x, y, x + metrics.width, y + metrics.height, fill)
      val under = cells.getOrNull(cursor.row.toInt() * width + cursor.column.toInt())
      if (under != null && under.text.isNotBlank()) {
        style(under)
        text.color = argb(under.background)
        canvas.drawText(under.text, x, y + metrics.ascent, text)
      }
    }
  }

  private fun sameStyle(a: ScreenCell, b: ScreenCell) =
    a.foreground == b.foreground &&
      a.bold == b.bold &&
      a.italic == b.italic &&
      a.underline == b.underline

  private fun style(cell: ScreenCell) {
    text.typeface = if (cell.italic) faces.italic else faces.regular
    text.fontVariationSettings = if (cell.bold) "'wght' 700" else "'wght' 400"
    text.isUnderlineText = cell.underline
    text.color = argb(cell.foreground)
  }

  private fun argb(rgb: UInt): Int = (0xFF000000u or (rgb and 0xFFFFFFu)).toInt()
}

private val Float.sp: androidx.compose.ui.unit.TextUnit
  get() = androidx.compose.ui.unit.TextUnit(this, androidx.compose.ui.unit.TextUnitType.Sp)
