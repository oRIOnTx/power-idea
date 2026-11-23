---
name: write-tests-first
description: Implement a feature or fix test-first (red, green, refactor). Use when adding behavior or fixing a bug in code that has a test suite.
---

# Write tests first

1. Find how the project runs tests (README, CONTRIBUTING, justfile, package.json,
   Makefile) and run the relevant subset once to see the baseline.
2. Write the smallest test that describes the new behavior or reproduces the bug.
   Run it and confirm it **fails for the expected reason**.
3. Implement the minimal change that makes it pass. Run the test again.
4. Refactor with the tests green; run the wider suite before finishing.
5. Report which tests you added and the final test output. If a test cannot be
   written (no harness, UI-only), say so explicitly instead of skipping silently.
