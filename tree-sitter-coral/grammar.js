/// <reference types="tree-sitter-cli/dsl" />
// @ts-check

// Tree-sitter grammar for the Coral programming language.
//
// Coral is indentation-sensitive (like Python). The external scanner
// handles NEWLINE, INDENT, and DEDENT tokens.
//
// Multi-line expressions inside (...) and [...] are handled by a 4th
// external token (_ws_newline) placed in extras. When the scanner sees
// a newline but NEWLINE is not valid (e.g. inside brackets), it emits
// _ws_newline instead, which is silently skipped as whitespace.

const PREC = {
  PIPELINE: 1,
  TERNARY: 2,
  OR: 3,
  AND: 4,
  COMPARE: 5,
  EQUALITY: 6,
  BIT_OR: 7,
  BIT_XOR: 8,
  BIT_AND: 9,
  SHIFT: 10,
  ADD: 11,
  MUL: 12,
  UNARY: 13,
  CALL: 14,
  MEMBER: 15,
};

module.exports = grammar({
  name: "coral",

  extras: ($) => [/[ \t\r]/, $.comment, $._ws_newline],

  externals: ($) => [$._indent, $._dedent, $._newline, $._ws_newline],

  word: ($) => $.identifier,

  conflicts: ($) => [
    [$.binding, $._primary_expression],
    [$._primary_expression, $.named_argument],
    [$.taxonomy_definition, $.taxonomy_path],
  ],

  rules: {
    source_file: ($) =>
      repeat(choice($._item, $._newline)),

    // ─── Top-level items ──────────────────────────────────────
    _item: ($) =>
      choice(
        $.function_definition,
        $.type_definition,
        $.enum_definition,
        $.store_definition,
        $.actor_definition,
        $.store_actor_definition,
        $.trait_definition,
        $.error_definition,
        $.taxonomy_definition,
        $.extern_function,
        $.use_statement,
        $.persist_store_definition,
        $._statement,
      ),

    // ─── Use / Import ─────────────────────────────────────────
    use_statement: ($) =>
      seq("use", $.dotted_name, $._newline),

    dotted_name: ($) =>
      seq($.identifier, repeat(seq(".", $.identifier))),

    // ─── Function Definition ──────────────────────────────────
    function_definition: ($) =>
      seq(
        "*",
        field("name", $.identifier),
        "(",
        optional(field("parameters", $.parameter_list)),
        ")",
        $._newline,
        field("body", $.block),
      ),

    parameter_list: ($) =>
      commaSep1($.parameter),

    parameter: ($) =>
      seq(
        field("name", $.identifier),
        optional(seq(":", field("type", $.type_annotation))),
        optional(seq("?", field("default", $._expression))),
      ),

    type_annotation: ($) =>
      seq($.identifier, repeat(seq(".", $.identifier))),

    // ─── Extern Function ──────────────────────────────────────
    extern_function: ($) =>
      seq(
        "extern",
        "fn",
        field("name", $.identifier),
        "(",
        optional(field("parameters", $.parameter_list)),
        ")",
        optional(seq(":", field("return_type", $.type_annotation))),
        $._newline,
      ),

    // ─── Type Definition ──────────────────────────────────────
    type_definition: ($) =>
      seq(
        "type",
        field("name", $.identifier),
        optional($._trait_with_clause),
        $._newline,
        optional(field("body", $.type_body)),
      ),

    type_body: ($) =>
      seq(
        $._indent,
        repeat(choice($.field_definition, $.function_definition, $.with_clause, $._newline)),
        $._dedent,
      ),

    field_definition: ($) =>
      seq(
        optional("&"),
        field("name", $.identifier),
        optional(choice(
          seq("?", field("default", $._expression)),
          seq("is", field("default", $._expression)),
        )),
        $._newline,
      ),

    _trait_with_clause: ($) =>
      seq("with", commaSep1($.identifier)),

    with_clause: ($) =>
      seq("with", commaSep1($.identifier), $._newline),

    // ─── Enum Definition ──────────────────────────────────────
    enum_definition: ($) =>
      seq("enum", field("name", $.identifier), $._newline,
        optional(field("body", $.enum_body))),

    enum_body: ($) =>
      seq($._indent, repeat(choice($.variant_definition, $._newline)), $._dedent),

    variant_definition: ($) =>
      seq(
        field("name", $.identifier),
        optional(seq("(", optional(commaSep1($.identifier)), ")")),
        $._newline,
      ),

    // ─── Store / Actor / Trait ──────────────────────────────
    store_definition: ($) =>
      seq("store", field("name", $.identifier), $._newline,
        optional(field("body", $.type_body))),

    persist_store_definition: ($) =>
      seq("persist", "store", field("name", $.identifier), $._newline,
        optional(field("body", $.type_body))),

    actor_definition: ($) =>
      seq("actor", field("name", $.identifier), $._newline,
        optional(field("body", $.actor_body))),

    store_actor_definition: ($) =>
      seq("store", "actor", field("name", $.identifier), $._newline,
        optional(field("body", $.actor_body))),

    actor_body: ($) =>
      seq($._indent, repeat(choice($.field_definition, $.function_definition, $.message_handler, $._newline)), $._dedent),

    message_handler: ($) =>
      seq("@", field("name", $.identifier), "(", optional(field("parameters", $.parameter_list)), ")", $._newline,
        field("body", $.block)),

    trait_definition: ($) =>
      seq("trait", field("name", $.identifier), $._newline,
        optional(field("body", $.trait_body))),

    trait_body: ($) =>
      seq($._indent, repeat(choice($.function_definition, $.trait_method_signature, $.with_clause, $._newline)), $._dedent),

    trait_method_signature: ($) =>
      seq("*", field("name", $.identifier), "(", optional(field("parameters", $.parameter_list)), ")", $._newline),

    // ─── Error / Taxonomy ─────────────────────────────────────
    error_definition: ($) =>
      prec(10, seq("err", field("name", $.identifier), $._newline,
        optional(field("body", $.error_body)))),

    error_body: ($) =>
      seq($._indent, repeat(choice($.error_variant, $.error_field, $._newline)), $._dedent),

    error_variant: ($) =>
      seq("err", field("name", $.identifier), $._newline),

    error_field: ($) =>
      seq(field("name", $.identifier), "is", field("value", $._expression), $._newline),

    taxonomy_definition: ($) =>
      seq("!!", field("name", $.identifier), $._newline,
        optional(field("body", $.taxonomy_body))),

    taxonomy_body: ($) =>
      seq($._indent, repeat(choice($.taxonomy_definition, $.binding, $._newline)), $._dedent),

    // ─── Block ────────────────────────────────────────────────
    block: ($) =>
      seq($._indent, repeat1(choice($._statement, $._newline)), $._dedent),

    // ─── Statements ───────────────────────────────────────────
    _statement: ($) =>
      choice(
        $.binding,
        $.typed_binding,
        $.return_statement,
        $.if_statement,
        $.unless_statement,
        $.while_statement,
        $.until_statement,
        $.loop_statement,
        $.for_statement,
        $.break_statement,
        $.continue_statement,
        $.unsafe_block,
        $.expression_statement,
      ),

    binding: ($) =>
      seq(field("name", $.identifier), "is", field("value", $._expression), $._newline),

    typed_binding: ($) =>
      seq(field("name", $.identifier), ":", field("type", $.type_annotation), "is", field("value", $._expression), $._newline),

    return_statement: ($) =>
      seq("return", optional($._expression), $._newline),

    break_statement: ($) => seq("break", $._newline),
    continue_statement: ($) => seq("continue", $._newline),

    if_statement: ($) =>
      seq("if", field("condition", $._expression), $._newline,
        field("body", $.block),
        repeat($.elif_clause),
        optional($.else_clause)),

    elif_clause: ($) =>
      seq("elif", field("condition", $._expression), $._newline, field("body", $.block)),

    else_clause: ($) =>
      seq("else", $._newline, field("body", $.block)),

    while_statement: ($) =>
      seq("while", field("condition", $._expression), $._newline, field("body", $.block)),

    unless_statement: ($) =>
      seq("unless", field("condition", $._expression), $._newline, field("body", $.block)),

    until_statement: ($) =>
      seq("until", field("condition", $._expression), $._newline, field("body", $.block)),

    loop_statement: ($) =>
      seq("loop", $._newline, field("body", $.block)),

    for_statement: ($) =>
      seq("for", field("variable", $.identifier), "in", field("iterable", $._expression), $._newline,
        field("body", $.block)),

    match_block: ($) =>
      seq($._indent, repeat1(choice($.match_arm, $._newline)), $._dedent),

    match_arm: ($) =>
      choice(
        seq(field("pattern", $.pattern), "?", field("body", $._expression), $._newline),
        seq(field("pattern", $.pattern), "?", $._newline, field("body", $.block)),
        seq("!", field("body", $._expression), $._newline),
        seq("_", "?", field("body", $._expression), $._newline),
      ),

    pattern: ($) =>
      choice(
        $.integer, $.float, $.string, $.template_string,
        $.true, $.false, $.none,
        $.constructor_pattern, $.list_pattern, $.tuple_pattern,
        $.identifier,
      ),

    constructor_pattern: ($) =>
      seq(field("name", $.identifier), "(", optional(commaSep1($.pattern)), ")"),

    list_pattern: ($) =>
      seq("[", optional(commaSep1($.pattern)), "]"),

    tuple_pattern: ($) =>
      seq("(", commaSep1($.pattern), ")"),

    unsafe_block: ($) =>
      seq("unsafe", $._newline, field("body", $.block)),

    expression_statement: ($) =>
      seq($._expression, $._newline),

    // ─── Expressions ──────────────────────────────────────────
    _expression: ($) =>
      choice(
        $.ternary_expression,
        $.error_propagation,
        $.guard_expression,
        $.throw_expression,
        $.pipeline_expression,
        $.binary_expression,
        $.unary_expression,
        $.call_expression,
        $.member_expression,
        $.index_expression,
        $.lambda_expression,
        $.match_expression,
        $.when_expression,
        $.inline_asm,
        $.ptr_load,
        $._primary_expression,
      ),

    _primary_expression: ($) =>
      choice(
        $.identifier, $.integer, $.float,
        $.string, $.template_string, $.bytes_literal,
        $.true, $.false, $.none,
        $.placeholder,
        $.list_literal, $.map_literal, $.tuple_expression,
        $.error_value, $.taxonomy_path,
        $.parenthesized_expression, $.unit,
      ),

    binary_expression: ($) => {
      const table = [
        ["or", PREC.OR], ["and", PREC.AND],
        ["is", PREC.EQUALITY], ["isnt", PREC.EQUALITY],
        [">", PREC.COMPARE], [">=", PREC.COMPARE],
        ["<", PREC.COMPARE], ["<=", PREC.COMPARE],
        ["|", PREC.BIT_OR], ["^", PREC.BIT_XOR], ["&", PREC.BIT_AND],
        ["<<", PREC.SHIFT], [">>", PREC.SHIFT],
        ["+", PREC.ADD], ["-", PREC.ADD],
        ["*", PREC.MUL], ["/", PREC.MUL], ["%", PREC.MUL],
      ];
      return choice(
        ...table.map(([op, p]) =>
          prec.left(p, seq(
            field("left", $._expression),
            field("operator", alias(op, $.operator)),
            field("right", $._expression),
          )),
        ),
      );
    },

    unary_expression: ($) =>
      prec(PREC.UNARY, seq(field("operator", choice("-", "!")), field("operand", $._expression))),

    pipeline_expression: ($) =>
      prec.left(PREC.PIPELINE, seq(field("left", $._expression), "~", field("right", $._expression))),

    ternary_expression: ($) =>
      prec.right(PREC.TERNARY, seq(
        field("condition", $._expression), "?",
        field("consequence", $._expression), "!",
        field("alternative", $._expression),
      )),

    error_propagation: ($) =>
      prec.right(PREC.TERNARY, seq(field("expression", $._expression), "!", "return", "err")),

    guard_expression: ($) =>
      prec.right(PREC.TERNARY, seq(field("condition", $._expression), "?", "!", field("error", $._expression))),

    throw_expression: ($) =>
      prec.right(PREC.TERNARY, seq("!", field("value", $._expression))),

    // ── Call / member / index ─────────────────────────────────
    // Arguments allow _newline between items for multi-line calls.
    call_expression: ($) =>
      prec(PREC.CALL, seq(
        field("function", $._expression),
        "(",
        optional(field("arguments", $.argument_list)),
        ")",
      )),

    argument_list: ($) =>
      seq(
        $._argument,
        repeat(seq(",", $._argument)),
        optional(","),
      ),

    _argument: ($) => choice($.named_argument, $._expression),

    named_argument: ($) =>
      seq(field("name", $.identifier), "is", field("value", $._expression)),

    member_expression: ($) =>
      prec(PREC.MEMBER, seq(field("object", $._expression), ".", field("property", $.identifier))),

    index_expression: ($) =>
      prec(PREC.CALL, seq(field("object", $._expression), "[", field("index", $._expression), "]")),

    lambda_expression: ($) =>
      choice(
        seq("*", "fn", "(", optional(field("parameters", $.parameter_list)), ")", field("body", $._expression)),
        seq("*", "fn", "(", optional(field("parameters", $.parameter_list)), ")", $._newline, field("body", $.block)),
      ),

    match_expression: ($) =>
      seq("match", field("value", $._expression), $._newline, field("arms", $.match_block)),

    when_expression: ($) =>
      seq("when", $._newline, field("arms", $.when_block)),

    when_block: ($) =>
      seq($._indent, repeat1(choice($.when_arm, $._newline)), $._dedent),

    when_arm: ($) =>
      choice(
        seq(field("condition", $._expression), "?", field("body", $._expression), $._newline),
        seq("_", "?", field("body", $._expression), $._newline),
      ),

    inline_asm: ($) =>
      seq("asm", "(", field("template", $.string),
        optional(seq(",", $.asm_operand_list)), ")"),

    asm_operand_list: ($) =>
      seq($.asm_operand, repeat(seq(",", $.asm_operand))),

    asm_operand: ($) =>
      seq(field("register", $.identifier), ":", field("value", $._expression)),

    ptr_load: ($) =>
      prec(PREC.UNARY, seq("@", field("address", $._expression))),

    parenthesized_expression: ($) =>
      seq("(", $._expression, ")"),

    tuple_expression: ($) =>
      seq("(", $._expression, ",", optional(seq($._expression, repeat(seq(",", $._expression)))), optional(","), ")"),

    unit: (_$) => seq("(", ")"),

    // ─── Literals ─────────────────────────────────────────────
    identifier: (_$) => /[a-zA-Z_][a-zA-Z0-9_]*/,

    integer: (_$) => token(choice(
      /0[xX][0-9a-fA-F][0-9a-fA-F_]*/,
      /0[bB][01][01_]*/,
      /0[oO][0-7][0-7_]*/,
      /[0-9][0-9_]*/,
    )),

    float: (_$) => token(/[0-9][0-9_]*\.[0-9][0-9_]*/),

    string: (_$) => token(seq('"', repeat(choice(/[^"\\]/, /\\./)), '"')),

    template_string: ($) =>
      seq("'", repeat(choice($.template_content, $.template_interpolation, $.escape_sequence)), "'"),

    template_content: (_$) => token.immediate(prec(1, /[^'\\{]+/)),

    template_interpolation: ($) =>
      seq(token.immediate("{"), $._expression, "}"),

    escape_sequence: (_$) =>
      token.immediate(seq("\\", /[nrt0\\'"{}]/)),

    bytes_literal: (_$) =>
      token(seq(/[bB]/, '"', repeat(choice(/[^"\\]/, /\\./)), '"')),

    true: (_$) => "true",
    false: (_$) => "false",
    none: (_$) => "none",

    placeholder: (_$) => token(choice("$", /\$[0-9]+/)),

    list_literal: ($) =>
      seq("[",
        optional(seq(
          $._expression,
          repeat(seq(",", $._expression)),
          optional(","),
        )),
        "]"),

    map_literal: ($) =>
      seq("map", "(",
        optional(seq(
          $.map_entry,
          repeat(seq(",", $.map_entry)),
          optional(","),
        )),
        ")"),

    map_entry: ($) =>
      seq(field("key", $._expression), "is", field("value", $._expression)),

    error_value: ($) =>
      seq("err", optional($.error_path)),

    error_path: ($) =>
      seq($.identifier, repeat(seq(":", $.identifier))),

    taxonomy_path: ($) =>
      seq("!!", $.identifier, repeat(seq(":", $.identifier))),

    // ─── Comment ──────────────────────────────────────────────
    comment: (_$) => token(seq("#", /.*/)),
  },
});

function commaSep1(rule) {
  return seq(rule, repeat(seq(",", rule)));
}
