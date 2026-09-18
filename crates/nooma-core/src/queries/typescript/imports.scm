; Module specifiers, quotes and all — the quotes are trimmed by the caller.

(import_statement source: (string) @module)
(export_statement source: (string) @module)

; Dynamic `import("./thing")`.
(call_expression
  function: (import)
  arguments: (arguments (string) @module))

; CommonJS `require("./thing")`, which a TypeScript codebase still meets in
; config files and tooling. The name is matched explicitly, and the predicate
; sits INSIDE the pattern's parentheses: outside them it parses as its own
; pattern, applies to nothing, and the query then records the argument of
; every one-string call in the file — `t("./looks-like-a-path")` included.
((call_expression
   function: (identifier) @_fn
   arguments: (arguments (string) @module))
 (#eq? @_fn "require"))
