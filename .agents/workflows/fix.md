---
description: Automatically resolve borrow checker, lifetime, and type mismatch errors by leveraging `rustc` compiler diagnostics.
---

## Execution Steps
1.  **Format:** Run `cargo fmt` to baseline the code structure.
2.  **Check:** Run `cargo check --message-format=json`. 
3.  **Analyze & Iterate (Max 3 Loops):**
    *   If `cargo check` fails, read the compiler output. Pay special attention to Rust compiler help messages (e.g., "consider borrowing here: `&foo`").
    *   Formulate a plan to fix the error (preferring safe Rust and referencing `AGENTS.md`).
    *   Apply the fix to the code.
    *   Re-run `cargo check`.
4.  **Linting:** Once `cargo check` passes cleanly, run `cargo clippy -- -W clippy::pedantic`. Apply any suggested idiomatic fixes.
5.  **Completion:** Present a summary of the changes made to achieve a successful compilation. If the error cannot be resolved after 3 loops, stop and explain the architectural blocker to the user.