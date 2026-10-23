package dev.condr.companion.ui.agents

import androidx.compose.foundation.background
import androidx.compose.foundation.layout.Box
import androidx.compose.foundation.layout.offset
import androidx.compose.foundation.layout.padding
import androidx.compose.foundation.layout.size
import androidx.compose.foundation.shape.CircleShape
import androidx.compose.foundation.shape.RoundedCornerShape
import androidx.compose.material3.Icon
import androidx.compose.material3.MaterialTheme
import androidx.compose.runtime.Composable
import androidx.compose.ui.Alignment
import androidx.compose.ui.Modifier
import androidx.compose.ui.graphics.Color
import androidx.compose.ui.res.painterResource
import androidx.compose.ui.unit.Dp
import dev.condr.companion.R
import dev.condr.companion.ui.theme.StatusGlyph
import dev.condr.companion.ui.theme.tint
import uniffi.condr_mobile.AgentStatus

/** The marks from `assets/agents`, as script/generate-android-agent-marks.py drew them. */
object AgentMarks {
  private val marks =
    mapOf(
      "antigravity" to R.drawable.agent_antigravity,
      "claude" to R.drawable.agent_claude,
      "codex" to R.drawable.agent_codex,
      "copilot" to R.drawable.agent_copilot,
      "cursor" to R.drawable.agent_cursor,
      "grok" to R.drawable.agent_grok,
      "kimi" to R.drawable.agent_kimi,
      "omp" to R.drawable.agent_omp,
      "opencode" to R.drawable.agent_opencode,
      "pi" to R.drawable.agent_pi,
    )

  /** The mark for the Agent's kind, or none for a kind this build has no mark for. */
  fun resource(kind: String?): Int? = kind?.let { marks[it] }
}

/**
 * A mark on a soft tile with its status glyph on a disc of the surface's colour, so a row reads by
 * Agent first and by state second, as the desktop sidebar does. The tile is a tint over whatever it
 * sits on, so it shows on a card as on the page; a kind without a mark, and a shell, get the
 * terminal symbol.
 */
@Composable
fun AgentAvatar(
  kind: String?,
  status: AgentStatus,
  side: Dp,
  shell: Boolean = false,
  surface: Color = MaterialTheme.colorScheme.surface,
  /** A Blocked glyph pulses where one wait is the subject: a banner. */
  ripple: Boolean = false,
) {
  Box(Modifier.size(side)) {
    Box(
      Modifier.size(side)
        .background(
          MaterialTheme.colorScheme.onSurface.copy(alpha = 0.08f),
          RoundedCornerShape(side * 0.3f),
        ),
      contentAlignment = Alignment.Center,
    ) {
      val mark = if (shell) null else AgentMarks.resource(kind)
      Icon(
        painterResource(mark ?: R.drawable.ic_terminal),
        contentDescription = null,
        tint =
          if (mark != null) MaterialTheme.colorScheme.onSurface
          else MaterialTheme.colorScheme.onSurfaceVariant,
        modifier = Modifier.size(side * 0.55f),
      )
    }
    if (!shell) {
      Box(
        Modifier.align(Alignment.BottomEnd)
          .offset(side * 0.12f, side * 0.12f)
          .background(surface, CircleShape)
          .padding(side * 0.06f)
      ) {
        StatusGlyph(status, side * 0.27f, ripple = ripple)
      }
    }
  }
}
