---
name: code-review
description: Review a change for correctness, security and maintainability before it is committed. Use when asked to review code, a diff or a pull request.
---

# Code review checklist

Review the change in this order and report findings grouped by severity
(blocker, should fix, nit). Quote file:line for each finding.

1. **Correctness** — does it do what the request asked? Edge cases: empty input,
   errors, concurrency, off-by-one, time zones, unicode.
2. **Security** — untrusted input reaching shell commands, SQL, paths or HTML;
   secrets in code or logs; permissions widened.
3. **Tests** — is new behavior covered? Would a test fail if the change were reverted?
4. **Design** — does it follow the project's existing patterns and file layout?
   Is there duplicated logic that belongs in a shared helper?
5. **Readability** — names, comments explaining *why*, dead code.

Do not rewrite the code while reviewing; propose concrete fixes instead.
