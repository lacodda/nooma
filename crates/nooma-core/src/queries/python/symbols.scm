; Python declarations.
;
; Methods and nested functions are the same node as a top-level `def`; what
; makes a method a method is the class span around it, which the parent pass
; works out from the @kind captures.

(function_definition name: (identifier) @name) @kind.function
(class_definition name: (identifier) @name) @kind.type

; Module-level bindings in SCREAMING_CASE are the language's constants by
; convention; the convention is all there is, since Python has no keyword.
; Other module-level assignments are left out rather than recorded as
; constants they are not.
;
; Two things this pattern has to get right, and both are silent when wrong:
;
; - The predicate sits INSIDE the pattern. Outside the closing paren it parses
;   as its own pattern, applies to nothing, and every assignment is recorded.
; - The @kind capture wraps the ASSIGNMENT, not the enclosing `module`. On the
;   module node the span covers the whole file, and every function and class
;   below it comes out as a child of the first constant.
(module
  (expression_statement
    ((assignment left: (identifier) @name) @kind.constant
     (#match? @name "^[A-Z][A-Z0-9_]*$"))))
