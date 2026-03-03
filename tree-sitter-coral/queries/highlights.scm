; Tree-sitter highlight queries for the Coral programming language

; ─── Comments ─────────────────────────────────────────────────
(comment) @comment.line

; ─── Keywords ─────────────────────────────────────────────────
[
  "if"
  "elif"
  "else"
  "while"
  "for"
  "in"
  "match"
  "return"
  "break"
  "continue"
  "unsafe"
] @keyword

[
  "type"
  "enum"
  "store"
  "actor"
  "trait"
  "err"
  "persist"
  "extern"
  "fn"
  "with"
] @keyword.type

"use" @keyword.import

"is" @keyword.operator

; Keyword-like operators (is, isnt, and, or) inside binary_expression
; are aliased as `operator` nodes — match them via the operator capture.
((operator) @keyword.operator
  (#match? @keyword.operator "^(is|isnt|and|or)$"))

; ─── Literals ─────────────────────────────────────────────────
(integer) @number
(float) @number.float

(string) @string
(bytes_literal) @string

(template_string) @string
(template_content) @string
(escape_sequence) @string.escape
(template_interpolation
  "{" @punctuation.special
  "}" @punctuation.special)

(true) @constant.builtin
(false) @constant.builtin
(none) @constant.builtin

(placeholder) @variable.builtin

; ─── Operators ────────────────────────────────────────────────
(operator) @operator

[
  "~"
  "!"
  "@"
  "!!"
  "?"
] @operator

; ─── Punctuation ──────────────────────────────────────────────
[ "(" ")" ] @punctuation.bracket
[ "[" "]" ] @punctuation.bracket
[ "," ] @punctuation.delimiter
[ "." ] @punctuation.delimiter
[ ":" ] @punctuation.delimiter

; ─── Functions ────────────────────────────────────────────────
(function_definition
  "*" @keyword.function
  name: (identifier) @function)

(trait_method_signature
  "*" @keyword.function
  name: (identifier) @function)

(lambda_expression
  "*" @keyword.function
  "fn" @keyword.function)

(extern_function
  "extern" @keyword.function
  "fn" @keyword.function
  name: (identifier) @function)

(call_expression
  function: (identifier) @function.call)

(call_expression
  function: (member_expression
    property: (identifier) @function.method.call))

(parameter
  name: (identifier) @variable.parameter)

; ─── Type / Enum / Store / Actor / Trait definitions ──────────
(type_definition
  name: (identifier) @type)

(enum_definition
  name: (identifier) @type)

(variant_definition
  name: (identifier) @type.enummember)

(store_definition
  name: (identifier) @type)

(persist_store_definition
  name: (identifier) @type)

(actor_definition
  name: (identifier) @type)

(store_actor_definition
  name: (identifier) @type)

(trait_definition
  name: (identifier) @type)

(type_annotation) @type

; ─── Error definitions ────────────────────────────────────────
(error_definition
  name: (identifier) @type)

(error_variant
  name: (identifier) @type)

(error_value
  "err" @keyword.type)

(error_path) @type

; ─── Taxonomy ─────────────────────────────────────────────────
(taxonomy_definition
  "!!" @keyword.type
  name: (identifier) @type)

(taxonomy_path
  "!!" @keyword.type)

; ─── Message handler ──────────────────────────────────────────
(message_handler
  "@" @keyword.function
  name: (identifier) @function)

; ─── Patterns ─────────────────────────────────────────────────
(constructor_pattern
  name: (identifier) @type)

; ─── Bindings / Fields ────────────────────────────────────────
(binding
  name: (identifier) @variable)

(typed_binding
  name: (identifier) @variable)

(field_definition
  name: (identifier) @property)

(error_field
  name: (identifier) @property)

(named_argument
  name: (identifier) @variable.parameter)

(map_entry
  key: (template_string) @property)

(map_entry
  key: (string) @property)

; ─── Member access ────────────────────────────────────────────
(member_expression
  property: (identifier) @property)
