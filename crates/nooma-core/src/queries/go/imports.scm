; Import paths, quoted; the quotes are trimmed by the caller.

(import_declaration (import_spec path: (interpreted_string_literal) @module))
(import_declaration
  (import_spec_list
    (import_spec path: (interpreted_string_literal) @module)))
