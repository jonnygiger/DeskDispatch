# Scribe's Journal

This journal records critical documentation insights, architectural learnings, and valuable lessons discovered while documenting DeskDispatch.

2026-03-29 Step Position Compaction Threshold Mechanics
Learning: Step position ordering uses DOUBLE PRECISION sparse positions, but floating-point precision exhaustion is guarded against by checking if the absolute gap between adjacent steps is less than `0.0001` (`check_and_compact_positions`). When triggered, `compact_positions` re-spaces all steps across the automation to clean multiples of `10.0` inside a single database transaction.
Action: Highlight both the trigger threshold (`0.0001`) and the exact re-spacing increment (`10.0`) when documenting position management or step ordering.
