package dev.condr.companion.demo

import dev.condr.companion.model.needsYou
import uniffi.condr_mobile.AgentStatus
import uniffi.condr_mobile.AgentView
import uniffi.condr_mobile.Change
import uniffi.condr_mobile.CompanionInterface
import uniffi.condr_mobile.DeviceStatus
import uniffi.condr_mobile.DeviceView
import uniffi.condr_mobile.Event
import uniffi.condr_mobile.Key
import uniffi.condr_mobile.MobileException
import uniffi.condr_mobile.NeedsYouItem
import uniffi.condr_mobile.PaneView
import uniffi.condr_mobile.SavedDevice
import uniffi.condr_mobile.ScreenCell
import uniffi.condr_mobile.ScreenCursor
import uniffi.condr_mobile.TerminalScreen
import uniffi.condr_mobile.WorkspaceView

/**
 * What `-e scene <name>` opens, for design review and the UI tests: a world that holds still, drawn
 * by the same screens as the real one.
 */
enum class DemoScene(val id: String) {
  WELCOME("welcome"),
  PAIR("pair"),
  SCAN("scan"),
  NAME("name"),
  CONNECTING("connecting"),
  PAIRED("paired"),
  AGENTS("agents"),
  LOCK("lock"),
  DEVICES("devices"),
  DEVICE("device"),
  OFFLINE("offline"),
  SETTINGS("settings"),
  EMPTY("empty"),
  BANNER("banner"),
  ERROR("error"),
  COMPUTER("computer"),
  TERMINAL("terminal"),
  QUESTION("question"),
  CODEX("codex"),
  SHELL("shell"),
  TERMINAL_ERROR("terminal-error"),
  CROWD("crowd"),
  MANY("many"),
  CROWD_DEVICE("crowd-device");

  /** The pairing scenes start with no Device. */
  val unpaired: Boolean
    get() = this in setOf(WELCOME, PAIR, SCAN, NAME, CONNECTING, PAIRED)

  companion object {
    fun named(id: String?): DemoScene? = entries.firstOrNull { it.id == id }
  }
}

/** Three computers and their Agents, as the prototype drew them. */
object DemoWorld {
  const val MACMINI = "tcp://CxQ2mZ5kL9pR3wT7vY1bN4dH8fJ0sA6gK2eU5iO9rW@192.168.1.20:7417"
  const val JPDEV = "p2p://h7k2x9q4m3v8w1n6c5b0z2r7t4y9u3p8"
  const val STUDIO = "tcp://K8pL2mQ5rT9wX3zB6cF1gH4jN7sV0yA3dE6iU9oR2tW@studio.local:7417"
  const val BUILDBOX = "p2p://k9m2c4x7v1b8n3q6w5e0r2t9y4u7i1o3"
  const val GPU = "p2p://g4p8u1x6c3n9m2v7b5z0q4w8e1r6t3y9"
  const val FINGERPRINT = "q3Zr8wX0bH4nT1sVd9cK2mLp7fYg5jQe6uRa0iWo-Lk"

  private val labels = mapOf("claude" to "Claude", "codex" to "Codex", "opencode" to "OpenCode")

  fun agent(kind: String, status: AgentStatus, detail: String? = null) =
    AgentView(kind, labels[kind] ?: kind, status, detail)

  fun pane(id: Long, title: String, agent: AgentView? = null) =
    PaneView(id.toULong(), title, agent, attention = false, exited = false)

  val devices =
    listOf(
      DeviceView(MACMINI, "macmini", "macmini", DeviceStatus.Connected),
      DeviceView(JPDEV, "jpdev", "jpdev", DeviceStatus.Connected),
      DeviceView(
        STUDIO,
        "studio",
        "studio",
        DeviceStatus.Failed("No answer from studio.local. Is Condr running there?"),
      ),
    )

  val workspaces: Map<String, List<WorkspaceView>> =
    mapOf(
      MACMINI to
        listOf(
          WorkspaceView(
            1u,
            "condr",
            "~/Developer/project/condr",
            listOf(
              pane(
                1,
                "Implement statistics page",
                agent("claude", AgentStatus.BLOCKED, "Bash: cargo test -p condr-client"),
              ),
              pane(2, "Fix sidebar flicker", agent("codex", AgentStatus.WORKING)),
              pane(3, "condr"),
            ),
          ),
          WorkspaceView(
            2u,
            "rime-config",
            "~/rime-config",
            listOf(pane(4, "Double pinyin schema", agent("opencode", AgentStatus.DONE))),
          ),
        ),
      JPDEV to
        listOf(
          WorkspaceView(
            11u,
            "admin-dashboard",
            "~/work/admin-dashboard",
            listOf(
              pane(
                11,
                "Add usage chart",
                agent(
                  "claude",
                  AgentStatus.BLOCKED,
                  "Which chart library should the dashboard use?",
                ),
              ),
              pane(12, "Migrate auth to passkeys", agent("claude", AgentStatus.WORKING)),
            ),
          ),
          WorkspaceView(
            12u,
            "condr-mobile",
            "~/work/condr-mobile",
            listOf(pane(13, "Pairing screen copy", agent("codex", AgentStatus.IDLE))),
          ),
        ),
      STUDIO to emptyList(),
    )

  data class World(val devices: List<DeviceView>, val workspaces: Map<String, List<WorkspaceView>>)

  /** One computer running fifteen Agents beside the usual two, to try the design at scale. */
  val crowd: World by lazy {
    val box = DeviceView(BUILDBOX, "buildbox", "buildbox", DeviceStatus.Connected)
    val w = AgentStatus.WORKING
    val workspaces =
      listOf(
        WorkspaceView(
          21u,
          "api",
          "~/src/api",
          listOf(
            pane(
              21,
              "Rate-limit the public API",
              agent("claude", AgentStatus.BLOCKED, "Bash: cargo test -p api --features limits"),
            ),
            pane(22, "Paginate the audit log", agent("codex", w)),
            pane(23, "Upgrade axum to 0.9", agent("claude", w)),
            pane(24, "Trace slow queries", agent("codex", w)),
            pane(25, "Retry failed webhooks", agent("codex", AgentStatus.IDLE)),
          ),
        ),
        WorkspaceView(
          22u,
          "web",
          "~/src/web",
          listOf(
            pane(26, "Dark mode for settings", agent("claude", w)),
            pane(27, "Fix the hydration warning", agent("opencode", AgentStatus.DONE)),
            pane(
              28,
              "Onboarding checklist",
              agent("claude", AgentStatus.BLOCKED, "Should the checklist be dismissible?"),
            ),
            pane(29, "Storybook for buttons", agent("codex", w)),
            pane(30, "Lazy-load the charts", agent("claude", w)),
          ),
        ),
        WorkspaceView(
          23u,
          "infra",
          "~/src/infra",
          listOf(
            pane(31, "Terraform the staging database", agent("claude", w)),
            pane(32, "Rotate CI secrets", agent("codex", w)),
            pane(33, "Monthly cost report", agent("codex", AgentStatus.IDLE)),
            pane(34, "Pin the base images", agent("codex", w)),
            pane(35, "Alert on disk usage", agent("codex", AgentStatus.IDLE)),
          ),
        ),
      )
    World(
      listOf(box, devices[0], devices[2]),
      mapOf(
        BUILDBOX to workspaces,
        MACMINI to (this.workspaces[MACMINI] ?: emptyList()),
        STUDIO to emptyList(),
      ),
    )
  }

  /** Five computers, one running twenty-four Agents: the score past the screen's width. */
  val many: World by lazy {
    val tasks =
      listOf(
        "Shard the embeddings job",
        "Profile the tokenizer",
        "Resume failed epochs",
        "Cache the eval set",
        "Quantize the 8B model",
        "Fix NaN in layer norm",
        "Compare optimizers",
        "Prune dead checkpoints",
      )
    val mix =
      listOf(AgentStatus.BLOCKED, AgentStatus.DONE, AgentStatus.DONE) +
        List(14) { AgentStatus.WORKING } +
        List(7) { AgentStatus.IDLE }
    val panes = mix.mapIndexed { index, status ->
      val suffix = if (index < tasks.size) "" else " #${index / tasks.size + 1}"
      pane(
        100L + index,
        "${tasks[index % tasks.size]}$suffix",
        agent(
          if (index % 3 == 0) "codex" else "claude",
          status,
          if (status == AgentStatus.BLOCKED) "Bash: python train.py --resume runs/0412" else null,
        ),
      )
    }
    val workspaces = HashMap(this.workspaces)
    workspaces.putAll(crowd.workspaces)
    workspaces[GPU] =
      listOf(
        WorkspaceView(31u, "training", "~/ml/training", panes.subList(0, 12)),
        WorkspaceView(32u, "evals", "~/ml/evals", panes.subList(12, panes.size)),
      )
    World(
      listOf(crowd.devices[0], DeviceView(GPU, "gpu", "gpu-01", DeviceStatus.Connected)) + devices,
      workspaces,
    )
  }

  /** The world a scene opens on. */
  fun world(scene: DemoScene): World {
    if (scene.unpaired) return World(emptyList(), emptyMap())
    var world =
      when (scene) {
        DemoScene.CROWD,
        DemoScene.CROWD_DEVICE -> crowd
        DemoScene.MANY -> many
        else -> World(devices, workspaces)
      }
    if (scene == DemoScene.EMPTY) {
      world =
        world.copy(
          workspaces =
            world.workspaces.mapValues { (_, list) ->
              list.map { workspace ->
                workspace.copy(
                  panes =
                    workspace.panes.map { pane ->
                      val agent = pane.agent
                      if (agent?.status == AgentStatus.BLOCKED) {
                        pane.copy(agent = agent.copy(status = AgentStatus.WORKING, detail = null))
                      } else pane
                    }
                )
              }
            }
        )
    }
    if (scene == DemoScene.BANNER) {
      val list = world.workspaces[JPDEV]?.toMutableList()
      if (list != null) {
        val panes = list[0].panes.toMutableList()
        panes[1] =
          panes[1].copy(
            agent =
              agent("claude", AgentStatus.BLOCKED, "Bash: pnpm prisma migrate dev --name passkeys")
          )
        list[0] = list[0].copy(panes = panes)
        world = world.copy(workspaces = world.workspaces + (JPDEV to list))
      }
    }
    return world
  }
}

/**
 * The screens the demo Panes show: a few lines each, so the terminal page has something to draw and
 * the key row something to answer.
 */
object DemoScreens {
  const val COLUMNS: Int = 44
  const val ROWS: Int = 18
  const val FOREGROUND = 0xc9d1d9u
  const val BACKGROUND = 0x0d1117u
  const val GREEN = 0x3fb950u
  const val YELLOW = 0xd29922u
  const val BLUE = 0x58a6ffu
  const val GRAY = 0x6e7681u
  const val WHITE = 0xffffffu

  data class Span(val text: String, val color: UInt = FOREGROUND, val bold: Boolean = false)

  private fun s(text: String, color: UInt = FOREGROUND, bold: Boolean = false) =
    Span(text, color, bold)

  private fun lines(pane: ULong): List<List<Span>> =
    when (pane.toLong()) {
      1L ->
        listOf(
          listOf(s("● ", GREEN), s("Bash", WHITE, true), s("(cargo test -p condr-client)", GRAY)),
          listOf(s("  ⎿  ", GRAY), s("Running 46 tests")),
          emptyList(),
          listOf(s("Allow Bash to run this command?", WHITE, true)),
          emptyList(),
          listOf(s("❯ 1. Yes", BLUE, true)),
          listOf(s("  2. Yes, and don't ask again this session")),
          listOf(s("  3. No, and tell Claude what to do differently")),
          emptyList(),
          listOf(s("Esc to cancel · Enter to choose", GRAY)),
        )
      11L ->
        listOf(
          listOf(s("● ", GREEN), s("Which chart library should the dashboard use?", WHITE, true)),
          emptyList(),
          listOf(s("❯ 1. Recharts", BLUE, true), s("  already in package.json", GRAY)),
          listOf(s("  2. Chart.js")),
          listOf(s("  3. Something else")),
          emptyList(),
          listOf(s("Enter to choose", GRAY)),
        )
      2L ->
        listOf(
          listOf(s("codex", WHITE, true), s(" · gpt-5-codex", GRAY)),
          listOf(s("⠋ ", YELLOW), s("Reading crates/condr-gui/src/app/sidebar.rs")),
          listOf(s("  ", GRAY), s("sidebar.rs:214 repaints every Pane on hover", GRAY)),
          emptyList(),
          listOf(s("Working…", YELLOW)),
        )
      12L ->
        listOf(
          listOf(s("● ", YELLOW), s("Edit", WHITE, true), s("(src/auth/passkeys.ts)", GRAY)),
          listOf(s("  ⎿  ", GRAY), s("Added 42 lines")),
          listOf(s("● ", YELLOW), s("Bash", WHITE, true), s("(pnpm test auth)", GRAY)),
          listOf(s("  ⎿  ", GRAY), s("12 passed", GREEN)),
          emptyList(),
          listOf(s("✻ Thinking…", YELLOW)),
        )
      4L ->
        listOf(
          listOf(s("opencode", WHITE, true)),
          listOf(s("● ", GREEN), s("Rewrote double_pinyin.schema.yaml")),
          listOf(s("● ", GREEN), s("rime --deploy: ok", GREEN)),
          emptyList(),
          listOf(s("Done.", GREEN, true)),
        )
      13L ->
        listOf(
          listOf(s("codex", WHITE, true), s(" · gpt-5-codex", GRAY)),
          listOf(s("› ", BLUE), s("Tighten the pairing screen copy")),
          listOf(s("  ", GRAY), s("Rewrote 6 strings in Onboarding.swift", GRAY)),
          emptyList(),
          listOf(s("› ", BLUE), s("")),
        )
      else ->
        listOf(
          listOf(s("rocky@macmini ", GREEN), s("~/Developer/project/condr", BLUE), s(" %")),
          listOf(s("$ cargo build")),
          listOf(s("   Compiling condr-core v0.1.10", GREEN)),
          listOf(s("    Finished dev profile in 12.4s", GREEN)),
          listOf(s("rocky@macmini ", GREEN), s("~/Developer/project/condr", BLUE), s(" % ")),
        )
    }

  fun screen(pane: ULong): TerminalScreen {
    val lines = lines(pane)
    val cells = ArrayList<ScreenCell>(ROWS * COLUMNS)
    var cursorColumn = 0
    for (row in 0 until ROWS) {
      var column = 0
      for (span in if (row < lines.size) lines[row] else emptyList()) {
        for (character in span.text) {
          if (column >= COLUMNS) break
          cells.add(
            ScreenCell(character.toString(), span.color, BACKGROUND, span.bold, false, false, null)
          )
          column += 1
        }
      }
      if (row == lines.size - 1) cursorColumn = column
      while (column < COLUMNS) {
        cells.add(ScreenCell(" ", FOREGROUND, BACKGROUND, false, false, false, null))
        column += 1
      }
    }
    return TerminalScreen(
      revision = 1u,
      rows = ROWS.toUShort(),
      columns = COLUMNS.toUShort(),
      scrolledLines = 0u,
      cells = cells,
      cursor = ScreenCursor((lines.size - 1).toUShort(), cursorColumn.toUShort(), 0xf0f6fcu),
      background = BACKGROUND,
      mouseTracking = false,
      hasSelection = false,
    )
  }
}

/** The library's shape over the demo world: nothing connects, nothing moves on its own. */
class DemoClient(scene: DemoScene) : CompanionInterface {
  private val lock = Any()
  private var devices: MutableList<DeviceView>
  private var workspaces: MutableMap<String, List<WorkspaceView>>
  /** The demo Pane's selection, as cell indices, both included. */
  private var selection: Pair<ULong, IntRange>? = null
  private val changes = ArrayList<Change>()
  private val events = ArrayList<Event>()

  init {
    val world = DemoWorld.world(scene)
    devices = world.devices.toMutableList()
    workspaces = world.workspaces.toMutableMap()
  }

  override fun shutdown() {}

  override fun resume() {}

  override fun suspend() {}

  override fun setActive(active: Boolean) {}

  override fun networkChanged() {}

  override fun takeChanges(): List<Change> =
    synchronized(lock) { changes.toList().also { changes.clear() } }

  override fun takeEvents(): List<Event> =
    synchronized(lock) { events.toList().also { events.clear() } }

  override fun devices(): List<DeviceView> = synchronized(lock) { devices.toList() }

  override fun needsYou(): List<NeedsYouItem> =
    synchronized(lock) {
      devices
        .filter { it.status == DeviceStatus.Connected }
        .flatMap { device ->
          (workspaces[device.address] ?: emptyList()).flatMap { workspace ->
            workspace.panes.mapNotNull { pane ->
              val agent = pane.agent?.takeIf { it.status.needsYou } ?: return@mapNotNull null
              NeedsYouItem(
                device.address,
                device.label,
                workspace.name,
                pane.id,
                agent.kind,
                agent.label,
                agent.detail,
              )
            }
          }
        }
    }

  override fun workspaces(address: String): List<WorkspaceView> =
    synchronized(lock) { workspaces[address] ?: emptyList() }

  override fun setName(name: String) {}

  override fun setPush(push: Boolean) {}

  override fun cloudToken(): String = ""

  override fun pair(link: String, label: String?): SavedDevice {
    if (!(link.startsWith("p2p://") || link.startsWith("tcp://")) || '.' !in link) {
      throw MobileException.Invalid("That isn't a pairing link. It starts with p2p:// or tcp://.")
    }
    return synchronized(lock) {
      val known = devices.map { it.address }.toSet()
      val next =
        DemoWorld.devices.firstOrNull {
          it.address !in known && it.status == DeviceStatus.Connected
        } ?: DemoWorld.devices[0]
      if (next.address !in known) {
        devices.add(next)
        workspaces[next.address] = DemoWorld.workspaces[next.address] ?: emptyList()
      }
      SavedDevice(next.address, next.label)
    }
  }

  override fun forget(address: String) {
    synchronized(lock) {
      devices.removeAll { it.address == address }
      workspaces.remove(address)
    }
  }

  /** Whatever screen the Pane has, its selection in the selection's colours. */
  override fun terminal(address: String, pane: ULong): TerminalScreen? {
    val screen = DemoScreens.screen(pane)
    val selection = synchronized(lock) { selection } ?: return screen
    if (selection.first != pane) return screen
    val cells = screen.cells.toMutableList()
    for (index in selection.second) {
      if (index in cells.indices) {
        cells[index] =
          cells[index].copy(foreground = SELECTION_TEXT, background = SELECTION_BACKGROUND)
      }
    }
    return screen.copy(cells = cells, hasSelection = true)
  }

  override fun showPane(address: String?, pane: ULong, rows: UShort, columns: UShort) {}

  override fun rename(address: String, label: String) {}

  override fun sendText(address: String, pane: ULong, text: String) {}

  override fun sendKey(address: String, pane: ULong, key: Key, control: Boolean) {}

  override fun paste(address: String, pane: ULong, text: String) {}

  override fun submit(address: String, pane: ULong, text: String) {}

  override fun wheel(address: String, pane: ULong, row: UShort, column: UShort, lines: Int) {}

  override fun click(address: String, pane: ULong, row: UShort, column: UShort) {}

  override fun scrollToBottom(address: String, pane: ULong) {}

  /** The run of non-blank cells around the one pressed, as the Server picks a word. */
  override fun selectWord(address: String, pane: ULong, row: UShort, column: UShort) {
    val screen = DemoScreens.screen(pane)
    val width = screen.columns.toInt()
    val start = row.toInt() * width
    val pressed = start + column.toInt()
    if (pressed !in screen.cells.indices || screen.cells[pressed].text == " ") return
    var first = pressed
    var last = pressed
    while (first > start && screen.cells[first - 1].text != " ") first -= 1
    while (last + 1 < start + width && screen.cells[last + 1].text != " ") last += 1
    select(pane, address, first..last)
  }

  override fun selectRange(
    address: String,
    pane: ULong,
    fromRow: UShort,
    fromColumn: UShort,
    toRow: UShort,
    toColumn: UShort,
  ) {
    val width = DemoScreens.COLUMNS
    val from = fromRow.toInt() * width + fromColumn.toInt()
    val to = toRow.toInt() * width + toColumn.toInt()
    select(pane, address, minOf(from, to)..maxOf(from, to))
  }

  override fun clearSelection(address: String, pane: ULong) {
    synchronized(lock) {
      selection = null
      changes.add(Change.Terminal(address, pane))
    }
  }

  override fun copySelection(address: String, pane: ULong) {
    val screen = DemoScreens.screen(pane)
    synchronized(lock) {
      val selection = selection?.takeIf { it.first == pane } ?: return
      val text = selection.second.joinToString("") { screen.cells[it].text }
      events.add(Event.Copied(text.trim()))
    }
  }

  private fun select(pane: ULong, address: String, cells: IntRange) {
    synchronized(lock) {
      selection = pane to cells
      changes.add(Change.Terminal(address, pane))
    }
  }

  companion object {
    /** The default scheme's selection, as the library colours it. */
    private const val SELECTION_BACKGROUND = 0xe6edf3u
    private const val SELECTION_TEXT = 0x0d1117u
  }
}
