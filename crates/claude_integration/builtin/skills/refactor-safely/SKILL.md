---
name: refactor-safely
description: Restructure code without changing behavior, in small verified steps. Use when asked to refactor, split a large file, rename or extract code.
---

# Refactor safely

- Behavior must not change. If tests are missing for the code you touch, add
  characterization tests first.
- Work in small steps (rename, extract, move); build and run tests after each.
- Keep public APIs stable unless asked; update every call site when you change one.
- Do not mix refactoring with feature work or formatting churn in the same change.
- Finish with the project's formatter, linter and test suite, and summarize what
  moved where.
