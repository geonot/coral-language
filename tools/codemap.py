#!/usr/bin/env python3
"""
codemap.py — Generate a structural map of a codebase for LLM agent navigation.

Produces a Markdown document with:
  - File index with line counts and descriptions
  - Per-file structural maps: classes, functions, methods, traits, types
  - Symbol definitions with line numbers, parameters, return types
  - Caller/callee relationships (where detectable)
  - Docstrings and annotations

Supports: Rust, Python, TypeScript/JavaScript, Coral, Go, C/C++, Java, Ruby

Usage:
    python tools/codemap.py [directory] [options]
    python tools/codemap.py src/ --output codemap.md
    python tools/codemap.py . --include "*.rs" "*.py" --exclude "target/*"
    python tools/codemap.py --help
"""

import argparse
import os
import re
import sys
from dataclasses import dataclass, field
from pathlib import Path
from typing import Optional


# ---------------------------------------------------------------------------
# Data model
# ---------------------------------------------------------------------------

@dataclass
class Symbol:
    """A named code symbol (function, class, method, trait, type, etc.)."""
    kind: str              # "fn", "method", "class", "struct", "trait", "enum", "type", "impl", "const", "module", "interface", "actor", "store"
    name: str
    line: int
    end_line: Optional[int] = None
    params: Optional[str] = None
    return_type: Optional[str] = None
    doc: Optional[str] = None
    visibility: Optional[str] = None   # "pub", "pub(crate)", "private", "protected", etc.
    decorators: list = field(default_factory=list)
    children: list = field(default_factory=list)   # nested symbols
    calls: list = field(default_factory=list)       # functions called within this symbol
    parent: Optional[str] = None

    @property
    def display_kind(self) -> str:
        kind_map = {
            "fn": "fn",
            "method": "method",
            "class": "class",
            "struct": "struct",
            "trait": "trait",
            "enum": "enum",
            "type": "type",
            "impl": "impl",
            "const": "const",
            "module": "mod",
            "interface": "interface",
            "actor": "actor",
            "store": "store",
            "handler": "handler",
        }
        return kind_map.get(self.kind, self.kind)


@dataclass
class FileMap:
    """Structural map of a single source file."""
    path: str
    language: str
    line_count: int
    symbols: list = field(default_factory=list)
    imports: list = field(default_factory=list)
    description: Optional[str] = None  # from file-level docstring


# ---------------------------------------------------------------------------
# Language detection
# ---------------------------------------------------------------------------

LANG_EXTENSIONS = {
    ".rs": "rust",
    ".py": "python",
    ".ts": "typescript",
    ".tsx": "typescript",
    ".js": "javascript",
    ".jsx": "javascript",
    ".go": "go",
    ".c": "c",
    ".h": "c",
    ".cpp": "cpp",
    ".hpp": "cpp",
    ".cc": "cpp",
    ".java": "java",
    ".rb": "ruby",
    ".coral": "coral",
    ".toml": "toml",
    ".yaml": "yaml",
    ".yml": "yaml",
    ".json": "json",
    ".md": "markdown",
}

def detect_language(filepath: str) -> Optional[str]:
    ext = Path(filepath).suffix.lower()
    return LANG_EXTENSIONS.get(ext)


# ---------------------------------------------------------------------------
# Parsers — one per language family
# ---------------------------------------------------------------------------

class BaseParser:
    """Base class for language-specific symbol extraction."""

    def parse(self, filepath: str, content: str) -> FileMap:
        lines = content.split("\n")
        fm = FileMap(
            path=filepath,
            language=self.__class__.__name__.replace("Parser", "").lower(),
            line_count=len(lines),
        )
        fm.description = self._extract_file_doc(lines)
        fm.symbols = self._extract_symbols(lines, content)
        fm.imports = self._extract_imports(lines)
        return fm

    def _extract_file_doc(self, lines: list) -> Optional[str]:
        return None

    def _extract_symbols(self, lines: list, content: str) -> list:
        return []

    def _extract_imports(self, lines: list) -> list:
        return []

    @staticmethod
    def _collect_doc_comment(lines: list, target_line: int, prefixes: tuple) -> Optional[str]:
        """Collect doc-comment lines immediately above target_line."""
        docs = []
        i = target_line - 2  # 0-indexed, line above
        while i >= 0:
            stripped = lines[i].strip()
            if any(stripped.startswith(p) for p in prefixes):
                for p in prefixes:
                    if stripped.startswith(p):
                        docs.insert(0, stripped[len(p):].strip())
                        break
                i -= 1
            else:
                break
        return " ".join(docs) if docs else None

    @staticmethod
    def _find_end_line_brace(lines: list, start: int) -> int:
        """Find closing brace for a block starting at `start` (0-indexed)."""
        depth = 0
        for i in range(start, len(lines)):
            depth += lines[i].count("{") - lines[i].count("}")
            if depth <= 0 and i > start:
                return i + 1
        return len(lines)

    @staticmethod
    def _find_end_line_indent(lines: list, start: int) -> int:
        """Find end of indentation block starting at `start` (0-indexed)."""
        if start >= len(lines):
            return start + 1
        base_indent = len(lines[start]) - len(lines[start].lstrip())
        for i in range(start + 1, len(lines)):
            line = lines[i]
            if line.strip() == "":
                continue
            indent = len(line) - len(line.lstrip())
            if indent <= base_indent:
                return i  # 1-indexed in output later
        return len(lines)


class RustParser(BaseParser):
    # Patterns
    FN_RE = re.compile(
        r'^(\s*)(pub(?:\(crate\))?\s+)?(?:async\s+)?fn\s+(\w+)\s*(?:<[^>]*>)?\s*\(([^)]*)\)(?:\s*->\s*(.+?))?\s*\{?',
    )
    STRUCT_RE = re.compile(r'^(\s*)(pub(?:\(crate\))?\s+)?struct\s+(\w+)(?:<[^>]*>)?')
    ENUM_RE = re.compile(r'^(\s*)(pub(?:\(crate\))?\s+)?enum\s+(\w+)(?:<[^>]*>)?')
    TRAIT_RE = re.compile(r'^(\s*)(pub(?:\(crate\))?\s+)?trait\s+(\w+)(?:<[^>]*>)?')
    IMPL_RE = re.compile(r'^(\s*)impl(?:<[^>]*>)?\s+(?:(\w+)\s+for\s+)?(\w+)(?:<[^>]*>)?')
    CONST_RE = re.compile(r'^(\s*)(pub(?:\(crate\))?\s+)?const\s+(\w+)\s*:\s*(.+?)\s*=')
    MOD_RE = re.compile(r'^(\s*)(pub(?:\(crate\))?\s+)?mod\s+(\w+)')
    USE_RE = re.compile(r'^\s*use\s+(.+?);')

    def _extract_file_doc(self, lines):
        return self._collect_doc_comment(lines, 1, ("//!", ))

    def _extract_symbols(self, lines, content):
        symbols = []
        i = 0
        while i < len(lines):
            line = lines[i]
            lineno = i + 1

            # Skip attributes but collect them
            decorators = []
            while i < len(lines) and lines[i].strip().startswith("#["):
                decorators.append(lines[i].strip())
                i += 1
                lineno = i + 1
            if i >= len(lines):
                break
            line = lines[i]

            doc = self._collect_doc_comment(lines, lineno, ("///", "//!"))

            # Struct
            m = self.STRUCT_RE.match(line)
            if m:
                vis = (m.group(2) or "").strip() or None
                name = m.group(3)
                end = self._find_end_line_brace(lines, i) if "{" in line or (i + 1 < len(lines) and "{" in lines[i + 1]) else lineno
                symbols.append(Symbol(kind="struct", name=name, line=lineno, end_line=end, visibility=vis, doc=doc, decorators=decorators))
                i += 1
                continue

            # Enum
            m = self.ENUM_RE.match(line)
            if m:
                vis = (m.group(2) or "").strip() or None
                name = m.group(3)
                end = self._find_end_line_brace(lines, i)
                symbols.append(Symbol(kind="enum", name=name, line=lineno, end_line=end, visibility=vis, doc=doc, decorators=decorators))
                i += 1
                continue

            # Trait
            m = self.TRAIT_RE.match(line)
            if m:
                vis = (m.group(2) or "").strip() or None
                name = m.group(3)
                end = self._find_end_line_brace(lines, i)
                children = self._extract_methods_in_block(lines, i, end - 1)
                symbols.append(Symbol(kind="trait", name=name, line=lineno, end_line=end, visibility=vis, doc=doc, children=children, decorators=decorators))
                i = end
                continue

            # Impl block
            m = self.IMPL_RE.match(line)
            if m:
                trait_name = m.group(2)
                type_name = m.group(3)
                name = f"{trait_name} for {type_name}" if trait_name else type_name
                end = self._find_end_line_brace(lines, i)
                children = self._extract_methods_in_block(lines, i, end - 1)
                symbols.append(Symbol(kind="impl", name=name, line=lineno, end_line=end, children=children, decorators=decorators))
                i = end
                continue

            # Standalone function
            m = self.FN_RE.match(line)
            if m:
                vis = (m.group(2) or "").strip() or None
                name = m.group(3)
                params = m.group(4).strip()
                ret = (m.group(5) or "").strip() or None
                end = self._find_end_line_brace(lines, i)
                calls = self._extract_calls(lines, i, end)
                symbols.append(Symbol(kind="fn", name=name, line=lineno, end_line=end, params=params, return_type=ret, visibility=vis, doc=doc, calls=calls, decorators=decorators))
                i = end
                continue

            # Const
            m = self.CONST_RE.match(line)
            if m:
                vis = (m.group(2) or "").strip() or None
                name = m.group(3)
                ret = m.group(4).strip()
                symbols.append(Symbol(kind="const", name=name, line=lineno, return_type=ret, visibility=vis, doc=doc, decorators=decorators))
                i += 1
                continue

            # Module
            m = self.MOD_RE.match(line)
            if m and ";" in line:
                vis = (m.group(2) or "").strip() or None
                name = m.group(3)
                symbols.append(Symbol(kind="module", name=name, line=lineno, visibility=vis, decorators=decorators))
                i += 1
                continue

            i += 1
        return symbols

    def _extract_methods_in_block(self, lines, block_start, block_end):
        methods = []
        i = block_start + 1
        while i < block_end and i < len(lines):
            line = lines[i]
            lineno = i + 1
            doc = self._collect_doc_comment(lines, lineno, ("///",))
            m = self.FN_RE.match(line)
            if m:
                vis = (m.group(2) or "").strip() or None
                name = m.group(3)
                params = m.group(4).strip()
                ret = (m.group(5) or "").strip() or None
                end = self._find_end_line_brace(lines, i)
                calls = self._extract_calls(lines, i, end)
                methods.append(Symbol(kind="method", name=name, line=lineno, end_line=end, params=params, return_type=ret, visibility=vis, doc=doc, calls=calls))
                i = end
                continue
            i += 1
        return methods

    def _extract_calls(self, lines, start, end):
        """Extract function/method call names from a block."""
        calls = set()
        call_re = re.compile(r'(\w+)\s*\(')
        for i in range(start + 1, min(end, len(lines))):
            for m in call_re.finditer(lines[i]):
                name = m.group(1)
                # Skip keywords and common non-function tokens
                if name not in ("if", "while", "for", "match", "let", "return", "Some", "None", "Ok", "Err", "assert", "assert_eq", "vec", "format", "panic", "todo", "unimplemented", "unreachable", "cfg", "derive", "test", "println", "eprintln", "write", "writeln"):
                    calls.add(name)
        return sorted(calls)

    def _extract_imports(self, lines):
        imports = []
        for line in lines:
            m = self.USE_RE.match(line)
            if m:
                imports.append(m.group(1).strip())
        return imports


class PythonParser(BaseParser):
    CLASS_RE = re.compile(r'^(\s*)class\s+(\w+)(?:\((.*?)\))?\s*:')
    FN_RE = re.compile(r'^(\s*)(?:async\s+)?def\s+(\w+)\s*\(([^)]*)\)(?:\s*->\s*(.+?))?\s*:')
    DECORATOR_RE = re.compile(r'^(\s*)@(\w+.*)')
    IMPORT_RE = re.compile(r'^\s*(?:from\s+(\S+)\s+)?import\s+(.+)')

    def _extract_file_doc(self, lines):
        for i, line in enumerate(lines):
            s = line.strip()
            if s == "" or s.startswith("#"):
                continue
            if s.startswith('"""') or s.startswith("'''"):
                quote = s[:3]
                if s.count(quote) >= 2:
                    return s[3:s.index(quote, 3)].strip()
                doc_lines = [s[3:]]
                for j in range(i + 1, len(lines)):
                    if quote in lines[j]:
                        doc_lines.append(lines[j][:lines[j].index(quote)].strip())
                        return " ".join(dl for dl in doc_lines if dl)
                    doc_lines.append(lines[j].strip())
            break
        return None

    def _extract_symbols(self, lines, content):
        symbols = []
        i = 0
        while i < len(lines):
            line = lines[i]
            lineno = i + 1

            decorators = []
            while i < len(lines) and self.DECORATOR_RE.match(lines[i]):
                decorators.append(lines[i].strip())
                i += 1
                lineno = i + 1
            if i >= len(lines):
                break
            line = lines[i]

            doc = self._collect_doc_comment(lines, lineno, ("#",))

            m = self.CLASS_RE.match(line)
            if m:
                indent = len(m.group(1))
                name = m.group(2)
                bases = m.group(3)
                end = self._find_end_line_indent(lines, i)
                docstring = self._extract_docstring(lines, i + 1)
                children = self._extract_methods(lines, i + 1, end, indent)
                symbols.append(Symbol(kind="class", name=name, line=lineno, end_line=end, params=bases, doc=docstring or doc, children=children, decorators=decorators))
                i = end
                continue

            m = self.FN_RE.match(line)
            if m:
                name = m.group(2)
                params = m.group(3).strip()
                ret = (m.group(4) or "").strip() or None
                end = self._find_end_line_indent(lines, i)
                docstring = self._extract_docstring(lines, i + 1)
                calls = self._extract_calls(lines, i, end)
                symbols.append(Symbol(kind="fn", name=name, line=lineno, end_line=end, params=params, return_type=ret, doc=docstring or doc, calls=calls, decorators=decorators))
                i = end
                continue

            i += 1
        return symbols

    def _extract_methods(self, lines, start, end, parent_indent):
        methods = []
        i = start
        while i < end and i < len(lines):
            line = lines[i]
            m = self.FN_RE.match(line)
            if m:
                indent = len(m.group(1))
                if indent > parent_indent:
                    name = m.group(2)
                    params = m.group(3).strip()
                    ret = (m.group(4) or "").strip() or None
                    m_end = self._find_end_line_indent(lines, i)
                    docstring = self._extract_docstring(lines, i + 1)
                    calls = self._extract_calls(lines, i, m_end)
                    methods.append(Symbol(kind="method", name=name, line=i + 1, end_line=m_end, params=params, return_type=ret, doc=docstring, calls=calls))
                    i = m_end
                    continue
            i += 1
        return methods

    @staticmethod
    def _extract_docstring(lines, start):
        if start >= len(lines):
            return None
        s = lines[start].strip()
        if s.startswith('"""') or s.startswith("'''"):
            quote = s[:3]
            if s.count(quote) >= 2:
                return s[3:s.rindex(quote)].strip()
            doc_lines = [s[3:]]
            for j in range(start + 1, len(lines)):
                if quote in lines[j]:
                    doc_lines.append(lines[j][:lines[j].index(quote)].strip())
                    return " ".join(dl for dl in doc_lines if dl)
                doc_lines.append(lines[j].strip())
        return None

    def _extract_calls(self, lines, start, end):
        calls = set()
        call_re = re.compile(r'(\w+)\s*\(')
        keywords = {"if", "while", "for", "def", "class", "return", "with", "as", "in", "not", "and", "or", "print", "range", "len", "str", "int", "float", "list", "dict", "set", "type", "super", "isinstance", "hasattr", "getattr", "setattr"}
        for i in range(start + 1, min(end, len(lines))):
            for m in call_re.finditer(lines[i]):
                name = m.group(1)
                if name not in keywords:
                    calls.add(name)
        return sorted(calls)

    def _extract_imports(self, lines):
        imports = []
        for line in lines:
            m = self.IMPORT_RE.match(line)
            if m:
                imports.append(line.strip())
        return imports


class CoralParser(BaseParser):
    """Parser for the Coral programming language."""
    FN_RE = re.compile(r'^(\s*)\*(\w+)\s*\(([^)]*)\)')
    TYPE_RE = re.compile(r'^(\s*)type\s+(\w+)(?:\[([^\]]*)\])?')
    ENUM_RE = re.compile(r'^(\s*)enum\s+(\w+)(?:\[([^\]]*)\])?')
    STORE_RE = re.compile(r'^(\s*)store\s+(\w+)')
    ACTOR_RE = re.compile(r'^(\s*)actor\s+(\w+)')
    TRAIT_RE = re.compile(r'^(\s*)trait\s+(\w+)')
    HANDLER_RE = re.compile(r'^(\s*)@(\w+)\s*\(([^)]*)\)')
    USE_RE = re.compile(r'^\s*use\s+(.+)')

    def _extract_file_doc(self, lines):
        return self._collect_doc_comment(lines, 1, ("##",))

    def _extract_symbols(self, lines, content):
        symbols = []
        i = 0
        while i < len(lines):
            line = lines[i]
            lineno = i + 1
            doc = self._collect_doc_comment(lines, lineno, ("##",))

            # Actor
            m = self.ACTOR_RE.match(line)
            if m:
                name = m.group(2)
                end = self._find_end_line_indent(lines, i)
                children = self._extract_handlers_and_methods(lines, i + 1, end)
                symbols.append(Symbol(kind="actor", name=name, line=lineno, end_line=end, doc=doc, children=children))
                i = end
                continue

            # Store
            m = self.STORE_RE.match(line)
            if m:
                name = m.group(2)
                end = self._find_end_line_indent(lines, i)
                children = self._extract_coral_methods(lines, i + 1, end)
                symbols.append(Symbol(kind="store", name=name, line=lineno, end_line=end, doc=doc, children=children))
                i = end
                continue

            # Trait
            m = self.TRAIT_RE.match(line)
            if m:
                name = m.group(2)
                end = self._find_end_line_indent(lines, i)
                children = self._extract_coral_methods(lines, i + 1, end)
                symbols.append(Symbol(kind="trait", name=name, line=lineno, end_line=end, doc=doc, children=children))
                i = end
                continue

            # Type
            m = self.TYPE_RE.match(line)
            if m:
                name = m.group(2)
                params = m.group(3)
                end = self._find_end_line_indent(lines, i)
                children = self._extract_coral_methods(lines, i + 1, end)
                symbols.append(Symbol(kind="type", name=name, line=lineno, end_line=end, params=params, doc=doc, children=children))
                i = end
                continue

            # Enum
            m = self.ENUM_RE.match(line)
            if m:
                name = m.group(2)
                params = m.group(3)
                end = self._find_end_line_indent(lines, i)
                symbols.append(Symbol(kind="enum", name=name, line=lineno, end_line=end, params=params, doc=doc))
                i = end
                continue

            # Function
            m = self.FN_RE.match(line)
            if m:
                indent = len(m.group(1))
                if indent == 0:  # top-level function only
                    name = m.group(2)
                    params = m.group(3).strip()
                    end = self._find_end_line_indent(lines, i)
                    symbols.append(Symbol(kind="fn", name=name, line=lineno, end_line=end, params=params, doc=doc))
                    i = end
                    continue

            i += 1
        return symbols

    def _extract_handlers_and_methods(self, lines, start, end):
        children = []
        i = start
        while i < end and i < len(lines):
            line = lines[i]
            m = self.HANDLER_RE.match(line)
            if m:
                name = m.group(2)
                params = m.group(3).strip()
                h_end = self._find_end_line_indent(lines, i)
                children.append(Symbol(kind="handler", name=name, line=i + 1, end_line=h_end, params=params))
                i = h_end
                continue
            m = self.FN_RE.match(line)
            if m:
                name = m.group(2)
                params = m.group(3).strip()
                m_end = self._find_end_line_indent(lines, i)
                children.append(Symbol(kind="method", name=name, line=i + 1, end_line=m_end, params=params))
                i = m_end
                continue
            i += 1
        return children

    def _extract_coral_methods(self, lines, start, end):
        methods = []
        i = start
        while i < end and i < len(lines):
            m = self.FN_RE.match(lines[i])
            if m:
                name = m.group(2)
                params = m.group(3).strip()
                m_end = self._find_end_line_indent(lines, i)
                methods.append(Symbol(kind="method", name=name, line=i + 1, end_line=m_end, params=params))
                i = m_end
                continue
            i += 1
        return methods

    def _extract_imports(self, lines):
        imports = []
        for line in lines:
            m = self.USE_RE.match(line)
            if m:
                imports.append(m.group(1).strip())
        return imports


class TypeScriptParser(BaseParser):
    """Parser for TypeScript/JavaScript."""
    CLASS_RE = re.compile(r'^(\s*)(?:export\s+)?(?:abstract\s+)?class\s+(\w+)(?:<[^>]*>)?(?:\s+extends\s+\w+)?(?:\s+implements\s+[\w,\s]+)?\s*\{')
    INTERFACE_RE = re.compile(r'^(\s*)(?:export\s+)?interface\s+(\w+)(?:<[^>]*>)?\s*(?:extends\s+[\w,\s<>]+)?\s*\{')
    FN_RE = re.compile(r'^(\s*)(?:export\s+)?(?:async\s+)?function\s+(\w+)\s*(?:<[^>]*>)?\s*\(([^)]*)\)(?:\s*:\s*(.+?))?\s*\{')
    ARROW_RE = re.compile(r'^(\s*)(?:export\s+)?(?:const|let|var)\s+(\w+)\s*=\s*(?:async\s+)?\(([^)]*)\)(?:\s*:\s*(.+?))?\s*=>')
    METHOD_RE = re.compile(r'^(\s*)(?:public|private|protected|static|async|readonly|\s)*(\w+)\s*\(([^)]*)\)(?:\s*:\s*(.+?))?\s*\{')
    IMPORT_RE = re.compile(r"""^\s*import\s+.*?from\s+['"](.+?)['"]""")

    def _extract_symbols(self, lines, content):
        symbols = []
        i = 0
        while i < len(lines):
            line = lines[i]
            lineno = i + 1

            m = self.CLASS_RE.match(line)
            if m:
                name = m.group(2)
                end = self._find_end_line_brace(lines, i)
                children = self._extract_ts_methods(lines, i + 1, end - 1)
                symbols.append(Symbol(kind="class", name=name, line=lineno, end_line=end, children=children))
                i = end
                continue

            m = self.INTERFACE_RE.match(line)
            if m:
                name = m.group(2)
                end = self._find_end_line_brace(lines, i)
                symbols.append(Symbol(kind="interface", name=name, line=lineno, end_line=end))
                i = end
                continue

            m = self.FN_RE.match(line)
            if m:
                name = m.group(2)
                params = m.group(3).strip()
                ret = (m.group(4) or "").strip() or None
                end = self._find_end_line_brace(lines, i)
                symbols.append(Symbol(kind="fn", name=name, line=lineno, end_line=end, params=params, return_type=ret))
                i = end
                continue

            m = self.ARROW_RE.match(line)
            if m:
                name = m.group(2)
                params = m.group(3).strip()
                ret = (m.group(4) or "").strip() or None
                symbols.append(Symbol(kind="fn", name=name, line=lineno, params=params, return_type=ret))
                i += 1
                continue

            i += 1
        return symbols

    def _extract_ts_methods(self, lines, start, end):
        methods = []
        i = start
        while i < end and i < len(lines):
            m = self.METHOD_RE.match(lines[i])
            if m:
                name = m.group(2)
                params = m.group(3).strip()
                ret = (m.group(4) or "").strip() or None
                m_end = self._find_end_line_brace(lines, i)
                methods.append(Symbol(kind="method", name=name, line=i + 1, end_line=m_end, params=params, return_type=ret))
                i = m_end
                continue
            i += 1
        return methods

    def _extract_imports(self, lines):
        imports = []
        for line in lines:
            m = self.IMPORT_RE.match(line)
            if m:
                imports.append(m.group(1))
        return imports


class GoParser(BaseParser):
    FN_RE = re.compile(r'^func\s+(?:\((\w+)\s+\*?(\w+)\)\s+)?(\w+)\s*\(([^)]*)\)(?:\s*(?:\(([^)]*)\)|(\w+)))?\s*\{')
    TYPE_RE = re.compile(r'^type\s+(\w+)\s+(struct|interface)\s*\{')
    IMPORT_RE = re.compile(r'^\s*"([^"]+)"')

    def _extract_symbols(self, lines, content):
        symbols = []
        i = 0
        while i < len(lines):
            line = lines[i]
            lineno = i + 1

            m = self.TYPE_RE.match(line)
            if m:
                name = m.group(1)
                kind = m.group(2)
                end = self._find_end_line_brace(lines, i)
                symbols.append(Symbol(kind=kind, name=name, line=lineno, end_line=end))
                i = end
                continue

            m = self.FN_RE.match(line)
            if m:
                receiver_type = m.group(2)
                name = m.group(3)
                params = m.group(4).strip()
                ret = (m.group(5) or m.group(6) or "").strip() or None
                end = self._find_end_line_brace(lines, i)
                kind = "method" if receiver_type else "fn"
                sym = Symbol(kind=kind, name=name, line=lineno, end_line=end, params=params, return_type=ret)
                if receiver_type:
                    sym.parent = receiver_type
                symbols.append(sym)
                i = end
                continue

            i += 1
        return symbols

    def _extract_imports(self, lines):
        imports = []
        in_import = False
        for line in lines:
            if line.strip().startswith("import ("):
                in_import = True
                continue
            if in_import:
                if line.strip() == ")":
                    in_import = False
                    continue
                m = self.IMPORT_RE.match(line)
                if m:
                    imports.append(m.group(1))
        return imports


class JavaParser(BaseParser):
    CLASS_RE = re.compile(r'^(\s*)(?:public|private|protected)?\s*(?:static\s+)?(?:abstract\s+)?(?:final\s+)?class\s+(\w+)(?:<[^>]*>)?(?:\s+extends\s+\w+)?(?:\s+implements\s+[\w,\s]+)?\s*\{')
    INTERFACE_RE = re.compile(r'^(\s*)(?:public\s+)?interface\s+(\w+)(?:<[^>]*>)?(?:\s+extends\s+[\w,\s<>]+)?\s*\{')
    METHOD_RE = re.compile(r'^(\s*)(?:public|private|protected)?\s*(?:static\s+)?(?:final\s+)?(?:synchronized\s+)?(?:<[^>]*>\s+)?(\w+(?:<[^>]*>)?(?:\[\])?)\s+(\w+)\s*\(([^)]*)\)\s*(?:throws\s+[\w,\s]+)?\s*\{')
    IMPORT_RE = re.compile(r'^\s*import\s+([\w.]+);')

    def _extract_symbols(self, lines, content):
        symbols = []
        i = 0
        while i < len(lines):
            line = lines[i]
            lineno = i + 1

            m = self.CLASS_RE.match(line)
            if m:
                name = m.group(2)
                end = self._find_end_line_brace(lines, i)
                children = self._extract_java_methods(lines, i + 1, end - 1)
                symbols.append(Symbol(kind="class", name=name, line=lineno, end_line=end, children=children))
                i = end
                continue

            m = self.INTERFACE_RE.match(line)
            if m:
                name = m.group(2)
                end = self._find_end_line_brace(lines, i)
                symbols.append(Symbol(kind="interface", name=name, line=lineno, end_line=end))
                i = end
                continue

            i += 1
        return symbols

    def _extract_java_methods(self, lines, start, end):
        methods = []
        i = start
        while i < end and i < len(lines):
            m = self.METHOD_RE.match(lines[i])
            if m:
                ret = m.group(2)
                name = m.group(3)
                params = m.group(4).strip()
                m_end = self._find_end_line_brace(lines, i)
                methods.append(Symbol(kind="method", name=name, line=i + 1, end_line=m_end, params=params, return_type=ret))
                i = m_end
                continue
            i += 1
        return methods

    def _extract_imports(self, lines):
        return [m.group(1) for line in lines if (m := self.IMPORT_RE.match(line))]


class CParser(BaseParser):
    """Parser for C and C++ files."""
    FN_RE = re.compile(r'^(?:static\s+|extern\s+|inline\s+)*(?:const\s+)?(\w[\w\s*&]+?)\s+(\w+)\s*\(([^)]*)\)\s*\{')
    STRUCT_RE = re.compile(r'^(?:typedef\s+)?struct\s+(\w+)?\s*\{')
    ENUM_RE = re.compile(r'^(?:typedef\s+)?enum\s+(\w+)?\s*\{')
    CLASS_RE = re.compile(r'^(\s*)class\s+(\w+)(?:\s*:\s*(?:public|private|protected)\s+\w+)?\s*\{')
    INCLUDE_RE = re.compile(r'^\s*#include\s+[<"](.+?)[>"]')

    def _extract_symbols(self, lines, content):
        symbols = []
        i = 0
        while i < len(lines):
            line = lines[i]
            lineno = i + 1

            m = self.CLASS_RE.match(line)
            if m:
                name = m.group(2)
                end = self._find_end_line_brace(lines, i)
                symbols.append(Symbol(kind="class", name=name, line=lineno, end_line=end))
                i = end
                continue

            m = self.STRUCT_RE.match(line)
            if m:
                name = m.group(1) or "(anon)"
                end = self._find_end_line_brace(lines, i)
                symbols.append(Symbol(kind="struct", name=name, line=lineno, end_line=end))
                i = end
                continue

            m = self.ENUM_RE.match(line)
            if m:
                name = m.group(1) or "(anon)"
                end = self._find_end_line_brace(lines, i)
                symbols.append(Symbol(kind="enum", name=name, line=lineno, end_line=end))
                i = end
                continue

            m = self.FN_RE.match(line)
            if m:
                ret = m.group(1).strip()
                name = m.group(2)
                params = m.group(3).strip()
                end = self._find_end_line_brace(lines, i)
                symbols.append(Symbol(kind="fn", name=name, line=lineno, end_line=end, params=params, return_type=ret))
                i = end
                continue

            i += 1
        return symbols

    def _extract_imports(self, lines):
        return [m.group(1) for line in lines if (m := self.INCLUDE_RE.match(line))]


class RubyParser(BaseParser):
    CLASS_RE = re.compile(r'^(\s*)class\s+(\w+)(?:\s*<\s*(\w+))?')
    MODULE_RE = re.compile(r'^(\s*)module\s+(\w+)')
    FN_RE = re.compile(r'^(\s*)def\s+(self\.)?(\w+[?!=]?)\s*(?:\(([^)]*)\))?')

    def _extract_symbols(self, lines, content):
        symbols = []
        i = 0
        while i < len(lines):
            line = lines[i]
            lineno = i + 1

            m = self.CLASS_RE.match(line)
            if m:
                name = m.group(2)
                end = self._find_end_keyword(lines, i)
                children = self._extract_ruby_methods(lines, i + 1, end)
                symbols.append(Symbol(kind="class", name=name, line=lineno, end_line=end, children=children))
                i = end
                continue

            m = self.MODULE_RE.match(line)
            if m:
                name = m.group(2)
                end = self._find_end_keyword(lines, i)
                symbols.append(Symbol(kind="module", name=name, line=lineno, end_line=end))
                i = end
                continue

            m = self.FN_RE.match(line)
            if m:
                is_class_method = bool(m.group(2))
                name = ("self." if is_class_method else "") + m.group(3)
                params = (m.group(4) or "").strip()
                end = self._find_end_keyword(lines, i)
                symbols.append(Symbol(kind="fn", name=name, line=lineno, end_line=end, params=params))
                i = end
                continue

            i += 1
        return symbols

    def _extract_ruby_methods(self, lines, start, end):
        methods = []
        i = start
        while i < end and i < len(lines):
            m = self.FN_RE.match(lines[i])
            if m:
                name = m.group(3)
                params = (m.group(4) or "").strip()
                m_end = self._find_end_keyword(lines, i)
                methods.append(Symbol(kind="method", name=name, line=i + 1, end_line=m_end, params=params))
                i = m_end
                continue
            i += 1
        return methods

    @staticmethod
    def _find_end_keyword(lines, start):
        indent = len(lines[start]) - len(lines[start].lstrip())
        for i in range(start + 1, len(lines)):
            s = lines[i].strip()
            line_indent = len(lines[i]) - len(lines[i].lstrip())
            if s == "end" and line_indent == indent:
                return i + 1
        return len(lines)

    def _extract_imports(self, lines):
        imports = []
        for line in lines:
            s = line.strip()
            if s.startswith("require ") or s.startswith("require_relative "):
                imports.append(s)
        return imports


# ---------------------------------------------------------------------------
# Parser registry
# ---------------------------------------------------------------------------

PARSERS = {
    "rust": RustParser(),
    "python": PythonParser(),
    "coral": CoralParser(),
    "typescript": TypeScriptParser(),
    "javascript": TypeScriptParser(),
    "go": GoParser(),
    "java": JavaParser(),
    "c": CParser(),
    "cpp": CParser(),
    "ruby": RubyParser(),
}


# ---------------------------------------------------------------------------
# File discovery
# ---------------------------------------------------------------------------

DEFAULT_EXCLUDE = {
    "target", "node_modules", ".git", "__pycache__", ".idea", ".vscode",
    "dist", "build", ".next", "vendor", ".tox", "venv", ".venv", "env",
    ".mypy_cache", ".pytest_cache", ".cargo", "pkg", "bin",
}

def discover_files(root: str, include_patterns: list = None, exclude_patterns: list = None) -> list:
    """Discover source files under root, respecting include/exclude patterns."""
    from fnmatch import fnmatch

    files = []
    root = os.path.abspath(root)

    for dirpath, dirnames, filenames in os.walk(root):
        # Prune excluded directories
        rel_dir = os.path.relpath(dirpath, root)
        dirnames[:] = [
            d for d in dirnames
            if d not in DEFAULT_EXCLUDE
            and not d.startswith(".")
            and not any(fnmatch(os.path.join(rel_dir, d), ep) for ep in (exclude_patterns or []))
        ]

        for fname in sorted(filenames):
            filepath = os.path.join(dirpath, fname)
            rel_path = os.path.relpath(filepath, root)

            # Check exclude
            if exclude_patterns and any(fnmatch(rel_path, ep) for ep in exclude_patterns):
                continue

            # Check language support
            lang = detect_language(fname)
            if lang is None or lang not in PARSERS:
                continue

            # Check include (if specified)
            if include_patterns and not any(fnmatch(rel_path, ip) for ip in include_patterns):
                continue

            files.append(rel_path)

    return sorted(files)


# ---------------------------------------------------------------------------
# Markdown output
# ---------------------------------------------------------------------------

def format_symbol(sym: Symbol, indent: int = 0, file_path: str = "") -> list:
    """Format a symbol as Markdown lines."""
    lines = []
    prefix = "  " * indent
    vis = f"`{sym.visibility}` " if sym.visibility else ""

    # Main definition line
    sig_parts = [f"{prefix}- **{sym.display_kind}** {vis}`{sym.name}`"]
    if sym.params is not None and sym.params != "":
        # Truncate very long parameter lists
        params = sym.params
        if len(params) > 120:
            params = params[:117] + "..."
        sig_parts.append(f"({params})")
    if sym.return_type:
        ret = sym.return_type
        if len(ret) > 80:
            ret = ret[:77] + "..."
        sig_parts.append(f"→ `{ret}`")

    line_ref = f"L{sym.line}"
    if sym.end_line and sym.end_line > sym.line:
        line_ref = f"L{sym.line}-L{sym.end_line}"

    sig_parts.append(f"  [{line_ref}]({file_path}#{line_ref.split('-')[0]})")
    lines.append(" ".join(sig_parts))

    # Doc
    if sym.doc:
        doc = sym.doc
        if len(doc) > 200:
            doc = doc[:197] + "..."
        lines.append(f"{prefix}  > {doc}")

    # Decorators / attributes
    if sym.decorators:
        dec_str = " ".join(sym.decorators[:3])
        if len(sym.decorators) > 3:
            dec_str += f" (+{len(sym.decorators) - 3} more)"
        lines.append(f"{prefix}  Attrs: `{dec_str}`")

    # Calls
    if sym.calls:
        call_str = ", ".join(f"`{c}`" for c in sym.calls[:15])
        if len(sym.calls) > 15:
            call_str += f" (+{len(sym.calls) - 15} more)"
        lines.append(f"{prefix}  Calls: {call_str}")

    # Children
    for child in sym.children:
        lines.extend(format_symbol(child, indent + 1, file_path))

    return lines


def generate_codemap(root: str, file_maps: list) -> str:
    """Generate the complete codemap Markdown document."""
    lines = []
    root_name = os.path.basename(os.path.abspath(root))

    lines.append(f"# Code Map: {root_name}")
    lines.append("")
    lines.append(f"_Generated from `{os.path.abspath(root)}`_")
    lines.append("")

    # Summary stats
    total_lines = sum(fm.line_count for fm in file_maps)
    total_symbols = sum(len(fm.symbols) + sum(len(s.children) for s in fm.symbols) for fm in file_maps)
    lang_counts = {}
    for fm in file_maps:
        lang_counts[fm.language] = lang_counts.get(fm.language, 0) + 1

    lines.append("## Summary")
    lines.append("")
    lines.append(f"| Metric | Value |")
    lines.append(f"|--------|-------|")
    lines.append(f"| Files | {len(file_maps)} |")
    lines.append(f"| Lines | {total_lines:,} |")
    lines.append(f"| Symbols | {total_symbols:,} |")
    lines.append(f"| Languages | {', '.join(f'{lang} ({count})' for lang, count in sorted(lang_counts.items()))} |")
    lines.append("")

    # File index
    lines.append("## File Index")
    lines.append("")
    lines.append("| File | Language | Lines | Symbols | Description |")
    lines.append("|------|----------|-------|---------|-------------|")
    for fm in file_maps:
        sym_count = len(fm.symbols) + sum(len(s.children) for s in fm.symbols)
        desc = (fm.description or "")[:80]
        lines.append(f"| [{fm.path}](#{fm.path.replace('/', '').replace('.', '').replace('_', '').lower()}) | {fm.language} | {fm.line_count} | {sym_count} | {desc} |")
    lines.append("")

    # Per-file details
    lines.append("---")
    lines.append("")
    lines.append("## File Details")
    lines.append("")

    for fm in file_maps:
        anchor = fm.path.replace("/", "").replace(".", "").replace("_", "").lower()
        lines.append(f"### [{fm.path}]({fm.path})")
        lines.append("")
        lines.append(f"**{fm.language}** | {fm.line_count} lines")
        if fm.description:
            lines.append(f"> {fm.description}")
        lines.append("")

        if fm.imports:
            displayed_imports = fm.imports[:10]
            imports_str = ", ".join(f"`{imp}`" for imp in displayed_imports)
            if len(fm.imports) > 10:
                imports_str += f" (+{len(fm.imports) - 10} more)"
            lines.append(f"**Imports:** {imports_str}")
            lines.append("")

        if fm.symbols:
            for sym in fm.symbols:
                lines.extend(format_symbol(sym, 0, fm.path))
            lines.append("")

        lines.append("---")
        lines.append("")

    return "\n".join(lines)


# ---------------------------------------------------------------------------
# Main
# ---------------------------------------------------------------------------

def main():
    parser = argparse.ArgumentParser(
        description="Generate a structural code map for LLM agent navigation.",
        formatter_class=argparse.RawDescriptionHelpFormatter,
        epilog="""
Examples:
  %(prog)s src/                          # Map the src/ directory
  %(prog)s . --output codemap.md         # Map entire project, write to file
  %(prog)s . --include "*.rs" "*.coral"  # Only Rust and Coral files
  %(prog)s . --exclude "tests/*"         # Skip test files
  %(prog)s . --compact                   # Compact output (no calls/docs)
        """,
    )
    parser.add_argument("directory", nargs="?", default=".", help="Root directory to map (default: .)")
    parser.add_argument("--output", "-o", help="Output file (default: stdout)")
    parser.add_argument("--include", nargs="*", help="Glob patterns for files to include (e.g., '*.rs' '*.py')")
    parser.add_argument("--exclude", nargs="*", help="Glob patterns for files to exclude (e.g., 'tests/*')")
    parser.add_argument("--compact", action="store_true", help="Compact output — omit calls, docs, imports")
    parser.add_argument("--max-files", type=int, default=500, help="Max files to process (default: 500)")

    args = parser.parse_args()
    root = args.directory

    if not os.path.isdir(root):
        print(f"Error: '{root}' is not a directory", file=sys.stderr)
        sys.exit(1)

    # Discover files
    files = discover_files(root, args.include, args.exclude)
    if not files:
        print("No supported source files found.", file=sys.stderr)
        sys.exit(1)

    if len(files) > args.max_files:
        print(f"Warning: Found {len(files)} files, processing first {args.max_files}. Use --max-files to increase.", file=sys.stderr)
        files = files[:args.max_files]

    # Parse each file
    file_maps = []
    for rel_path in files:
        filepath = os.path.join(root, rel_path)
        lang = detect_language(rel_path)
        parser_obj = PARSERS.get(lang)
        if not parser_obj:
            continue

        try:
            with open(filepath, "r", encoding="utf-8", errors="replace") as f:
                content = f.read()
            fm = parser_obj.parse(rel_path, content)
            if args.compact:
                # Strip docs, calls, imports for compact mode
                fm.imports = []
                fm.description = None
                for sym in fm.symbols:
                    sym.doc = None
                    sym.calls = []
                    for child in sym.children:
                        child.doc = None
                        child.calls = []
            file_maps.append(fm)
        except Exception as e:
            print(f"Warning: Could not parse {rel_path}: {e}", file=sys.stderr)

    # Generate output
    output = generate_codemap(root, file_maps)

    if args.output:
        with open(args.output, "w", encoding="utf-8") as f:
            f.write(output)
        print(f"Code map written to {args.output} ({len(file_maps)} files)", file=sys.stderr)
    else:
        print(output)


if __name__ == "__main__":
    main()
