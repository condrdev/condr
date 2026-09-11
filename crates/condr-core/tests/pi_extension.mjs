// Run with: node crates/condr-core/tests/pi_extension.mjs
import assert from "node:assert/strict"
import { mkdtempSync, readFileSync, writeFileSync, rmSync } from "node:fs"
import { tmpdir } from "node:os"
import { join } from "node:path"

const source = readFileSync(new URL("../src/agent/pi-extension.js", import.meta.url), "utf8")
const root = mkdtempSync(join(tmpdir(), "condr-pi-test-"))
const originalCwd = process.cwd()
try {
  process.chdir(root)
  process.env.CONDR_ENV = "1"
  writeFileSync("agent-hook", `
    const fs = require("node:fs"); let input = "";
    process.stdin.on("data", chunk => input += chunk);
    process.stdin.on("end", () => fs.appendFileSync("reports", JSON.stringify({
      agent: process.argv[2], event: process.argv[3], ...JSON.parse(input),
    }) + "\\n"));
  `)
  const reports = () => readFileSync("reports", "utf8").trim().split("\n").filter(Boolean).map(JSON.parse)
  for (const agent of ["pi", "omp"]) {
    writeFileSync("reports", "")
    const extension = (await import("data:text/javascript;base64," + Buffer.from(source
      .replace("__CONDR_EXECUTABLE__", JSON.stringify(process.execPath))
      .replace("__CONDR_AGENT__", JSON.stringify(agent))
      .replace("__CONDR_AGENT_MARKER__", agent)).toString("base64"))).default
    const handlers = new Map()
    extension({ on: (event, fn) => handlers.set(event, fn) })
    let id = "root-A", idle = true
    const ctx = { mode: "tui", isIdle: () => idle, sessionManager: { getSessionId: () => id } }
    const emit = async (type, fields = {}, context = ctx) => handlers.get(type)?.({ type, ...fields }, context)
    const last = () => reports().at(-1).event
    await emit("session_start", { reason: "startup" })
    assert.equal(last(), "session-start")
    idle = false
    await emit("agent_start")
    assert.equal(last(), "prompt-submit")
    await emit("agent_end", { willContinue: true })
    assert.equal(last(), "prompt-submit", "loop end/retry is not settled")
    if (agent === "pi") {
      await emit("ui_prompt_start", { kind: "confirm", reason: "ui_prompt" })
      assert.equal(last(), "question-asked")
      await emit("ui_prompt_end", { kind: "confirm", reason: "ui_prompt" })
      assert.equal(last(), "prompt-submit")
      await emit("session_start", { reason: "reload" })
      assert.equal(last(), "prompt-submit", "reload during work must not complete the turn")
      idle = true
      await emit("agent_settled")
      assert.equal(last(), "stop")
      await emit("ui_prompt_start", { kind: "select" })
      await emit("ui_prompt_end", { kind: "select" })
      assert.equal(last(), "stop", "an idle command's dialog must return to Idle")
    } else {
      for (const toolCallId of ["a", "b"]) await emit("tool_approval_requested", { sessionId: id, toolCallId })
      await emit("tool_approval_resolved", { sessionId: id, toolCallId: "a", approved: true })
      assert.equal(last(), "question-asked", "the other approval still waits")
      await emit("tool_approval_resolved", { sessionId: id, toolCallId: "b", approved: false })
      assert.equal(last(), "prompt-submit", "denial ends the wait, not the run")
      await emit("tool_execution_start", { toolName: "ask", toolCallId: "q" })
      assert.equal(last(), "question-asked")
      await emit("tool_result", { toolName: "ask", toolCallId: "q" })
      assert.equal(last(), "prompt-submit")
      await emit("tool_approval_requested", { sessionId: id, toolCallId: "cancelled" })
      idle = true
      await emit("agent_end", { willContinue: false })
      assert.equal(last(), "stop", "final cancellation clears unfinished waits")
    }
    const before = reports().length
    for (const mode of ["rpc", "print", "sdk"]) await emit("agent_start", {}, { ...ctx, mode })
    await emit("tool_approval_requested", { sessionId: "child", toolCallId: "nested" })
    assert.equal(reports().length, before, "other sessions and non-TUI modes stay silent")
    id = "root-B"
    await emit(agent === "pi" ? "session_start" : "session_switch", { reason: "resume" })
    assert.equal(reports().at(-1).session_id, id)
    if (agent === "omp") {
      id = "root-C"
      await emit("session_branch")
      assert.equal(reports().at(-1).session_id, id)
    }
    assert.ok(reports().every(report => report.agent === agent))
  }
  console.log("Pi/OMP: settled turns, UI/approval waits, continuation, cancellation, session switches and mode isolation passed")
} finally {
  process.chdir(originalCwd)
  rmSync(root, { recursive: true, force: true })
}
