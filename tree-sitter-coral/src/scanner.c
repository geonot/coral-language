#include "tree_sitter/parser.h"
#include <string.h>
#include <stdlib.h>
#include <stdbool.h>

/*
 * External scanner for Coral's indentation-sensitive grammar.
 *
 * Emits INDENT / DEDENT / NEWLINE tokens.
 *
 * After a newline, skips blank lines and measures the indent of the
 * next content line. Pushes INDENT or queues DEDENT as needed.
 *
 * Multi-line bracketed expressions (calls, lists, maps) are handled
 * at the grammar level — the grammar rules accept optional _newline
 * tokens inside ( ) and [ ].
 */

enum TokenType {
  INDENT,
  DEDENT,
  NEWLINE,
  WS_NEWLINE,
};

#define MAX_DEPTH 128

typedef struct {
  uint16_t indent_stack[MAX_DEPTH];
  uint8_t indent_depth;
  uint8_t queued_dedents;
  bool pending_indent;
  bool eof_handled;
} Scanner;

static void scanner_init(Scanner *s) {
  s->indent_stack[0] = 0;
  s->indent_depth = 1;
  s->queued_dedents = 0;
  s->pending_indent = false;
  s->eof_handled = false;
}

void *tree_sitter_coral_external_scanner_create(void) {
  Scanner *s = calloc(1, sizeof(Scanner));
  scanner_init(s);
  return s;
}

void tree_sitter_coral_external_scanner_destroy(void *payload) {
  free(payload);
}

unsigned tree_sitter_coral_external_scanner_serialize(void *payload, char *buf) {
  Scanner *s = payload;
  unsigned i = 0;
  buf[i++] = s->indent_depth;
  buf[i++] = s->queued_dedents;
  buf[i++] = s->pending_indent ? 1 : 0;
  buf[i++] = s->eof_handled ? 1 : 0;
  for (uint8_t j = 0; j < s->indent_depth && i + 2 <= TREE_SITTER_SERIALIZATION_BUFFER_SIZE; j++) {
    buf[i++] = s->indent_stack[j] & 0xFF;
    buf[i++] = (s->indent_stack[j] >> 8) & 0xFF;
  }
  return i;
}

void tree_sitter_coral_external_scanner_deserialize(void *payload,
                                                     const char *buf,
                                                     unsigned length) {
  Scanner *s = payload;
  if (length == 0) {
    scanner_init(s);
    return;
  }
  unsigned i = 0;
  s->indent_depth = (uint8_t)buf[i++];
  s->queued_dedents = (uint8_t)buf[i++];
  s->pending_indent = buf[i++] != 0;
  s->eof_handled = buf[i++] != 0;
  for (uint8_t j = 0; j < s->indent_depth && i + 2 <= length; j++) {
    s->indent_stack[j] = (uint16_t)((unsigned char)buf[i] | ((unsigned char)buf[i + 1] << 8));
    i += 2;
  }
}

static uint16_t current_indent(Scanner *s) {
  return s->indent_stack[s->indent_depth - 1];
}

bool tree_sitter_coral_external_scanner_scan(void *payload, TSLexer *lex,
                                              const bool *valid) {
  Scanner *s = payload;

  /* 1. Drain queued DEDENTs */
  if (s->queued_dedents > 0) {
    if (valid[DEDENT]) {
      s->queued_dedents--;
      lex->result_symbol = DEDENT;
      return true;
    }
    /* Parser needs NEWLINE (statement boundary) before more DEDENTs.
       Emit a zero-width NEWLINE so the enclosing statement can close,
       then the parser will ask for DEDENT on the next call. */
    if (valid[NEWLINE]) {
      lex->result_symbol = NEWLINE;
      return true;
    }
  }

  /* 2. Emit pending INDENT */
  if (s->pending_indent) {
    if (valid[INDENT]) {
      s->pending_indent = false;
      lex->result_symbol = INDENT;
      return true;
    }
    /* INDENT is not valid in this context (e.g. inside brackets or
       a construct that doesn't use indentation). Undo the push. */
    if (s->indent_depth > 1) {
      s->indent_depth--;
    }
    s->pending_indent = false;
  }

  if (!valid[NEWLINE]) {
    /* If NEWLINE is not valid but we see a newline character, emit it as
       WS_NEWLINE (which is placed in extras, making it invisible whitespace).
       This handles multi-line expressions inside brackets, where newlines
       should NOT trigger indentation processing. */
    if (valid[WS_NEWLINE] && !lex->eof(lex) &&
        (lex->lookahead == '\n' || lex->lookahead == '\r')) {
      if (lex->lookahead == '\r') lex->advance(lex, false);
      if (!lex->eof(lex) && lex->lookahead == '\n') lex->advance(lex, false);
      lex->mark_end(lex);
      lex->result_symbol = WS_NEWLINE;
      return true;
    }
    return false;
  }

  /* 3. At EOF: one final NEWLINE + queue remaining dedents */
  if (lex->eof(lex)) {
    if (s->eof_handled) return false;
    s->eof_handled = true;
    while (s->indent_depth > 1) {
      s->indent_depth--;
      s->queued_dedents++;
    }
    lex->result_symbol = NEWLINE;
    lex->mark_end(lex);
    return true;
  }

  /* 4. Only trigger on actual newline characters */
  if (lex->lookahead != '\n' && lex->lookahead != '\r') return false;

  lex->result_symbol = NEWLINE;

  /* Consume the newline */
  if (lex->lookahead == '\r') lex->advance(lex, false);
  if (!lex->eof(lex) && lex->lookahead == '\n') lex->advance(lex, false);
  lex->mark_end(lex);

  /* Skip blank lines and measure next content line's indentation */
  uint16_t indent = 0;

  for (;;) {
    indent = 0;
    while (!lex->eof(lex) &&
           (lex->lookahead == ' ' || lex->lookahead == '\t')) {
      indent += (lex->lookahead == '\t') ? 4 : 1;
      lex->advance(lex, true);
    }

    if (lex->eof(lex)) {
      while (s->indent_depth > 1) {
        s->indent_depth--;
        s->queued_dedents++;
      }
      return true;
    }

    if (lex->lookahead == '\n' || lex->lookahead == '\r') {
      /* Blank line: consume and extend token */
      if (lex->lookahead == '\r') lex->advance(lex, false);
      if (!lex->eof(lex) && lex->lookahead == '\n') lex->advance(lex, false);
      lex->mark_end(lex);
      continue;
    }

    break;
  }

  uint16_t cur = current_indent(s);

  if (indent > cur) {
    if (s->indent_depth < MAX_DEPTH) {
      s->indent_stack[s->indent_depth++] = indent;
    }
    s->pending_indent = true;
  } else if (indent < cur) {
    while (s->indent_depth > 1 && s->indent_stack[s->indent_depth - 1] > indent) {
      s->indent_depth--;
      s->queued_dedents++;
    }
  }

  return true;
}
