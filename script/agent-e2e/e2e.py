#!/usr/bin/env python3
"""Runs one agent CLI through a real Condr session, inside the container run.sh starts.

The container's home starts empty, so the Server, the agent's configuration and its hooks
are this run's alone. The model is the one the host's Codex configuration names, mounted
at /run/codex.toml.

Each step is PASS, FAIL, or NOTE (worth reading, not a failure):
  hooks    `agent hooks install`; only `status` for an agent recognized only
  start    `agent start`; a first-run dialog is answered as a user would
  turn     a prompt watched through `agent list`: it must pass through working and end
           idle or done with the model's answer on screen
  session  the native session ID the agent reported
  wait     `agent prompt --wait`, given that first prompt, returns only once it settled
  resume   `server restart` reopens the conversation: same ID, and the model remembers
  blocked  a shell command the CLI asks to approve shows blocked, and Esc or Ctrl+C
           settles it

Writes /out/<agent>/result.json with the screen and the Server's log beside it.
`e2e.py --report` prints every result under /out as one Markdown table.
"""

import json
import os
import shutil
import subprocess
import sys
import time
import tomllib
from pathlib import Path

HOME = Path.home()
WORK = HOME / "work"
OUT = Path("/out")
KEY_ENV = "CONDR_E2E_API_KEY"
NO_TOOLS = " Answer with the number only. Do not use any tools."
# Each answer shows on screen only once the model replied, never in the prompt.
TURN = ("What is 1234 + 4321?" + NO_TOOLS, "5555")
RESUMED = ("Add 1 to the last number you answered." + NO_TOOLS, "5556")
# A write through a redirect: Grok's ask mode still lets a bare `touch` through.
BLOCK = "Use your shell tool to run `echo probe > blocked-probe`, then say done."
SETTLED = ("idle", "done")
TURN_SECONDS = 240
STEPS = ("hooks", "start", "turn", "session", "wait", "resume", "blocked")
# The dialog a CLI shows in a folder it has not seen, and the keys a user answers it with.
FIRST_RUN = {"grok": ("Do you trust the contents", ["y"]),
             "kimi": ("Trust this folder?", ["enter"])}


def write(path, text):
    path = HOME / path
    path.parent.mkdir(parents=True, exist_ok=True)
    path.write_text(text)


def codex_provider():
    """The base URL, key and model the host's Codex uses."""
    config = tomllib.loads(Path("/run/codex.toml").read_text())
    provider = config.get("model_providers", {}).get(config.get("model_provider", ""))
    if not provider or "base_url" not in provider:
        sys.exit("Codex's model_provider must be a [model_providers.<id>] entry with base_url")
    key = provider.get("experimental_bearer_token") or os.environ.get(provider.get("env_key", ""))
    if not key:
        sys.exit("Codex's provider has no experimental_bearer_token, and its env_key is not set")
    return provider["base_url"].rstrip("/"), key, config["model"]


# What each CLI needs to start on the Codex model; each returns its Pane's environment.
# Grok and Copilot speak Chat Completions: a gateway may refuse what they send to Responses.

def pi(base, model):
    write(".pi/agent/models.json", json.dumps({"providers": {"condr-e2e": {
        "baseUrl": base, "api": "openai-responses", "apiKey": f"${KEY_ENV}",
        "authHeader": True, "models": [{"id": model}]}}}))
    write(".pi/agent/settings.json", json.dumps(
        {"defaultProvider": "condr-e2e", "defaultModel": model}))
    return {}


def omp(base, model):
    write(".omp/agent/models.yml", f"""providers:
  condr-e2e:
    baseUrl: {base}
    api: openai-responses
    apiKey: {KEY_ENV}
    authHeader: true
    models:
      - id: {model}
""")
    write(".omp/agent/config.yml", f"""startup:
  setupWizard: false
# Ask before every tool, so the blocked step has a prompt to see.
tools:
  approvalMode: always-ask
defaultProvider: condr-e2e
defaultModel: {model}
""")
    return {}


def grok(base, model):
    write(".grok/config.toml", f"""[models]
default = "condr-e2e"

# Ask before every tool, so the blocked step has a prompt to see.
[ui]
permission_mode = "ask"

[model.condr-e2e]
model = "{model}"
base_url = "{base}"
env_key = "{KEY_ENV}"
context_window = 128000
""")
    return {}


def copilot(base, model):
    write(".copilot/config.json", json.dumps(
        {"trustedFolders": [str(WORK)], "banner": "never"}))
    return {
        "COPILOT_OFFLINE": "true",
        "COPILOT_PROVIDER_BASE_URL": base,
        "COPILOT_PROVIDER_API_KEY": os.environ[KEY_ENV],
        "COPILOT_PROVIDER_WIRE_API": "completions",
        "COPILOT_MODEL": model,
    }


def opencode(base, model):
    write(".config/opencode/opencode.json", json.dumps({
        "$schema": "https://opencode.ai/config.json",
        "model": f"condr-e2e/{model}",
        "provider": {"condr-e2e": {
            "npm": "@ai-sdk/openai",
            "options": {"baseURL": base, "apiKey": f"{{env:{KEY_ENV}}}"},
            "models": {model: {}}}},
        # Ask before shell commands, so the blocked step has a prompt to see.
        "permission": {"bash": "ask"}}))
    return {}


def kimi(base, model):
    write(".kimi-code/config.toml", f"""default_model = "condr-e2e"

[providers.condr-e2e]
type = "openai_responses"
base_url = "{base}"
api_key = "{os.environ[KEY_ENV]}"

[models.condr-e2e]
provider = "condr-e2e"
model = "{model}"
max_context_size = 128000
""")
    return {}


PROFILES = {f.__name__: f for f in (pi, omp, grok, copilot, opencode, kimi)}


class Failed(Exception):
    pass


def condr(*args, timeout=600):
    run = subprocess.run(["condr", *map(str, args)], capture_output=True, text=True,
                         timeout=timeout)
    if run.returncode:
        raise Failed(f"condr {args[0]} {args[1]}: {(run.stderr or run.stdout).strip()}")
    out = run.stdout.strip()
    return json.loads(out) if out.startswith("{") else out


def agent_in(pane=None, kind=None):
    for agent in condr("agent", "list")["agents"]:
        if agent["pane_id"] == pane or (pane is None and agent["agent"] == kind):
            return agent
    return None


def screen(pane):
    return condr("pane", "read", pane, "--lines", 300)


def reported(pane, seconds):
    """The state the agent reports within `seconds`; None if it stays silent."""
    try:
        return condr("agent", "wait", pane, "--until", "idle",
                     "--timeout", seconds * 1000)["agent"]["agent_status"]
    except Failed:
        return None


def shows(pane, text, seconds):
    deadline = time.monotonic() + seconds
    while text not in screen(pane):
        if time.monotonic() > deadline:
            return False
        time.sleep(1)
    return True


def watch(pane, answer=None, until=SETTLED, seconds=TURN_SECONDS, poll=None):
    """The states the agent went through until, having left where it started, it reached
    one of `until` with `answer` on screen, failed, or the time ran out. `poll` sees the
    states so far on every look."""
    trace = []
    deadline = time.monotonic() + seconds
    while time.monotonic() < deadline:
        if poll:
            poll(trace)
        agent = agent_in(pane)
        state = agent["agent_status"] if agent else "exited"
        if not trace or trace[-1] != state:
            trace.append(state)
        moved = "working" in trace or "blocked" in trace
        if state in ("exited", "error") or (
                moved and state in until and (answer is None or answer in screen(pane))):
            return trace
        time.sleep(0.3)
    return trace + ["timeout"]


def answered(trace, pane, answer):
    return "working" in trace and trace[-1] in SETTLED and answer in screen(pane)


def main(kind):
    out = OUT / kind
    out.mkdir(parents=True, exist_ok=True)
    base, key, model = codex_provider()
    os.environ[KEY_ENV] = key
    os.environ.update(PROFILES[kind](base, model))
    WORK.mkdir()
    subprocess.run(["git", "init", "-q", WORK], check=True)
    version = subprocess.run([kind, "--version"], capture_output=True, text=True,
                             timeout=60).stdout.strip().splitlines()
    condr_version = subprocess.run(["condr", "--version"], capture_output=True,
                                   text=True).stdout.strip()
    result = {"agent": kind, "version": version[0] if version else "?", "model": model,
              "condr": condr_version, "steps": []}
    pane = None

    def step(name, ok, note, fatal=True):
        result["steps"].append({"step": name, "ok": ok, "note": note})
        print(f"  {kind:9} {name:8} {'PASS' if ok else 'NOTE' if ok is None else 'FAIL'}"
              f"  {note}", flush=True)
        if ok is False and fatal:
            raise Failed(None)

    try:
        condr("server", "start")
        hooks = condr("agent", "hooks", "status", kind)
        reports = hooks["state"] != "unsupported"
        if reports:
            hooks = condr("agent", "hooks", "install", kind)
        step("hooks", hooks["state"] in ("installed", "unsupported"),
             hooks["state"] + (f": {hooks['note']}" if hooks.get("note") else ""))

        pane = condr("workspace", "create", "--cwd", WORK)["root_pane"]["pane_id"]
        typed = False
        try:
            started = condr("agent", "start", "e2e", "--kind", kind, "--pane", pane,
                            "--timeout", 60_000)["agent"]["agent_status"]
        except Failed as error:
            agent = agent_in(pane)
            if "agent_not_ready" in str(error):
                started = "blocked"
            elif "agent_timeout" in str(error) and agent and agent["agent"] == kind:
                started, typed = agent["agent_status"], True
            else:
                raise
        note, ok = started, True
        if kind in FIRST_RUN and shows(pane, FIRST_RUN[kind][0], 10):
            condr("pane", "send-keys", pane, *FIRST_RUN[kind][1])
            note += f", answered its first-run dialog with {'+'.join(FIRST_RUN[kind][1])}"
            if reports:
                started = reported(pane, 60) or "silent"
                note += f" → {started}"
                ok = started == "idle"
        if typed:
            note += (" and nothing within 60 s although its spec says it reports at startup, "
                     "so `agent prompt` refuses it; its first prompt is typed instead")
            ok = False
        elif started == "unknown" and reports:
            later = reported(pane, 15)
            if later:
                note += (f", then {later} on its own: a prompt typed when `agent start` "
                         "returns is lost, so its spec should say it reports at startup")
                ok = None
            else:
                note += ", silent until its first prompt"
        step("start", ok, note, fatal=False)

        if not reports:
            condr("agent", "prompt", pane, TURN[0])
            deadline = time.monotonic() + TURN_SECONDS
            while TURN[1] not in screen(pane) and time.monotonic() < deadline:
                time.sleep(2)
            step("turn", TURN[1] in screen(pane), "recognized only: answer read off the screen")
            state = agent_in(pane)["agent_status"]
            step("session", True if state == "unknown" else None,
                 "recognized only, stays unknown" if state == "unknown" else
                 f"recognized only, yet reports {state} through the Program Status Protocol")
            return result

        # The first prompt goes through `--wait` while the states are watched, so the
        # wait is seen against the turn it waited for.
        if typed:
            condr("pane", "send-text", pane, TURN[0])
            condr("pane", "send-keys", pane, "enter")
        else:
            waiting = subprocess.Popen(
                ["condr", "agent", "prompt", str(pane), TURN[0], "--wait",
                 "--timeout", str(TURN_SECONDS * 1000)],
                stdout=subprocess.PIPE, stderr=subprocess.PIPE, text=True)
        returned = []

        def poll(trace):
            if not typed and not returned and waiting.poll() is not None:
                returned.append((list(trace), TURN[1] in screen(pane)))

        trace = watch(pane, TURN[1], poll=poll)
        step("turn", answered(trace, pane, TURN[1]), " → ".join(trace))
        session = agent_in(pane)["session_id"]
        step("session", bool(session), session or "no session_id reported", fatal=False)
        if typed:
            step("wait", None, "not run: `agent prompt` refuses an agent that never reported")
        else:
            stdout, stderr = waiting.communicate()
            state = (json.loads(stdout)["agent"]["agent_status"] if stdout.strip()
                     else stderr.strip())
            early = returned and not returned[0][1]
            step("wait", state in SETTLED and not early,
                 f"returned {state}" + (f" before the answer, the turn at "
                                        f"{' → '.join(returned[0][0])}"
                                        if early else " once the turn settled"), fatal=False)

        condr("server", "restart")
        deadline = time.monotonic() + 60
        resumed = None
        while not resumed and time.monotonic() < deadline:
            time.sleep(1)
            resumed = agent_in(kind=kind)
        if not resumed:
            step("resume", False, "no agent came back after the restart")
        pane = resumed["pane_id"]
        if not reported(pane, 30):
            # shortcut: an agent silent until prompted gets a fixed head start to draw its
            # prompt; replace with a readiness signal once Condr has one.
            time.sleep(5)
        condr("agent", "prompt", pane, RESUMED[0])
        trace = watch(pane, RESUMED[1])
        again = agent_in(pane)["session_id"]
        remembered = answered(trace, pane, RESUMED[1])
        step("resume", remembered and again == session,
             " → ".join(trace) + f", {'remembered' if remembered else 'forgot'} the turn, "
             + ("same session" if again == session else f"session became {again}"))

        condr("agent", "prompt", pane, BLOCK)
        trace = watch(pane, until=("blocked",) + SETTLED)
        if trace[-1] != "blocked":
            ran = (WORK / "blocked-probe").exists()
            step("blocked", None, " → ".join(trace) + " without asking, "
                 + ("ran the command" if ran else "did not run it"))
        else:
            detail = agent_in(pane)["detail"]
            # Grok's approval closes on Ctrl+C, not Esc.
            for key in ("esc", "ctrl+c"):
                condr("pane", "send-keys", pane, key)
                after = watch(pane, seconds=20)
                if after[-1] in SETTLED:
                    break
            step("blocked", after[-1] in SETTLED,
                 f"detail {detail!r}; {key}: {' → '.join(after)}")
    except Failed as error:
        if error.args[0]:
            step("error", False, str(error), fatal=False)
    finally:
        if pane is not None:
            try:
                (out / "screen.txt").write_text(screen(pane))
            except Failed:
                pass
        logs = HOME / ".local/state/condr"
        if logs.exists():
            shutil.copytree(logs, out / "logs", dirs_exist_ok=True)
        (out / "result.json").write_text(json.dumps(result, indent=2, ensure_ascii=False))
    return result


def report():
    results = [json.loads(path.read_text()) for path in sorted(OUT.glob("*/result.json"))]
    mark = {True: "PASS", False: "FAIL", None: "NOTE"}
    if results:
        print(f"{results[0]['condr']}, model {results[0]['model']}\n")
    print("| Agent | Version | " + " | ".join(STEPS) + " |")
    print("| --- | --- |" + " --- |" * len(STEPS))
    for result in results:
        steps = {step["step"]: mark[step["ok"]] for step in result["steps"]}
        if any(step["step"] == "error" for step in result["steps"]):
            steps.setdefault(next(s for s in STEPS if s not in steps), "FAIL")
        print(f"| {result['agent']} | {result['version']} | "
              + " | ".join(steps.get(name, "–") for name in STEPS) + " |")
    print()
    for result in results:
        for step in result["steps"]:
            if step["ok"] is not True:
                print(f"- {result['agent']} {step['step']}: {step['note']}")


if __name__ == "__main__":
    if sys.argv[1] == "--report":
        report()
    else:
        result = main(sys.argv[1])
        sys.exit(1 if any(step["ok"] is False for step in result["steps"]) else 0)
