package dev.condr.companion.ui.terminal

import android.graphics.Bitmap
import android.graphics.BitmapFactory
import android.graphics.Paint
import android.graphics.Rect
import android.graphics.RectF
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
import kotlin.math.roundToInt
import uniffi.condr_mobile.ScreenCell
import uniffi.condr_mobile.ScreenImage
import uniffi.condr_mobile.ScreenImageLayer
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
  /** A terminal image's PNG by its key, once fetched (ADR 0065). */
  image: (String) -> ByteArray? = { null },
  /** The grid at this size and font, and a cell in pixels. */
  onGrid: (rows: Int, columns: Int, cellWidth: Int, cellHeight: Int) -> Unit,
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
  // Each image decoded once while it stays on screen.
  val bitmaps = remember { mutableMapOf<String, Bitmap>() }
  // The gestures outlive a composition: they act on the screen and handlers of the latest one.
  val latestScreen by rememberUpdatedState(screen)
  val latestMetrics by rememberUpdatedState(metrics)
  val latestTouches by rememberUpdatedState(touches)
  BoxWithConstraints(modifier) {
    val width = with(density) { maxWidth.toPx() }
    val height = with(density) { maxHeight.toPx() }
    val columns = floor(width / metrics.width).toInt().coerceAtLeast(1)
    val rows = floor(height / metrics.height).toInt().coerceAtLeast(1)
    LaunchedEffect(rows, columns, metrics) {
      onGrid(rows, columns, metrics.width.roundToInt(), metrics.height.roundToInt())
    }

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
      bitmaps.keys.retainAll(screen.images.map { it.key }.toSet())
      for (placement in screen.images) {
        if (placement.key in bitmaps) continue
        val png = image(placement.key) ?: continue
        BitmapFactory.decodeByteArray(png, 0, png.size)?.let { bitmaps[placement.key] = it }
      }
      drawIntoCanvas { canvas ->
        Renderer(faces, metrics, textSize)
          .draw(canvas.nativeCanvas, screen, rows, columns, bitmaps::get)
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

  private val pixels = Paint(Paint.FILTER_BITMAP_FLAG)

  /**
   * Images under the cell backgrounds, the backgrounds, images under the text, the text, images
   * over it, then the cursor (ADR 0065).
   */
  fun draw(
    canvas: android.graphics.Canvas,
    screen: TerminalScreen,
    rows: Int,
    columns: Int,
    bitmap: (String) -> Bitmap? = { null },
  ) {
    val width = screen.columns.toInt()
    val visibleRows = minOf(screen.rows.toInt(), rows)
    val visibleColumns = minOf(width, columns)
    val cells = screen.cells
    canvas.save()
    canvas.clipRect(0f, 0f, visibleColumns * metrics.width, visibleRows * metrics.height)
    images(canvas, screen, ScreenImageLayer.BELOW_BACKGROUND, bitmap)
    for (row in 0 until visibleRows) {
      val top = row * metrics.height
      val start = row * width
      // Backgrounds as runs of one colour.
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
    }
    images(canvas, screen, ScreenImageLayer.BELOW_TEXT, bitmap)
    for (row in 0 until visibleRows) {
      val top = row * metrics.height
      val start = row * width
      // The text as runs of one style; a cell that is not plain ASCII is drawn on its own so the
      // font's fallback for it cannot shift the cells after it.
      var column = 0
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
    images(canvas, screen, ScreenImageLayer.ABOVE_TEXT, bitmap)
    canvas.restore()
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

  private fun images(
    canvas: android.graphics.Canvas,
    screen: TerminalScreen,
    layer: ScreenImageLayer,
    bitmap: (String) -> Bitmap?,
  ) {
    for (placement in screen.images) {
      if (placement.layer != layer) continue
      val image = bitmap(placement.key) ?: continue
      canvas.drawBitmap(image, source(placement, image), destination(placement), pixels)
    }
  }

  private fun source(placement: ScreenImage, image: Bitmap) =
    Rect(
      (placement.sourceX * image.width).roundToInt(),
      (placement.sourceY * image.height).roundToInt(),
      ((placement.sourceX + placement.sourceWidth) * image.width).roundToInt(),
      ((placement.sourceY + placement.sourceHeight) * image.height).roundToInt(),
    )

  private fun destination(placement: ScreenImage) =
    RectF(
      (placement.column * metrics.width).toFloat(),
      (placement.row * metrics.height).toFloat(),
      ((placement.column + placement.width) * metrics.width).toFloat(),
      ((placement.row + placement.height) * metrics.height).toFloat(),
    )

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
