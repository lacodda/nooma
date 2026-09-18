; `import os.path` and `from . import widgets` both name a module; both are
; recorded as written, without resolving them against sys.path.

(import_statement name: (dotted_name) @module)
(import_statement name: (aliased_import name: (dotted_name) @module))

(import_from_statement module_name: (dotted_name) @module)
(import_from_statement module_name: (relative_import) @module)
