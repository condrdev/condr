package dev.condr.companion

import android.graphics.Bitmap
import android.graphics.Canvas
import android.graphics.Typeface
import androidx.core.content.res.ResourcesCompat
import androidx.test.core.app.ApplicationProvider
import dev.condr.companion.ui.terminal.CellMetrics
import dev.condr.companion.ui.terminal.drawTerminal
import org.junit.Assert.assertEquals
import org.junit.Assert.assertNotEquals
import org.junit.Assert.assertTrue
import org.junit.Test
import uniffi.condr_mobile.ScreenCell
import uniffi.condr_mobile.ScreenCursor
import uniffi.condr_mobile.TerminalScreen

/** The grid the renderer draws from the Server's cells (ADR 0048). */
class RendererTest {
  private val context = ApplicationProvider.getApplicationContext<android.content.Context>()
  private val regular =
    ResourcesCompat.getFont(context, R.font.jetbrains_mono) ?: Typeface.MONOSPACE
  private val italic =
    ResourcesCompat.getFont(context, R.font.jetbrains_mono_italic) ?: Typeface.MONOSPACE
  private val textSize = 40f
  private val metrics = CellMetrics(regular, textSize)

  private fun cell(text: String, fg: UInt = 0xC9D1D9u, bg: UInt = 0x0D1117u) =
    ScreenCell(text, fg, bg, bold = false, italic = false, underline = false, link = null)

  private fun screen(
    cells: List<ScreenCell>,
    columns: Int,
    rows: Int,
    cursor: ScreenCursor? = null,
  ) =
    TerminalScreen(
      1u,
      rows.toUShort(),
      columns.toUShort(),
      0u,
      cells,
      cursor,
      0x0D1117u,
      false,
      false,
    )

  private fun draw(screen: TerminalScreen, rows: Int, columns: Int): Bitmap {
    val bitmap =
      Bitmap.createBitmap(
        (columns * metrics.width).toInt() + 1,
        (rows * metrics.height).toInt() + 1,
        Bitmap.Config.ARGB_8888,
      )
    bitmap.eraseColor(0xFF0D1117.toInt())
    drawTerminal(Canvas(bitmap), screen, rows, columns, regular, italic, metrics, textSize)
    return bitmap
  }

  private fun Bitmap.cellHasInk(row: Int, column: Int): Boolean {
    val x0 = (column * metrics.width).toInt()
    val y0 = (row * metrics.height).toInt()
    for (y in y0 until minOf(y0 + metrics.height.toInt(), height)) for (x in
      x0 until minOf(x0 + metrics.width.toInt(), width)) {
      if (getPixel(x, y) != 0xFF0D1117.toInt()) return true
    }
    return false
  }

  @Test
  fun theGridAViewportGivesIsWhatTheServerIsSent() {
    val columns = (1000f / metrics.width).toInt()
    val rows = (600f / metrics.height).toInt()
    assertTrue(columns in 20..60)
    assertTrue(rows in 5..30)
    assertEquals(metrics.width, CellMetrics(regular, textSize).width, 0.01f)
  }

  @Test
  fun aWideCharacterTakesTwoColumnsAndItsSpacerDrawsNothing() {
    val cells = listOf(cell("漢"), cell(""), cell("a"), cell(" "))
    val bitmap = draw(screen(cells, 4, 1), 1, 4)
    assertTrue(bitmap.cellHasInk(0, 0) || bitmap.cellHasInk(0, 1))
    assertTrue(bitmap.cellHasInk(0, 2))
    assertTrue(!bitmap.cellHasInk(0, 3))
  }

  @Test
  fun selectionColoursComeFromTheCellsAsSent() {
    val selected = cell(" ", fg = 0x0D1117u, bg = 0xE6EDF3u)
    val bitmap = draw(screen(listOf(cell(" "), selected), 2, 1), 1, 2)
    assertEquals(0xFF0D1117.toInt(), bitmap.getPixel(2, 2))
    assertEquals(0xFFE6EDF3.toInt(), bitmap.getPixel(metrics.width.toInt() + 2, 2))
  }

  @Test
  fun theCursorIsDrawnInItsColourOverItsCell() {
    val bitmap =
      draw(screen(listOf(cell("x"), cell(" ")), 2, 1, ScreenCursor(0u, 1u, 0xF0F6FCu)), 1, 2)
    assertEquals(0xFFF0F6FC.toInt(), bitmap.getPixel(metrics.width.toInt() + 2, 2))
    assertNotEquals(0xFFF0F6FC.toInt(), bitmap.getPixel(2, 2))
  }
}
