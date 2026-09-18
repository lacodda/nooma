; Rust declarations worth naming.
;
; The @kind capture wraps the whole declaration, not just its name: the span is
; what tells an impl's methods that they belong to the impl.

(function_item name: (identifier) @name) @kind.function

(struct_item name: (type_identifier) @name) @kind.type
(enum_item name: (type_identifier) @name) @kind.type
(union_item name: (type_identifier) @name) @kind.type
(trait_item name: (type_identifier) @name) @kind.type
(type_item name: (type_identifier) @name) @kind.type

; An impl block is not a declaration — `Ledger` is declared once, by its
; struct — but its span is what tells a method which type it belongs to. It is
; captured as a scope: named, so a method's parent reads as the type, and left
; out of the symbol list, so searching for types does not return `Ledger` once
; per impl block on top of its declaration.
(impl_item type: (type_identifier) @name) @scope
(impl_item type: (generic_type type: (type_identifier) @name)) @scope

(mod_item name: (identifier) @name) @kind.module

(const_item name: (identifier) @name) @kind.constant
(static_item name: (identifier) @name) @kind.constant

; Macros are declarations too, and `macro_rules!` names are exactly what
; someone greps for when a macro misbehaves.
(macro_definition name: (identifier) @name) @kind.function
