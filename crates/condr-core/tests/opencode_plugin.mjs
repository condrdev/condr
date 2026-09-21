// Run with: node crates/condr-core/tests/opencode_plugin.mjs
import assert from "node:assert/strict"
import { mkdtempSync, readFileSync, writeFileSync, rmSync } from "node:fs"
import { tmpdir } from "node:os"
import { join } from "node:path"
import { setTimeout as delay } from "node:timers/promises"

const source = readFileSync(new URL("../src/agent/opencode.js", import.meta.url), "utf8")
const root = mkdtempSync(join(tmpdir(), "condr-opencode-test-"))
const originalCwd = process.cwd()
let dispose = () => {}
try {
  process.chdir(root)
  process.env.CONDR_ENV = "1"
  writeFileSync("agent-hook", `
    const fs = require("node:fs");
    let input = "";
    process.stdin.on("data", chunk => input += chunk);
    process.stdin.on("end", () => fs.appendFileSync("reports", JSON.stringify({
      event: process.argv[3], ...JSON.parse(input),
    }) + "\\n"));
  `)
  writeFileSync("reports", "")
  const plugin = (await import("data:text/javascript;base64," + Buffer.from(
    source.replace("__CONDR_EXECUTABLE__", JSON.stringify(process.execPath)),
  ).toString("base64"))).default
  const sessions = new Map([
    ["ses_A", { id: "ses_A" }], ["ses_B", { id: "ses_B" }],
    ["ses_child", { id: "ses_child", parentID: "ses_B" }],
  ])
  const status = new Map([["ses_A", "busy"], ["ses_B", "idle"]])
  const route = { current: { name: "session", params: { sessionID: "ses_A" } } }
  let permission = [], question = []
  const reports = () => readFileSync("reports", "utf8").trim().split("\n").filter(Boolean).map(JSON.parse)
  const waitFor = async (predicate) => {
    const deadline = Date.now() + 5000
    while (!predicate()) {
      assert.ok(Date.now() < deadline, JSON.stringify(reports()))
      await delay(20)
    }
  }
  await plugin.tui({
    route,
    state: { session: {
      get: id => sessions.get(id), status: id => ({ type: status.get(id) }),
      permission: () => permission, question: () => question,
    } },
    lifecycle: { onDispose: fn => { dispose = fn } },
  })
  await waitFor(() => reports().at(-1)?.event === "prompt-submit")
  route.current = { name: "session", params: { sessionID: "ses_B" } }
  await waitFor(() => reports().at(-1)?.session_id === "ses_B" && reports().at(-1)?.event === "stop")
  const switchedAt = reports().findIndex(report => report.session_id === "ses_B")
  // An old root finishing cannot replace the selected conversation.
  status.set("ses_A", "idle")
  permission = [{ id: "permission", title: "Run cargo test" }]
  await waitFor(() => reports().at(-1)?.event === "permission-request")
  assert.equal(reports().at(-1).detail, "Run cargo test", "a permission says what it is for")
  permission = []
  question = [{ id: "question", questions: [{ question: "Which DB?" }] }]
  await waitFor(() => reports().at(-1)?.event === "question-asked")
  assert.equal(reports().at(-1).detail, "Which DB?", "a question carries its text")
  question = []
  route.current = { name: "session", params: { sessionID: "ses_child" } }
  await delay(250)
  assert.ok(reports().slice(switchedAt).every(report => report.session_id === "ses_B"))
  dispose()
  const before = reports().length
  route.current = { name: "session", params: { sessionID: "ses_A" } }
  await delay(150)
  assert.equal(reports().length, before)
  console.log("OpenCode: existing-session switch, stale root, child isolation, status and disposal passed")
} finally {
  dispose()
  process.chdir(originalCwd)
  rmSync(root, { recursive: true, force: true })
}
