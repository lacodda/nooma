; TypeScript and TSX declarations.
;
; The TSX grammar is used for every flavour: it is a superset, so `.ts` parses
; under it unchanged while `.tsx` keeps its elements.

(function_declaration name: (identifier) @name) @kind.function
(generator_function_declaration name: (identifier) @name) @kind.function
(method_definition name: (property_identifier) @name) @kind.function

; `const Button = () => {}` is how most of a modern codebase declares its
; functions; without this it would all be recorded as constants.
;
; The plain-binding pattern below matches this node as well: a query has no way
; to say "unless the value is a function". Both fire, and the more specific
; kind wins when the two are folded together — see `symbols::collect_symbols`.
(lexical_declaration
  (variable_declarator
    name: (identifier) @name
    value: [(arrow_function) (function_expression) (generator_function)])) @kind.function

(class_declaration name: (type_identifier) @name) @kind.type
(interface_declaration name: (type_identifier) @name) @kind.type
(type_alias_declaration name: (type_identifier) @name) @kind.type
(enum_declaration name: (identifier) @name) @kind.type

(internal_module name: (identifier) @name) @kind.module

(lexical_declaration
  (variable_declarator name: (identifier) @name)) @kind.constant
