# An Agent the Process Table Names Outlives Prompt Marks

Status: accepted 2026-11-19. Amends ADR 0044.

ADR 0044 follows the Program Status Protocol in dropping a `working` or `blocked` record at a new shell prompt, OSC 133 `A`: a prompt means the program that reported has gone, and what it last said is stale. Kimi Code 2.6 writes OSC 133 `A`, `B` and `C` around the turns of its own transcript while it works, so its Agent turned `Unknown` in the middle of every turn and came back with its next report.

While the process table names the Agent, a prompt mark cannot mean it has gone: it is still the foreground process, the mark is its own output or a child's, and its exit clears the Agent anyway (ADR 0014). Such an Agent keeps its record through OSC 133 `A`. The rule stays for an Agent named by its own reports because the table cannot see through its launcher, an `ssh` among them: for that Agent the prompt after it exits is the only sign that it went.
