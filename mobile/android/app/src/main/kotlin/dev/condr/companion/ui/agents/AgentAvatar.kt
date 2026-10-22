package dev.condr.companion.ui.agents

import androidx.compose.foundation.background
import androidx.compose.foundation.border
import androidx.compose.foundation.layout.Box
import androidx.compose.foundation.layout.size
import androidx.compose.foundation.shape.CircleShape
import androidx.compose.foundation.shape.RoundedCornerShape
import androidx.compose.material3.Icon
import androidx.compose.material3.MaterialTheme
import androidx.compose.material3.Text
import androidx.compose.runtime.Composable
import androidx.compose.ui.Alignment
import androidx.compose.ui.Modifier
import androidx.compose.ui.res.painterResource
import androidx.compose.ui.text.font.FontWeight
import androidx.compose.ui.unit.Dp
import androidx.compose.ui.unit.dp
import dev.condr.companion.R
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
      "kilo" to R.drawable.agent_kilo,
      "kimi" to R.drawable.agent_kimi,
      "omp" to R.drawable.agent_omp,
      "opencode" to R.drawable.agent_opencode,
      "pi" to R.drawable.agent_pi,
      "qoder" to R.drawable.agent_qoder,
      "qwen" to R.drawable.agent_qwen,
    )

  /** The mark for the Agent's kind, or none for a kind this build has no mark for. */
  fun resource(kind: String?): Int? = kind?.let { marks[it] }
}

/** An Agent's mark on a rounded square, its state as a dot at the corner. */
@Composable
fun AgentAvatar(kind: String?, status: AgentStatus, side: Dp, shell: Boolean = false) {
  Box(Modifier.size(side)) {
    Box(
      Modifier.size(side)
        .background(
          MaterialTheme.colorScheme.surfaceContainerHighest,
          RoundedCornerShape(side * 0.3f),
        ),
      contentAlignment = Alignment.Center,
    ) {
      val mark = AgentMarks.resource(kind)
      if (shell) {
        Icon(
          painterResource(R.drawable.ic_terminal),
          contentDescription = null,
          tint = MaterialTheme.colorScheme.onSurfaceVariant,
          modifier = Modifier.size(side * 0.55f),
        )
      } else if (mark != null) {
        Icon(
          painterResource(mark),
          contentDescription = null,
          tint = MaterialTheme.colorScheme.onSurface,
          modifier = Modifier.size(side * 0.55f),
        )
      } else {
        Text(
          (kind ?: "?").take(1).uppercase(),
          style = MaterialTheme.typography.titleMedium,
          fontWeight = FontWeight.Bold,
          color = MaterialTheme.colorScheme.onSurfaceVariant,
        )
      }
    }
    if (!shell) {
      Box(
        Modifier.align(Alignment.BottomEnd)
          .size(side * 0.32f)
          .border(2.dp, MaterialTheme.colorScheme.surface, CircleShape)
          .background(status.tint, CircleShape)
      )
    }
  }
}
