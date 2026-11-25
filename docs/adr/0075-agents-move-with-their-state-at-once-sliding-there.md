# Agents Move With Their State at Once, Sliding There

Status: accepted 2026-11-25. Supersedes ADR 0067's sticky selection, which ADR 0074 kept, and Tasks mode's selected card keeping its column (ADR 0068).

ADR 0067 kept the selected Agent's row at the rank it had when it was selected until another Agent was selected, so a Done seen did not drop away under the pointer and stepping through finished Agents with the keyboard skipped none. In use it reads the other way round: the Agent the person is answering stays put while its state changes, and the list jumps later, when they click something else, at a moment that has nothing to do with that Agent. Tasks mode did the same with the selected card's column.

## Decision

**Every row is where its Agent's state puts it now, the selected one included**, in the Agents section and in Tasks mode alike. A Done the person selects is seen and moves among the Idle at once, as ADR 0052 has seeing it.

**`next-agent`, `previous-agent` and Tasks mode's arrows step through the order shown.** Stepping onto a Done sees it and moves it, so the next step goes on from where it now is, and the other finished Agents are a click each. Accepted: keys that follow an order no longer on screen confuse more.

**What moves slides there.** A row or card laid out somewhere new is drawn where it was and slides to its place on the section's fold spring, critically damped and settled in about 250 ms; a change mid-slide turns it from where it is, keeping its speed. One that comes or goes does so at once, and the rest slide around it; a group's heading slides with its group. On the Board a card that changes column leaves one and appears in the other while its neighbours slide, since each column scrolls and clips on its own. A card moving into a folded column or group only changes its count. Under reduced motion everything is in place at once.

`SlideColumn` (`condr-gui/src/app/slide.rs`) does it for the section's rows, each Board column and the Table: a column of keyed rows that, once laid out, compares each row's top, measured from the column's own top, with the previous frame's and offsets that row's prepaint by the difference, which a spring takes to zero. Measured from the column, scrolling or resizing what holds it moves nothing.

Nothing holds the order while the pointer is over the list, and the list does not scroll to follow the selected row. "Needs you" is untouched: its rows come and go but never reorder.

## Considered

- **Keeping the anchor for the keys only**, the rows shown live. Declined: the keys would step through an order the person cannot see.
- **A Done seen when the person leaves it** rather than when they select it, so stepping through finished Agents skips none. Declined: it changes ADR 0052's seen for every Client, and moves the row when the person clicks elsewhere, the jump this decision removes.
- **Holding the order while the pointer is over the list.** Declined: the list would jump when the pointer leaves it.
- **A card flying between Board columns** over a floating copy. Declined: two columns that clip and scroll on their own make it much more code for one motion.
- **Debouncing an Agent that flaps between states.** Not needed: the spring turns mid-slide without a jump.

## Tests

`SlideColumn` drawing a moved row where it was, sliding it and settling it in its place, and placing it at once under reduced motion; the section moving the selected Agent at once and sliding it; a selected card going to its new column at once; the section's order without the anchor.
