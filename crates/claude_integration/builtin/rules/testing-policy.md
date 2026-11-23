---
name: "Testing policy"
enabled: true
description: "When and how to write and run tests"
applies_to: "all"
priority: 40
tags: ["testing"]
---

- New behavior and bug fixes come with tests; a bug fix starts with a failing test.
- Run the relevant tests before saying a change is done, and show the result.
- Never weaken or delete a test to make it pass; explain the failure instead.
