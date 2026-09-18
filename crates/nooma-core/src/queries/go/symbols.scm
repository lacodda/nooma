; Go declarations.

(function_declaration name: (identifier) @name) @kind.function

; A method's receiver type is not its parent in the tree — Go declares methods
; beside the type, not inside it. The declaration is recorded as a function;
; the receiver shows in the source at the line given.
(method_declaration name: (field_identifier) @name) @kind.function

(type_declaration
  (type_spec name: (type_identifier) @name)) @kind.type
(type_declaration
  (type_alias name: (type_identifier) @name)) @kind.type

(package_clause (package_identifier) @name) @kind.module

(const_declaration
  (const_spec name: (identifier) @name)) @kind.constant
(var_declaration
  (var_spec name: (identifier) @name)) @kind.constant
